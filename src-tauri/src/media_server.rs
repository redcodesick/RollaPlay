use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use async_stream::stream;
use axum::{
    body::{Body, Bytes},
    extract::{ConnectInfo, Path, RawQuery, State},
    http::{
        header::{
            ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_MAX_AGE,
            ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, AUTHORIZATION,
            CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ORIGIN, RANGE,
            VARY, WWW_AUTHENTICATE,
        },
        HeaderMap, HeaderValue, Method, StatusCode,
    },
    response::Response,
    routing::get,
    Router,
};
use cap_std::{ambient_authority, fs::Dir};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use crate::network::{self, LanAddress};
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{Read, Seek, SeekFrom},
    net::SocketAddr,
    path::{Component, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::{io::{AsyncReadExt, AsyncSeekExt}, task};
use uuid::Uuid;

const STREAM_CHUNK_BYTES: usize = 64 * 1024;
const MIME_PROBE_BYTES: usize = 64 * 1024;
const CAST_QUERY_TOKEN_KEY: &str = "cast_token";
/// Sliding validity window for a Cast streaming grant. Renewed on every
/// authorized receiver request so a long-running playback session stays valid.
pub const CAST_GRANT_TTL: Duration = Duration::from_secs(12 * 3600);
/// Absolute ceiling from creation; after this a fresh grant must be issued.
pub const CAST_GRANT_MAX_LIFETIME: Duration = Duration::from_secs(24 * 3600);
/// Upper bound of live grants; issuing another evicts the oldest one.
pub const CAST_GRANT_MAX_ACTIVE: usize = 8;

/// Containers the Cast Default Media Receiver accepts. Codec compatibility
/// inside the container still depends on the receiver hardware (H.264/VP9 and
/// AAC/MP3 are safe on Xiaomi Mi Box).
pub fn is_cast_supported_extension(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "mp4" | "m4v" | "webm" | "mkv")
}

/// Detects an AV1 track in a WebM/MKV header. This is a compatibility hint
/// only: playback is never blocked from it. The probe stays inside the local
/// backend and no codec string or filename is shown to the receiver or UI.
pub fn detect_av1_webm(extension: &str, prefix: &[u8]) -> bool {
    let normalized = extension.trim_start_matches('.').to_ascii_lowercase();
    if normalized != "webm" && normalized != "mkv" {
        return false;
    }
    prefix.windows(5).any(|window| window == b"V_AV1")
        || prefix.windows(4).any(|window| window == b"av01")
}

/// Short-lived, media-scoped streaming credential for Cast receivers.
/// Receivers cannot send `Authorization` headers, so the sender requests a
/// scoped URL from the Tauri backend and passes it to the receiver. Grants
/// never touch disk, never appear in logs or diagnostics, and only authorize
/// the single media id they were created for.
#[derive(Debug, Clone)]
struct CastGrant {
    media_id: i64,
    created_at: Instant,
    expires_at: Instant,
    absolute_expires_at: Instant,
}

#[derive(Clone, Default)]
pub struct CastGrantStore(Arc<Mutex<HashMap<String, CastGrant>>>);

impl CastGrantStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, media_id: i64) -> (String, Duration) {
        self.create_with_lifetime(media_id, CAST_GRANT_TTL, CAST_GRANT_MAX_LIFETIME)
    }

    pub fn create_with_lifetime(&self, media_id: i64, ttl: Duration, max_lifetime: Duration) -> (String, Duration) {
        let mut raw = [0_u8; 32];
        OsRng.fill_bytes(&mut raw);
        let token = URL_SAFE_NO_PAD.encode(raw);
        let now = Instant::now();
        let expires_at = now + ttl;
        if let Ok(mut grants) = self.0.lock() {
            Self::prune_locked(&mut grants);
            while grants.len() >= CAST_GRANT_MAX_ACTIVE {
                let oldest = grants
                    .iter()
                    .min_by_key(|(_, grant)| grant.created_at)
                    .map(|(token, _)| token.clone());
                match oldest {
                    Some(token) => drop(grants.remove(&token)),
                    None => break,
                }
            }
            grants.insert(
                token.clone(),
                CastGrant {
                    media_id,
                    created_at: now,
                    expires_at,
                    absolute_expires_at: now + max_lifetime,
                },
            );
        }
        (token, ttl)
    }

    /// Constant-time check over the live grants; renews a match sliding from
    /// `now` while keeping the absolute lifetime ceiling.
    pub fn validate(&self, token: &str, media_id: i64) -> bool {
        self.validate_at(token, media_id, Instant::now())
    }

    pub fn validate_at(&self, token: &str, media_id: i64, now: Instant) -> bool {
        if token.is_empty() {
            return false;
        }
        let Ok(mut grants) = self.0.lock() else { return false };
        Self::prune_locked(&mut grants);
        let mut matched = None;
        for (stored, grant) in grants.iter() {
            if stored.len() == token.len() && bool::from(stored.as_bytes().ct_eq(token.as_bytes())) {
                matched = Some(grant.clone());
                break;
            }
        }
        match matched {
            Some(grant) if grant.media_id == media_id
                && now < grant.absolute_expires_at
                && grant.expires_at > now =>
            {
                let renewed_expiry = now + CAST_GRANT_TTL;
                let absolute_ceiling = grant.absolute_expires_at;
                if let Some(grant) = grants.get_mut(token) {
                    grant.expires_at = renewed_expiry.min(absolute_ceiling);
                }
                true
            }
            _ => false,
        }
    }

    pub fn revoke_media(&self, media_id: i64) -> usize {
        match self.0.lock() {
            Ok(mut grants) => {
                let before = grants.len();
                grants.retain(|_, grant| grant.media_id != media_id);
                before - grants.len()
            }
            Err(_) => 0,
        }
    }

    pub fn revoke_probe_token(&self, token: &str) {
        if let Ok(mut grants) = self.0.lock() {
            drop(grants.remove(token));
        }
    }

    pub fn active_count(&self) -> usize {
        self.0.lock().map(|mut grants| {
            Self::prune_locked(&mut grants);
            grants.len()
        }).unwrap_or(0)
    }

    fn prune_locked(grants: &mut HashMap<String, CastGrant>) {
        let now = Instant::now();
        grants.retain(|_, grant| grant.expires_at > now);
    }
}

/// Extracts the Cast grant token from the raw query string. Multiple or
/// repeated values are rejected rather than guessed.
pub fn parse_cast_query_token(query: Option<&str>) -> Option<String> {
    let query = query?;
    let mut values = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key == CAST_QUERY_TOKEN_KEY)
        .map(|(_, value)| value.into_owned());
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    (!first.is_empty()).then_some(first)
}

async fn bind_with_fallback(ip: std::net::Ipv4Addr, configured_port: u16) -> Result<(tokio::net::TcpListener, u16), std::io::Error> {
    let last_port = configured_port.saturating_add(PORT_FALLBACK_COUNT);
    let mut last_error = None;
    for port in configured_port..=last_port {
        match tokio::net::TcpListener::bind(SocketAddr::new(ip.into(), port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrInUse, "No fallback ports available")))
}

#[derive(Clone)]
pub struct MediaHttpState {
    database: Arc<Mutex<Connection>>,
    token: Arc<str>,
    diagnostics: StreamDiagnostics,
    cast_grants: CastGrantStore,
    allowed_subnet: Arc<std::sync::RwLock<Option<(std::net::Ipv4Addr, u8)>>>,
}

impl MediaHttpState {
    pub fn new(database: Arc<Mutex<Connection>>, token: String) -> Self {
        Self {
            database,
            token: Arc::from(token),
            diagnostics: StreamDiagnostics::default(),
            cast_grants: CastGrantStore::new(),
            allowed_subnet: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub fn cast_grants(&self) -> &CastGrantStore {
        &self.cast_grants
    }

    pub fn diagnostics(&self) -> StreamDiagnosticsSnapshot {
        self.diagnostics.snapshot()
    }

    fn set_lan_subnet(&self, subnet: Option<(std::net::Ipv4Addr, u8)>) {
        if let Ok(mut allowed) = self.allowed_subnet.write() {
            *allowed = subnet;
        }
    }

    fn allows_client(&self, address: std::net::IpAddr) -> bool {
        if address.is_loopback() {
            return true;
        }
        let std::net::IpAddr::V4(client) = address else { return false };
        if !network::is_private_lan_ipv4(client) {
            return false;
        }
        let subnet = self.allowed_subnet.read().ok().and_then(|allowed| *allowed);
        subnet.map(|(lan, prefix)| same_ipv4_subnet(client, lan, prefix)).unwrap_or(true)
    }
}

#[derive(Clone, Default)]
pub struct StreamDiagnostics(Arc<Mutex<StreamLog>>);

#[derive(Default)]
struct StreamLog {
    active: HashMap<Uuid, Arc<ActiveStream>>,
    recent: VecDeque<StreamDiagnostic>,
}

struct ActiveStream {
    id: Uuid,
    media_id: i64,
    filename: String,
    client_ip: String,
    started: Instant,
    bytes_transferred: AtomicU64,
    expected_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamDiagnostic {
    pub stream_id: String,
    pub media_id: i64,
    pub filename: String,
    pub client_ip: String,
    pub bytes_transferred: u64,
    pub duration_seconds: u64,
    pub request_status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamDiagnosticsSnapshot {
    pub active_streams: Vec<StreamDiagnostic>,
    pub recent_requests: Vec<StreamDiagnostic>,
}

impl StreamDiagnostics {
    fn begin(&self, media_id: i64, filename: String, client_ip: String, expected_bytes: u64) -> StreamGuard {
        let record = Arc::new(ActiveStream {
            id: Uuid::new_v4(),
            media_id,
            filename,
            client_ip,
            started: Instant::now(),
            bytes_transferred: AtomicU64::new(0),
            expected_bytes,
        });
        if let Ok(mut log) = self.0.lock() {
            log.active.insert(record.id, Arc::clone(&record));
        }
        StreamGuard {
            diagnostics: self.clone(),
            record,
            request_status: "complete".to_string(),
        }
    }

    fn record_request(&self, media_id: i64, filename: String, client_ip: String, status: &str) {
        self.push_recent(StreamDiagnostic {
            stream_id: Uuid::new_v4().to_string(),
            media_id,
            filename,
            client_ip,
            bytes_transferred: 0,
            duration_seconds: 0,
            request_status: status.to_string(),
        });
    }

    fn push_recent(&self, entry: StreamDiagnostic) {
        if let Ok(mut log) = self.0.lock() {
            log.recent.push_front(entry);
            log.recent.truncate(100);
        }
    }

    fn clear_recent(&self) {
        if let Ok(mut log) = self.0.lock() {
            log.recent.clear();
        }
    }

    fn snapshot(&self) -> StreamDiagnosticsSnapshot {
        let Ok(log) = self.0.lock() else {
            return StreamDiagnosticsSnapshot { active_streams: Vec::new(), recent_requests: Vec::new() };
        };
        let active_streams = log.active.values().map(|record| record.to_diagnostic("active")).collect();
        StreamDiagnosticsSnapshot {
            active_streams,
            recent_requests: log.recent.iter().cloned().collect(),
        }
    }
}

impl ActiveStream {
    fn to_diagnostic(&self, status: &str) -> StreamDiagnostic {
        StreamDiagnostic {
            stream_id: self.id.to_string(),
            media_id: self.media_id,
            filename: self.filename.clone(),
            client_ip: self.client_ip.clone(),
            bytes_transferred: self.bytes_transferred.load(Ordering::Relaxed),
            duration_seconds: self.started.elapsed().as_secs(),
            request_status: status.to_string(),
        }
    }
}

struct StreamGuard {
    diagnostics: StreamDiagnostics,
    record: Arc<ActiveStream>,
    request_status: String,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let transferred = self.record.bytes_transferred.load(Ordering::Relaxed);
        if transferred < self.record.expected_bytes {
            self.request_status = "interrupted".to_string();
        }
        let snapshot = self.record.to_diagnostic(&self.request_status);
        if let Ok(mut log) = self.diagnostics.0.lock() {
            log.active.remove(&self.record.id);
            log.recent.push_front(snapshot);
            log.recent.truncate(100);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn length(self) -> u64 {
        self.end - self.start + 1
    }
}

#[derive(Debug, PartialEq, Eq)]
enum MediaOpenError {
    NotFound,
    OutsideApprovedDirectory,
    Internal,
}

struct OpenedMedia {
    file: File,
    size: u64,
    filename: String,
    extension: String,
    mime_type: String,
    mime_source: &'static str,
    probe: Vec<u8>,
}

pub fn router(state: MediaHttpState) -> Router {
    Router::new()
        .route("/media/{media_id}", get(stream_media).head(stream_media).options(media_preflight))
        .route("/health", get(health))
        .with_state(state)
}

pub(crate) fn same_ipv4_subnet(client: std::net::Ipv4Addr, lan: std::net::Ipv4Addr, prefix_length: u8) -> bool {
    if !(1..=32).contains(&prefix_length) {
        return false;
    }
    let mask = if prefix_length == 32 { u32::MAX } else { u32::MAX << (32 - prefix_length) };
    (u32::from(client) & mask) == (u32::from(lan) & mask)
}

async fn health(
    State(state): State<MediaHttpState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response<Body> {
    if !authorized(&headers, &state.token) {
        return unauthorized_response();
    }
    if !state.allows_client(remote.ip()) {
        return response(StatusCode::FORBIDDEN, Body::empty());
    }
    response(StatusCode::NO_CONTENT, Body::empty())
}

async fn media_preflight(Path(id_text): Path<String>, headers: HeaderMap) -> Response<Body> {
    if id_text.parse::<i64>().map(|id| id > 0).unwrap_or(false) == false {
        return response(StatusCode::BAD_REQUEST, Body::empty());
    }
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return response(StatusCode::FORBIDDEN, Body::empty());
    };
    if !is_local_origin(origin) {
        return response(StatusCode::FORBIDDEN, Body::empty());
    }
    let requested_method = headers
        .get(ACCESS_CONTROL_REQUEST_METHOD)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !matches!(requested_method, "GET" | "HEAD") {
        return response(StatusCode::METHOD_NOT_ALLOWED, Body::empty());
    }
    let requested_headers = headers
        .get(ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if requested_headers.split(',').map(str::trim).any(|header| {
        !header.is_empty() && !matches!(header.to_ascii_lowercase().as_str(), "authorization" | "range")
    }) {
        return response(StatusCode::FORBIDDEN, Body::empty());
    }
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(ACCESS_CONTROL_ALLOW_HEADERS, "Authorization, Range")
        .header(ACCESS_CONTROL_MAX_AGE, "600")
        .header(VARY, "Origin")
        .header(CONTENT_LENGTH, "0")
        .body(Body::empty())
        .unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty()))
}

fn is_local_origin(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else { return false };
    if !matches!(url.scheme(), "http" | "https" | "tauri")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let normalized_host = host.trim_start_matches('[').trim_end_matches(']');
    normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host.to_ascii_lowercase().ends_with(".localhost")
        || normalized_host.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

async fn stream_media(
    State(state): State<MediaHttpState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Path(id_text): Path<String>,
    RawQuery(query): RawQuery,
    method: Method,
    headers: HeaderMap,
) -> Response<Body> {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok()).map(str::to_string);
    let cast_token = parse_cast_query_token(query.as_deref());
    let mut response = stream_media_inner(state, remote, id_text, cast_token, method, headers).await;
    if let Some(origin) = origin.filter(|origin| is_local_origin(origin)) {
        let response_headers = response.headers_mut();
        response_headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_str(&origin).unwrap());
        response_headers.insert(ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("Accept-Ranges, Content-Length, Content-Range, Content-Type, X-Media-Mime-Source"));
        response_headers.insert(VARY, HeaderValue::from_static("Origin"));
    }
    response
}

async fn stream_media_inner(
    state: MediaHttpState,
    remote: SocketAddr,
    id_text: String,
    cast_token: Option<String>,
    method: Method,
    headers: HeaderMap,
) -> Response<Body> {
    let media_id = match id_text.parse::<i64>() {
        Ok(id) if id > 0 => id,
        _ => return response(StatusCode::BAD_REQUEST, Body::empty()),
    };
    // Receivers cannot attach an Authorization header, so a request is valid
    // with either the trusted pairing bearer token or a media-scoped Cast
    // grant issued for exactly this media id. Nothing about the credential is
    // recorded in diagnostics.
    let bearer_ok = authorized(&headers, &state.token);
    let cast_ok = match cast_token {
        Some(token) => state.cast_grants.validate(&token, media_id),
        None => false,
    };
    if !bearer_ok && !cast_ok {
        state.diagnostics.record_request(media_id, "(unavailable)".to_string(), remote.ip().to_string(), "401");
        return unauthorized_response();
    }
    if !state.allows_client(remote.ip()) {
        return response(StatusCode::FORBIDDEN, Body::empty());
    }
    let database = Arc::clone(&state.database);
    let opened = match task::spawn_blocking(move || open_registered_media(&database, media_id)).await {
        Ok(Ok(media)) => media,
        Ok(Err(MediaOpenError::NotFound)) => {
            state.diagnostics.record_request(media_id, "(unavailable)".to_string(), remote.ip().to_string(), "404");
            return response(StatusCode::NOT_FOUND, Body::empty());
        }
        Ok(Err(MediaOpenError::OutsideApprovedDirectory)) => {
            state.diagnostics.record_request(media_id, "(unavailable)".to_string(), remote.ip().to_string(), "403");
            return response(StatusCode::FORBIDDEN, Body::empty());
        }
        Ok(Err(MediaOpenError::Internal)) | Err(_) => {
            state.diagnostics.record_request(media_id, "(unavailable)".to_string(), remote.ip().to_string(), "500");
            return response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty());
        }
    };

    if headers.get_all(RANGE).iter().count() > 1 {
        state.diagnostics.record_request(media_id, opened.filename.clone(), remote.ip().to_string(), "416");
        return range_not_satisfiable(opened.size);
    }
    let range_header = match headers.get(RANGE) {
        None => None,
        Some(value) => match value.to_str() {
            Ok(value) => Some(value),
            Err(_) => return range_not_satisfiable(opened.size),
        },
    };
    let byte_range = match resolve_range(range_header, opened.size) {
        Ok(range) => range,
        Err(()) => {
            state.diagnostics.record_request(media_id, opened.filename, remote.ip().to_string(), "416");
            return range_not_satisfiable(opened.size);
        }
    };
    let (status, start, length) = match byte_range {
        Some(range) => (StatusCode::PARTIAL_CONTENT, range.start, range.length()),
        None => (StatusCode::OK, 0, opened.size),
    };
    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, opened.mime_type)
        .header(CONTENT_LENGTH, length.to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "private, no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Media-Mime-Source", opened.mime_source);
    if let Some(range) = byte_range {
        builder = builder.header(CONTENT_RANGE, format!("bytes {}-{}/{}", range.start, range.end, opened.size));
    }
    if method == Method::HEAD {
        state.diagnostics.record_request(media_id, opened.filename, remote.ip().to_string(), &status.as_u16().to_string());
        return builder.body(Body::empty()).unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty()));
    }

    let mut file = tokio::fs::File::from_std(opened.file);
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty());
    }
    let guard = state.diagnostics.begin(media_id, opened.filename, remote.ip().to_string(), length);
    let stream_record = Arc::clone(&guard.record);
    let body_stream = stream! {
        let _guard = guard;
        let mut remaining = length;
        let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
        while remaining > 0 {
            let chunk_size = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
            match file.read(&mut buffer[..chunk_size]).await {
                Ok(0) => break,
                Ok(read) => {
                    remaining -= read as u64;
                    stream_record.bytes_transferred.fetch_add(read as u64, Ordering::Relaxed);
                    yield Ok(Bytes::copy_from_slice(&buffer[..read]));
                }
                Err(error) => {
                    yield Err(error);
                    break;
                }
            }
        }
    };
    builder.body(Body::from_stream(body_stream)).unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty()))
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    let Some(value) = headers.get(AUTHORIZATION).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Some((scheme, provided)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("Bearer") || provided.is_empty() || provided.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    provided.len() == token.len() && bool::from(provided.as_bytes().ct_eq(token.as_bytes()))
}

fn unauthorized_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(WWW_AUTHENTICATE, "Bearer")
        .header(CONTENT_LENGTH, "0")
        .body(Body::empty())
        .unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty()))
}

fn range_not_satisfiable(size: u64) -> Response<Body> {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_RANGE, format!("bytes */{size}"))
        .header(CONTENT_LENGTH, "0")
        .body(Body::empty())
        .unwrap_or_else(|_| response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty()))
}

fn response(status: StatusCode, body: Body) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_LENGTH, "0")
        .body(body)
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

pub fn resolve_range(header: Option<&str>, size: u64) -> Result<Option<ByteRange>, ()> {
    let Some(header) = header else { return Ok(None) };
    if size == 0 {
        return Err(());
    }
    let (unit, range) = header.trim().split_once('=').ok_or(())?;
    if !unit.trim().eq_ignore_ascii_case("bytes") || range.contains(',') {
        return Err(());
    }
    let (start_text, end_text) = range.trim().split_once('-').ok_or(())?;
    if start_text.is_empty() {
        let suffix = end_text.trim().parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let length = suffix.min(size);
        return Ok(Some(ByteRange { start: size - length, end: size - 1 }));
    }
    let start = start_text.trim().parse::<u64>().map_err(|_| ())?;
    if start >= size {
        return Err(());
    }
    let end = if end_text.trim().is_empty() {
        size - 1
    } else {
        let requested = end_text.trim().parse::<u64>().map_err(|_| ())?;
        if requested < start {
            return Err(());
        }
        requested.min(size - 1)
    };
    Ok(Some(ByteRange { start, end }))
}

fn open_registered_media(database: &Arc<Mutex<Connection>>, media_id: i64) -> Result<OpenedMedia, MediaOpenError> {
    let (stored_path, stored_root, filename, extension) = {
        let connection = database.lock().map_err(|_| MediaOpenError::Internal)?;
        connection
            .query_row(
                "SELECT f.path, d.path, f.filename, f.extension FROM media_files f
                 JOIN media_directories d ON d.id = f.directory_id
                 WHERE f.id = ?1 AND f.is_available = 1 AND d.is_active = 1",
                [media_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            )
            .optional()
            .map_err(|_| MediaOpenError::Internal)?
            .ok_or(MediaOpenError::NotFound)?
    };
    let root_path = PathBuf::from(stored_root).canonicalize().map_err(map_file_error)?;
    let source_path = PathBuf::from(stored_path).canonicalize().map_err(map_file_error)?;
    if !source_path.starts_with(&root_path) {
        return Err(MediaOpenError::OutsideApprovedDirectory);
    }
    let relative = source_path.strip_prefix(&root_path).map_err(|_| MediaOpenError::OutsideApprovedDirectory)?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
    {
        return Err(MediaOpenError::OutsideApprovedDirectory);
    }
    let directory = Dir::open_ambient_dir(&root_path, ambient_authority()).map_err(map_file_error)?;
    let capability_file = directory.open(relative).map_err(map_file_error)?;
    let metadata = capability_file.metadata().map_err(map_file_error)?;
    if !metadata.is_file() {
        return Err(MediaOpenError::NotFound);
    }
    let size = metadata.len();
    let mut file = capability_file.into_std();
    let mut probe = vec![0_u8; MIME_PROBE_BYTES];
    let read = file.read(&mut probe).map_err(|_| MediaOpenError::Internal)?;
    let (mime_type, mime_source) = detect_mime(&extension, &probe[..read]);
    probe.truncate(read);
    file.seek(SeekFrom::Start(0)).map_err(|_| MediaOpenError::Internal)?;
    Ok(OpenedMedia { file, size, filename, extension, mime_type, mime_source, probe })
}

fn map_file_error(error: std::io::Error) -> MediaOpenError {
    match error.kind() {
        std::io::ErrorKind::NotFound => MediaOpenError::NotFound,
        std::io::ErrorKind::PermissionDenied => MediaOpenError::OutsideApprovedDirectory,
        _ => MediaOpenError::Internal,
    }
}

pub fn detect_mime(extension: &str, prefix: &[u8]) -> (String, &'static str) {
    if let Some(kind) = infer::get(prefix) {
        if kind.mime_type().starts_with("video/") {
            return (kind.mime_type().to_string(), "container-signature");
        }
        if kind.mime_type() != "application/octet-stream" {
            return ("application/octet-stream".to_string(), "non-video-signature");
        }
    }
    let fallback = match extension.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "webm" => "video/webm",
        "mpeg" | "mpg" => "video/mpeg",
        "3gp" => "video/3gpp",
        _ => "application/octet-stream",
    };
    (fallback.to_string(), "extension-fallback")
}

const DEFAULT_STREAM_PORT: u16 = 8765;
const PORT_FALLBACK_COUNT: u16 = 20;

#[derive(Clone, Debug, Serialize)]
pub struct MediaServerStatus {
    pub state: String,
    pub lan_ipv4: Option<String>,
    pub interface_name: Option<String>,
    pub interface_display_name: Option<String>,
    pub listening_address: Option<String>,
    pub configured_port: u16,
    pub port: Option<u16>,
    pub media_url_template: Option<String>,
    pub network_status: String,
    pub firewall_status: String,
    pub active_streams: usize,
    pub uptime_seconds: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionTestResult {
    pub reachable: bool,
    pub status_code: Option<u16>,
    pub endpoint: Option<String>,
    pub message: String,
}

/// A receiver-ready streaming descriptor handed to the Chrome sender through
/// the loopback bridge. `url` embeds the scoped Cast grant and must never be
/// rendered, logged, or persisted by the sender page.
#[derive(Clone, Debug, Serialize)]
pub struct CastStreamUrl {
    pub url: String,
    pub media_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub mime_source: String,
    pub expires_in_seconds: u64,
    /// Fixed compatibility hint for the UI. It never carries codec strings,
    /// filenames, or paths.
    pub av1_webm: bool,
}

/// Strips the credential query from a cast URL so diagnostics can display the
/// endpoint without exposing the grant.
pub fn token_free_endpoint(url: &str) -> String {
    match url.split_once("?cast_token=") {
        Some((base, _)) => format!("{base}?cast_token=***"),
        None => url.to_string(),
    }
}

struct RunningServer {
    address: SocketAddr,
    interface: LanAddress,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    started_at: Instant,
}

#[derive(Clone)]
pub struct MediaServerManager {
    inner: Arc<MediaServerManagerInner>,
}

struct MediaServerManagerInner {
    app: AppHandle,
    database: Arc<Mutex<Connection>>,
    shared: MediaHttpState,
    runtime: tokio::sync::Mutex<Option<RunningServer>>,
    status: std::sync::RwLock<MediaServerStatus>,
    configured_port: std::sync::atomic::AtomicU16,
    desired_running: std::sync::atomic::AtomicBool,
    shutdown_started: std::sync::atomic::AtomicBool,
    shutdown_finished: std::sync::atomic::AtomicBool,
    monitor_shutdown: CancellationToken,
}

impl MediaServerManager {
    pub fn new(app: AppHandle, database: Arc<Mutex<Connection>>, token: String) -> Self {
        let configured_port = load_configured_port(&database).unwrap_or(DEFAULT_STREAM_PORT);
        let shared = MediaHttpState::new(Arc::clone(&database), token);
        let initial_status = MediaServerStatus {
            state: "stopped".to_string(),
            lan_ipv4: None,
            interface_name: None,
            interface_display_name: None,
            listening_address: None,
            configured_port,
            port: None,
            media_url_template: None,
            network_status: "not_checked".to_string(),
            firewall_status: "unknown".to_string(),
            active_streams: 0,
            uptime_seconds: None,
            error: None,
        };
        Self {
            inner: Arc::new(MediaServerManagerInner {
                app,
                database,
                shared,
                runtime: tokio::sync::Mutex::new(None),
                status: std::sync::RwLock::new(initial_status),
                configured_port: std::sync::atomic::AtomicU16::new(configured_port),
                desired_running: std::sync::atomic::AtomicBool::new(false),
                shutdown_started: std::sync::atomic::AtomicBool::new(false),
                shutdown_finished: std::sync::atomic::AtomicBool::new(false),
                monitor_shutdown: CancellationToken::new(),
            }),
        }
    }

    pub fn start_lifecycle_tasks(&self) {
        let starter = self.clone();
        tauri::async_runtime::spawn(async move {
            let _ = starter.start().await;
        });
        let monitor = self.clone();
        tauri::async_runtime::spawn(async move {
            monitor.monitor_network_changes().await;
        });
    }

    pub async fn start(&self) -> Result<MediaServerStatus, String> {
        self.inner.desired_running.store(true, Ordering::Relaxed);
        self.start_listener().await?;
        Ok(self.status_snapshot().await)
    }

    pub async fn stop(&self) -> MediaServerStatus {
        self.inner.desired_running.store(false, Ordering::Relaxed);
        self.stop_listener().await;
        let configured_port = self.inner.configured_port.load(Ordering::Relaxed);
        self.publish(MediaServerStatus {
            state: "stopped".to_string(),
            lan_ipv4: None,
            interface_name: None,
            interface_display_name: None,
            listening_address: None,
            configured_port,
            port: None,
            media_url_template: None,
            network_status: "stopped".to_string(),
            firewall_status: "unknown".to_string(),
            active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
            uptime_seconds: None,
            error: None,
        });
        self.status_snapshot().await
    }

    pub async fn restart(&self) -> Result<MediaServerStatus, String> {
        self.inner.desired_running.store(true, Ordering::Relaxed);
        self.stop_listener().await;
        self.start_listener().await?;
        Ok(self.status_snapshot().await)
    }

    pub async fn set_port(&self, port: u16) -> Result<MediaServerStatus, String> {
        if !(1024..=65535).contains(&port) {
            return Err("Choose a streaming port between 1024 and 65535".to_string());
        }
        {
            let connection = self.inner.database.lock().map_err(|_| "Local database is unavailable".to_string())?;
            connection
                .execute(
                    "INSERT INTO application_settings(key, value) VALUES ('media_server_port', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
                    [port.to_string()],
                )
                .map_err(|_| "Could not save the streaming port".to_string())?;
        }
        self.inner.configured_port.store(port, Ordering::Relaxed);
        if self.inner.desired_running.load(Ordering::Relaxed) {
            self.restart().await
        } else {
            let mut status = self.status_snapshot().await;
            status.configured_port = port;
            self.publish(status.clone());
            Ok(status)
        }
    }

    pub async fn status_snapshot(&self) -> MediaServerStatus {
        let mut status = self
            .inner
            .status
            .read()
            .map(|status| status.clone())
            .unwrap_or_else(|_| stopped_status(self.inner.configured_port.load(Ordering::Relaxed)));
        status.active_streams = self.inner.shared.diagnostics.snapshot().active_streams.len();
        let runtime = self.inner.runtime.lock().await;
        if let Some(runtime) = runtime.as_ref() {
            status.uptime_seconds = Some(runtime.started_at.elapsed().as_secs());
        } else {
            status.uptime_seconds = None;
        }
        status
    }

    pub fn stream_diagnostics(&self) -> StreamDiagnosticsSnapshot {
        self.inner.shared.diagnostics.snapshot()
    }

    /// Reads only the registered file's header to answer one fixed question:
    /// does the current WebM/MKV use AV1? Failures return `false`; no path or
    /// raw error text leaves this function.
    pub async fn probe_av1_webm(&self, media_id: i64) -> bool {
        let database = Arc::clone(&self.inner.database);
        task::spawn_blocking(move || match open_registered_media(&database, media_id) {
            Ok(opened) => detect_av1_webm(&opened.extension, &opened.probe),
            Err(_) => false,
        })
        .await
        .unwrap_or(false)
    }

    pub fn pairing_token(&self) -> String {
        self.inner.shared.token.to_string()
    }

    pub fn clear_stream_diagnostics(&self) {
        self.inner.shared.diagnostics.clear_recent();
    }

    pub fn cast_grants(&self) -> &CastGrantStore {
        &self.inner.shared.cast_grants
    }

    /// Builds a LAN streaming URL for a registered media file that a Cast
    /// receiver can fetch directly. The credential is a fresh, short-lived
    /// grant scoped to exactly this media id; the long-lived DPAPI pairing
    /// token is never included.
    pub async fn create_cast_stream_url(&self, media_id: i64) -> Result<CastStreamUrl, String> {
        let address = {
            let runtime = self.inner.runtime.lock().await;
            let Some(runtime) = runtime.as_ref() else {
                return Err("The media server is not running. Start it from Settings, or let Cast start it automatically.".to_string());
            };
            runtime.address
        };
        let database = Arc::clone(&self.inner.database);
        let opened = task::spawn_blocking(move || open_registered_media(&database, media_id))
            .await
            .map_err(|_| "Could not read the video for casting.".to_string())?;
        let opened = opened.map_err(|error| match error {
            MediaOpenError::NotFound => "This video is unavailable. Rescan its folder before casting.".to_string(),
            MediaOpenError::OutsideApprovedDirectory => "This video is outside an approved library folder.".to_string(),
            MediaOpenError::Internal => "Could not read the video for casting.".to_string(),
        })?;
        let normalized_extension = opened.extension.trim_start_matches('.').to_ascii_lowercase();
        if !is_cast_supported_extension(&normalized_extension) {
            return Err(
                "This video format cannot be played by the Cast Default Media Receiver. Use MP4, M4V, WebM, or MKV with H.264/VP9 video and AAC audio.".to_string(),
            );
        }
        let av1_webm = detect_av1_webm(&normalized_extension, &opened.probe);
        let (token, validity) = self.inner.shared.cast_grants.create(media_id);
        let url = format!("http://{}/media/{}?cast_token={}", address, media_id, token);
        Ok(CastStreamUrl {
            url,
            media_id,
            filename: opened.filename,
            mime_type: opened.mime_type,
            mime_source: opened.mime_source.to_string(),
            expires_in_seconds: validity.as_secs(),
            av1_webm,
        })
    }

    /// Performs a local HEAD probe against the exact URL a receiver would
    /// fetch (without leaking the credential into the reported endpoint) and
    /// records the result for Cast diagnostics.
    pub async fn verify_cast_stream_url(&self, media_id: i64) -> ConnectionTestResult {
        let scoped = match self.create_cast_stream_url(media_id).await {
            Ok(scoped) => scoped,
            Err(message) => {
                return ConnectionTestResult { reachable: false, status_code: None, endpoint: None, message };
            }
        };
        let probe_token = scoped
            .url
            .split("cast_token=")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let public_endpoint = token_free_endpoint(&scoped.url);
        let (address, path) = {
            let runtime = self.inner.runtime.lock().await;
            let Some(runtime) = runtime.as_ref() else {
                self.cast_grants().revoke_probe_token(&probe_token);
                return ConnectionTestResult {
                    reachable: false,
                    status_code: None,
                    endpoint: Some(public_endpoint),
                    message: "The media server stopped before the probe completed".to_string(),
                };
            };
            let path = format!("/media/{media_id}?cast_token={probe_token}");
            (runtime.address, path)
        };
        let probe = async {
            use tokio::io::AsyncWriteExt;
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            let request = format!(
                "HEAD {} HTTP/1.1\r\nHost: {}:{}\r\nRange: bytes=0-0\r\nConnection: close\r\n\r\n",
                path,
                address.ip(),
                address.port()
            );
            stream.write_all(request.as_bytes()).await?;
            let mut response = Vec::new();
            let mut buffer = [0_u8; 2048];
            while response.len() < 16384 && !response.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = tokio::io::AsyncReadExt::read(&mut stream, &mut buffer).await?;
                if read == 0 {
                    break;
                }
                response.extend_from_slice(&buffer[..read]);
            }
            let response = String::from_utf8_lossy(&response);
            let status_code = response.lines().next().and_then(|line| line.split_whitespace().nth(1)?.parse::<u16>().ok());
            let accept_ranges = response
                .to_ascii_lowercase()
                .as_bytes()
                .windows("accept-ranges:".len())
                .any(|window| window == b"accept-ranges:");
            Ok::<_, std::io::Error>((status_code, accept_ranges))
        };
        let result = match tokio::time::timeout(Duration::from_secs(4), probe).await {
            Ok(Ok((Some(200 | 206), true))) => ConnectionTestResult {
                reachable: true,
                status_code: Some(206),
                endpoint: Some(public_endpoint.clone()),
                message: "The media URL answered with a ranged response. The receiver should be able to stream it over LAN.".to_string(),
            },
            Ok(Ok((Some(code), _))) => ConnectionTestResult {
                reachable: false,
                status_code: Some(code),
                endpoint: Some(public_endpoint.clone()),
                message: format!("The media URL responded with HTTP {code} instead of a ranged video response"),
            },
            Ok(Ok((None, _))) | Ok(Err(_)) => ConnectionTestResult {
                reachable: false,
                status_code: None,
                endpoint: Some(public_endpoint.clone()),
                message: "Could not connect to the media endpoint on the selected LAN address".to_string(),
            },
            Err(_) => ConnectionTestResult {
                reachable: false,
                status_code: None,
                endpoint: Some(public_endpoint),
                message: "The media URL probe timed out".to_string(),
            },
        };
        self.cast_grants().revoke_probe_token(&probe_token);
        result
    }

    pub async fn test_connection(&self) -> ConnectionTestResult {
        let (address, endpoint) = {
            let runtime = self.inner.runtime.lock().await;
            let Some(runtime) = runtime.as_ref() else {
                return ConnectionTestResult {
                    reachable: false,
                    status_code: None,
                    endpoint: None,
                    message: "The media server is not listening".to_string(),
                };
            };
            let endpoint = format!("http://{}:{}/health", runtime.address.ip(), runtime.address.port());
            (runtime.address, endpoint)
        };
        let token = Arc::clone(&self.inner.shared.token);
        let probe = async move {
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            use tokio::io::AsyncWriteExt;
            let request = format!(
                "GET /health HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
                address.ip(), address.port(), token
            );
            stream.write_all(request.as_bytes()).await?;
            let mut response = Vec::new();
            let mut buffer = [0_u8; 1024];
            while response.len() < 8192 && !response.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = tokio::io::AsyncReadExt::read(&mut stream, &mut buffer).await?;
                if read == 0 {
                    break;
                }
                response.extend_from_slice(&buffer[..read]);
            }
            let response = String::from_utf8_lossy(&response);
            let status_code = response.lines().next().and_then(|line| line.split_whitespace().nth(1)?.parse::<u16>().ok());
            Ok::<_, std::io::Error>(status_code)
        };
        match tokio::time::timeout(Duration::from_secs(3), probe).await {
            Ok(Ok(Some(204))) => ConnectionTestResult {
                reachable: true,
                status_code: Some(204),
                endpoint: Some(endpoint),
                message: "Authenticated local connection succeeded. Remote firewall access still needs a second-device test.".to_string(),
            },
            Ok(Ok(code)) => ConnectionTestResult {
                reachable: false,
                status_code: code,
                endpoint: Some(endpoint),
                message: "The listener responded but did not return the expected health status".to_string(),
            },
            Ok(Err(_)) => ConnectionTestResult {
                reachable: false,
                status_code: None,
                endpoint: Some(endpoint),
                message: "Could not connect to the current LAN listener".to_string(),
            },
            Err(_) => ConnectionTestResult {
                reachable: false,
                status_code: None,
                endpoint: Some(endpoint),
                message: "The local connection test timed out".to_string(),
            },
        }
    }

    pub fn begin_shutdown(&self) -> bool {
        !self.inner.shutdown_started.swap(true, Ordering::AcqRel)
    }

    pub fn shutdown_is_finished(&self) -> bool {
        self.inner.shutdown_finished.load(Ordering::Acquire)
    }

    pub async fn shutdown(&self) {
        self.inner.shutdown_started.store(true, Ordering::Release);
        self.inner.desired_running.store(false, Ordering::Relaxed);
        self.inner.monitor_shutdown.cancel();
        self.stop_listener().await;
        self.inner.shutdown_finished.store(true, Ordering::Release);
    }

    async fn start_listener(&self) -> Result<(), String> {
        let mut runtime_slot = self.inner.runtime.lock().await;
        if runtime_slot.is_some() {
            return Ok(());
        }
        if !self.inner.desired_running.load(Ordering::Relaxed) {
            return Err("The media server start was cancelled".to_string());
        }
        let configured_port = self.inner.configured_port.load(Ordering::Relaxed);
        self.publish(MediaServerStatus {
            state: "starting".to_string(),
            lan_ipv4: None,
            interface_name: None,
            interface_display_name: None,
            listening_address: None,
            configured_port,
            port: None,
            media_url_template: None,
            network_status: "checking".to_string(),
            firewall_status: "unknown".to_string(),
            active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
            uptime_seconds: None,
            error: None,
        });
        let network = match task::spawn_blocking(network::detect_network).await {
            Ok(network) => network,
            Err(_) => {
                let message = "Could not inspect local network interfaces".to_string();
                self.publish(MediaServerStatus {
                    state: "error".to_string(),
                    lan_ipv4: None,
                    interface_name: None,
                    interface_display_name: None,
                    listening_address: None,
                    configured_port,
                    port: None,
                    media_url_template: None,
                    network_status: "error".to_string(),
                    firewall_status: "unknown".to_string(),
                    active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
                    uptime_seconds: None,
                    error: Some(message.clone()),
                });
                return Err(message);
            }
        };
        let Some(interface) = network.selected else {
            self.publish(MediaServerStatus {
                state: "network_unavailable".to_string(),
                lan_ipv4: None,
                interface_name: None,
                interface_display_name: None,
                listening_address: None,
                configured_port,
                port: None,
                media_url_template: None,
                network_status: "no_private_ipv4".to_string(),
                firewall_status: "unknown".to_string(),
                active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
                uptime_seconds: None,
                error: Some("Connect to a private IPv4 network to start local streaming".to_string()),
            });
            return Err("No active private LAN IPv4 address is available".to_string());
        };
        let ip = interface.ipv4.parse::<std::net::Ipv4Addr>().map_err(|_| "Detected LAN address is invalid".to_string())?;
        let last_port = configured_port.saturating_add(PORT_FALLBACK_COUNT);
        let (listener, port) = match bind_with_fallback(ip, configured_port).await {
            Ok(result) => result,
            Err(error) => {
                let state = match error.kind() {
                    std::io::ErrorKind::AddrNotAvailable => "network_unavailable",
                    std::io::ErrorKind::AddrInUse => "port_conflict",
                    _ => "error",
                };
                let message = match state {
                    "port_conflict" => format!("Ports {configured_port}–{last_port} are unavailable"),
                    "network_unavailable" => "The selected network interface disconnected before the server could bind".to_string(),
                    _ => format!("The server could not bind to the selected LAN address: {:?}", error.kind()),
                };
                self.publish(MediaServerStatus {
                    state: state.to_string(),
                    lan_ipv4: Some(interface.ipv4),
                    interface_name: Some(interface.interface_name),
                    interface_display_name: interface.display_name,
                    listening_address: None,
                    configured_port,
                    port: None,
                    media_url_template: None,
                    network_status: state.to_string(),
                    firewall_status: "unknown".to_string(),
                    active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
                    uptime_seconds: None,
                    error: Some(message.clone()),
                });
                return Err(message);
            }
        };
        let address = listener.local_addr().map_err(|_| "Could not read the media server listener address".to_string())?;
        self.inner.shared.set_lan_subnet(Some((ip, interface.prefix_length)));
        let shutdown = CancellationToken::new();
        let graceful_shutdown = shutdown.clone();
        let router = router(self.inner.shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(graceful_shutdown.cancelled_owned())
                .await;
        });
        let used_fallback = port != configured_port;
        *runtime_slot = Some(RunningServer {
            address,
            interface: interface.clone(),
            shutdown,
            task,
            started_at: Instant::now(),
        });
        drop(runtime_slot);
        self.publish(MediaServerStatus {
            state: "running".to_string(),
            lan_ipv4: Some(interface.ipv4.clone()),
            interface_name: Some(interface.interface_name),
            interface_display_name: interface.display_name,
            listening_address: Some(address.to_string()),
            configured_port,
            port: Some(port),
            media_url_template: Some(format!("http://{}:{port}/media/{{media_id}}", interface.ipv4)),
            network_status: "connected".to_string(),
            firewall_status: "unknown".to_string(),
            active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
            uptime_seconds: Some(0),
            error: used_fallback.then(|| format!("Port {configured_port} was busy; using {port} instead")),
        });
        Ok(())
    }

    async fn stop_listener(&self) {
        self.inner.shared.set_lan_subnet(None);
        let running = self.inner.runtime.lock().await.take();
        if let Some(mut running) = running {
            running.shutdown.cancel();
            if tokio::time::timeout(Duration::from_secs(3), &mut running.task).await.is_err() {
                running.task.abort();
                let _ = running.task.await;
            }
        }
    }

    async fn monitor_network_changes(self) {
        let mut interval = tokio::time::interval(Duration::from_secs(4));
        loop {
            tokio::select! {
                _ = self.inner.monitor_shutdown.cancelled() => return,
                _ = interval.tick() => {}
            }
            if !self.inner.desired_running.load(Ordering::Relaxed) {
                continue;
            }
            let network = match task::spawn_blocking(network::detect_network).await {
                Ok(network) => network,
                Err(_) => continue,
            };
            if !self.inner.desired_running.load(Ordering::Relaxed) {
                continue;
            }
            let desired_ip = network.selected.as_ref().map(|interface| interface.ipv4.clone());
            let desired_interface = network.selected.as_ref().map(|interface| interface.interface_name.clone());
            let runtime_state = self.inner.runtime.lock().await;
            let current_ip = runtime_state.as_ref().map(|runtime| runtime.interface.ipv4.clone());
            let current_interface = runtime_state.as_ref().map(|runtime| runtime.interface.interface_name.clone());
            let task_finished = runtime_state.as_ref().map(|runtime| runtime.task.is_finished()).unwrap_or(false);
            drop(runtime_state);
            if desired_ip.is_none() {
                if current_ip.is_some() {
                    self.stop_listener().await;
                }
                let configured_port = self.inner.configured_port.load(Ordering::Relaxed);
                self.publish(MediaServerStatus {
                    state: "network_unavailable".to_string(),
                    lan_ipv4: None,
                    interface_name: None,
                    interface_display_name: None,
                    listening_address: None,
                    configured_port,
                    port: None,
                    media_url_template: None,
                    network_status: "disconnected".to_string(),
                    firewall_status: "unknown".to_string(),
                    active_streams: self.inner.shared.diagnostics.snapshot().active_streams.len(),
                    uptime_seconds: None,
                    error: Some("Network disconnected. The server will retry when a private LAN address returns".to_string()),
                });
            } else if current_ip != desired_ip || current_interface != desired_interface || task_finished {
                self.stop_listener().await;
                let _ = self.start_listener().await;
            }
        }
    }

    fn publish(&self, status: MediaServerStatus) {
        if let Ok(mut current) = self.inner.status.write() {
            *current = status.clone();
        }
        let _ = self.inner.app.emit("media-server-status-changed", status);
    }
}

fn stopped_status(configured_port: u16) -> MediaServerStatus {
    MediaServerStatus {
        state: "stopped".to_string(),
        lan_ipv4: None,
        interface_name: None,
        interface_display_name: None,
        listening_address: None,
        configured_port,
        port: None,
        media_url_template: None,
        network_status: "stopped".to_string(),
        firewall_status: "unknown".to_string(),
        active_streams: 0,
        uptime_seconds: None,
        error: None,
    }
}

fn load_configured_port(database: &Arc<Mutex<Connection>>) -> Result<u16, String> {
    let connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let value = connection
        .query_row("SELECT value FROM application_settings WHERE key = 'media_server_port'", [], |row| row.get::<_, String>(0))
        .optional()
        .map_err(|_| "Could not read the streaming port".to_string())?;
    Ok(value
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| (1024..=65535).contains(port))
        .unwrap_or(DEFAULT_STREAM_PORT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE}, HeaderValue, Request};
    use http_body_util::BodyExt;
    use std::{fs, net::{Ipv4Addr, SocketAddr}};
    use tempfile::TempDir;
    use tower::ServiceExt;

    const TEST_TOKEN: &str = "phase-three-test-bearer-token";

    fn fixture(name: &str, bytes: &[u8]) -> (TempDir, MediaHttpState, i64, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("approved");
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::write(&path, bytes).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE media_directories(id INTEGER PRIMARY KEY, path TEXT NOT NULL, is_active INTEGER NOT NULL, availability_state TEXT NOT NULL);
             CREATE TABLE media_files(id INTEGER PRIMARY KEY, directory_id INTEGER NOT NULL REFERENCES media_directories(id), path TEXT NOT NULL, filename TEXT NOT NULL, extension TEXT NOT NULL, size_bytes INTEGER NOT NULL, modified_at TEXT NOT NULL, is_available INTEGER NOT NULL, availability_state TEXT NOT NULL);
             CREATE TABLE application_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        ).unwrap();
        connection.execute("INSERT INTO media_directories(id, path, is_active, availability_state) VALUES (1, ?1, 1, 'available')", [root.to_string_lossy().as_ref()]).unwrap();
        connection.execute(
            "INSERT INTO media_files(id, directory_id, path, filename, extension, size_bytes, modified_at, is_available, availability_state)
             VALUES (1, 1, ?1, ?2, ?3, ?4, '1', 1, 'available')",
            rusqlite::params![path.to_string_lossy().as_ref(), name, PathBuf::from(name).extension().unwrap().to_string_lossy().to_ascii_lowercase(), bytes.len() as i64],
        ).unwrap();
        let state = MediaHttpState::new(Arc::new(Mutex::new(connection)), TEST_TOKEN.to_string());
        (temp, state, 1, path)
    }

    fn make_request(method: Method, uri: &str, token: Option<&str>) -> Request<Body> {
        let mut request = Request::new(Body::empty());
        *request.method_mut() = method;
        *request.uri_mut() = uri.parse().unwrap();
        if let Some(token) = token {
            request.headers_mut().insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
        }
        request.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50999))));
        request
    }

    async fn call(app: Router, method: Method, uri: &str, token: Option<&str>, range: Option<&str>) -> Response<Body> {
        let mut request = make_request(method, uri, token);
        if let Some(range) = range {
            request.headers_mut().insert(RANGE, HeaderValue::from_str(range).unwrap());
        }
        app.oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn full_stream_and_head_have_expected_headers() {
        let (_temp, state, id, _) = fixture("movie.mp4", b"0123456789");
        let app = router(state);
        let response = call(app.clone(), Method::GET, &format!("/media/{id}"), Some(TEST_TOKEN), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_LENGTH], "10");
        assert_eq!(response.headers()[ACCEPT_RANGES], "bytes");
        assert_eq!(response.headers()[CONTENT_TYPE], "video/mp4");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"0123456789");

        let response = call(app.clone(), Method::HEAD, &format!("/media/{id}"), Some(TEST_TOKEN), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_LENGTH], "10");
        assert!(response.into_body().collect().await.unwrap().to_bytes().is_empty());
        assert_eq!(call(app.clone(), Method::GET, "/health", Some(TEST_TOKEN), None).await.status(), StatusCode::NO_CONTENT);
        assert_eq!(call(app, Method::GET, "/health", None, None).await.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn cast_grant_query_authorizes_a_receiver_without_a_bearer_token() {
        let (_temp, state, id, _) = fixture("movie.mp4", b"0123456789");
        let (token, validity) = state.cast_grants().create_with_lifetime(id, Duration::from_secs(600), Duration::from_secs(600));
        assert_eq!(validity, Duration::from_secs(600));
        let app = router(state);
        let response = call(app.clone(), Method::GET, &format!("/media/{id}?cast_token={token}"), None, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "video/mp4");
        let ranged = call(app.clone(), Method::GET, &format!("/media/{id}?cast_token={token}"), None, Some("bytes=2-5")).await;
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(ranged.headers()[CONTENT_RANGE], "bytes 2-5/10");
        // A grant is scoped to exactly one media id.
        let other = call(app.clone(), Method::GET, &format!("/media/999?cast_token={token}"), None, None).await;
        assert_eq!(other.status(), StatusCode::UNAUTHORIZED);
        // Missing or malformed credentials stay rejected.
        let missing = call(app.clone(), Method::GET, &format!("/media/{id}"), None, None).await;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let duplicated = call(app, Method::GET, &format!("/media/{id}?cast_token={token}&cast_token={token}"), None, None).await;
        assert_eq!(duplicated.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn closed_open_and_suffix_ranges_return_206() {
        let (_temp, state, id, _) = fixture("movie.mp4", b"0123456789");
        let app = router(state);
        for (range, expected, content_range) in [
            ("bytes=2-5", b"2345".as_slice(), "bytes 2-5/10"),
            ("bytes=3-3", b"3".as_slice(), "bytes 3-3/10"),
            ("bytes=7-", b"789".as_slice(), "bytes 7-9/10"),
            ("bytes=-3", b"789".as_slice(), "bytes 7-9/10"),
        ] {
            let response = call(app.clone(), Method::GET, &format!("/media/{id}"), Some(TEST_TOKEN), Some(range)).await;
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert_eq!(response.headers()[CONTENT_RANGE], content_range);
            assert_eq!(&response.into_body().collect().await.unwrap().to_bytes()[..], expected);
        }
    }

    #[tokio::test]
    async fn invalid_and_multiple_ranges_return_416() {
        let (_temp, state, id, _) = fixture("movie.mp4", b"0123456789");
        let app = router(state);
        for range in ["bytes=10-", "bytes=8-2", "items=0-1", "bytes=-0", "bytes=0-1,3-4"] {
            let response = call(app.clone(), Method::GET, &format!("/media/{id}"), Some(TEST_TOKEN), Some(range)).await;
            assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE, "{range}");
            assert_eq!(response.headers()[CONTENT_RANGE], "bytes */10");
        }
    }

    #[tokio::test]
    async fn unauthorized_unknown_missing_and_outside_files_are_rejected() {
        let (_temp, state, id, path) = fixture("movie.mp4", b"video");
        let app = router(state.clone());
        assert_eq!(call(app.clone(), Method::GET, &format!("/media/{id}"), None, None).await.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(call(app.clone(), Method::GET, "/media/not-a-number", Some(TEST_TOKEN), None).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(call(app.clone(), Method::GET, "/media/0", Some(TEST_TOKEN), None).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(call(app.clone(), Method::GET, "/media/999999", Some(TEST_TOKEN), None).await.status(), StatusCode::NOT_FOUND);

        let outside = path.parent().unwrap().parent().unwrap().join("outside.mp4");
        fs::write(&outside, b"outside").unwrap();
        state.database.lock().unwrap().execute("UPDATE media_files SET path = ?1 WHERE id = ?2", rusqlite::params![outside.to_string_lossy().as_ref(), id]).unwrap();
        assert_eq!(call(app.clone(), Method::GET, &format!("/media/{id}"), Some(TEST_TOKEN), None).await.status(), StatusCode::FORBIDDEN);

        state.database.lock().unwrap().execute("UPDATE media_files SET path = ?1 WHERE id = ?2", rusqlite::params![path.to_string_lossy().as_ref(), id]).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(call(app, Method::GET, &format!("/media/{id}"), Some(TEST_TOKEN), None).await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn preflight_is_limited_to_loopback_origins_and_admin_routes_are_absent() {
        let (_temp, state, id, _) = fixture("movie.mp4", b"video");
        let app = router(state);
        let mut request = make_request(Method::OPTIONS, &format!("/media/{id}"), None);
        request.headers_mut().insert(ORIGIN, HeaderValue::from_static("http://localhost:5173"));
        request.headers_mut().insert(ACCESS_CONTROL_REQUEST_METHOD, HeaderValue::from_static("GET"));
        request.headers_mut().insert(ACCESS_CONTROL_REQUEST_HEADERS, HeaderValue::from_static("authorization,range"));
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:5173");

        let mut request = make_request(Method::OPTIONS, &format!("/media/{id}"), None);
        request.headers_mut().insert(ORIGIN, HeaderValue::from_static("https://attacker.example"));
        request.headers_mut().insert(ACCESS_CONTROL_REQUEST_METHOD, HeaderValue::from_static("GET"));
        assert_eq!(app.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(call(app, Method::GET, "/api/status", Some(TEST_TOKEN), None).await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn large_stream_and_concurrent_requests_complete_without_shared_offsets() {
        let bytes = vec![0x5a; 2 * 1024 * 1024];
        let (_temp, state, id, _) = fixture("large.webm", &bytes);
        let app = router(state.clone());
        let uri = format!("/media/{id}");
        let (left, right) = tokio::join!(
            call(app.clone(), Method::GET, &uri, Some(TEST_TOKEN), None),
            call(app.clone(), Method::GET, &uri, Some(TEST_TOKEN), None),
        );
        assert_eq!(left.status(), StatusCode::OK);
        assert_eq!(right.status(), StatusCode::OK);
        let (left, right) = tokio::join!(left.into_body().collect(), right.into_body().collect());
        assert_eq!(left.unwrap().to_bytes().len(), bytes.len());
        assert_eq!(right.unwrap().to_bytes().len(), bytes.len());
        assert_eq!(state.diagnostics.snapshot().recent_requests.len(), 2);
    }

    #[tokio::test]
    async fn dropping_response_body_records_an_interrupted_stream() {
        let bytes = vec![0x33; 2 * 1024 * 1024];
        let (_temp, state, id, _) = fixture("large.mp4", &bytes);
        let response = call(router(state.clone()), Method::GET, &format!("/media/{id}"), Some(TEST_TOKEN), None).await;
        let mut body = response.into_body();
        let frame = body.frame().await.unwrap().unwrap();
        assert!(!frame.into_data().unwrap().is_empty());
        assert_eq!(state.diagnostics.snapshot().active_streams.len(), 1);
        drop(body);
        let snapshot = state.diagnostics.snapshot();
        assert!(snapshot.active_streams.is_empty());
        assert_eq!(snapshot.recent_requests[0].request_status, "interrupted");
    }

    #[test]
    fn lan_access_policy_uses_prefix_boundaries() {
        assert!(same_ipv4_subnet(Ipv4Addr::new(192, 168, 4, 35), Ipv4Addr::new(192, 168, 4, 1), 24));
        assert!(!same_ipv4_subnet(Ipv4Addr::new(192, 168, 5, 35), Ipv4Addr::new(192, 168, 4, 1), 24));
        assert!(!same_ipv4_subnet(Ipv4Addr::new(192, 168, 4, 35), Ipv4Addr::new(192, 168, 4, 1), 0));
    }

    #[test]
    fn lan_clients_must_match_the_selected_private_subnet() {
        let (_temp, state, _, _) = fixture("movie.mp4", b"video");
        state.set_lan_subnet(Some((Ipv4Addr::new(192, 168, 1, 12), 24)));
        assert!(state.allows_client("192.168.1.55".parse().unwrap()));
        assert!(!state.allows_client("192.168.2.55".parse().unwrap()));
        assert!(!state.allows_client("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn mime_detection_prefers_container_signature_then_uses_safe_extension_fallback() {
        let mp4_header = [0, 0, 0, 24, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's', b'o', b'm', b'i', b's', b'o', b'2', b'a', b'v', b'c', b'1', b'm', b'p', b'4', b'1'];
        let (mime, source) = detect_mime("mkv", &mp4_header);
        assert_eq!(mime, "video/mp4");
        assert_eq!(source, "container-signature");
        let (mime, source) = detect_mime("mkv", b"unknown");
        assert_eq!(mime, "video/x-matroska");
        assert_eq!(source, "extension-fallback");
        assert_eq!(detect_mime("unknown", b"unknown").0, "application/octet-stream");
        let (mime, source) = detect_mime("mp4", &[0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(mime, "application/octet-stream");
        assert_eq!(source, "non-video-signature");
    }

    #[test]
    fn av1_hint_is_limited_to_webm_and_mkv_headers() {
        assert!(detect_av1_webm("webm", b"\x1aE\xdf\xa3V_AV1A_OPUS"));
        assert!(detect_av1_webm("MKV", b"header av01 track"));
        assert!(!detect_av1_webm("webm", b"V_VP9A_OPUS"));
        assert!(!detect_av1_webm("mp4", b"V_AV1av01"));
        assert!(!detect_av1_webm("webm", b""));
    }

    #[test]
    fn extension_mime_fallbacks_cover_supported_video_containers() {
        for (extension, expected) in [
            ("mp4", "video/mp4"),
            ("mkv", "video/x-matroska"),
            ("avi", "video/x-msvideo"),
            ("mov", "video/quicktime"),
            ("wmv", "video/x-ms-wmv"),
            ("webm", "video/webm"),
            ("mpeg", "video/mpeg"),
            ("mpg", "video/mpeg"),
            ("m4v", "video/mp4"),
            ("3gp", "video/3gpp"),
        ] {
            assert_eq!(detect_mime(extension, b"unknown").0, expected, "{extension}");
        }
    }

    #[tokio::test]
    async fn occupied_port_uses_a_fallback_port() {
        let blocker = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let blocked_port = blocker.local_addr().unwrap().port();
        let (_listener, selected_port) = bind_with_fallback(Ipv4Addr::LOCALHOST, blocked_port).await.unwrap();
        assert_ne!(selected_port, blocked_port);
    }

    #[test]
    fn cast_grants_authorize_only_their_scoped_media_id() {
        let grants = CastGrantStore::new();
        let (token, validity) = grants.create_with_lifetime(7, Duration::from_secs(600), Duration::from_secs(1200));
        assert_eq!(validity, Duration::from_secs(600));
        assert!(grants.validate(&token, 7));
        assert!(!grants.validate(&token, 8));
        assert!(!grants.validate("different-token-of-43-chars-length-ok!!!", 7));
        assert!(!grants.validate("", 7));
        assert_eq!(grants.active_count(), 1);
        assert_eq!(grants.revoke_media(7), 1);
        assert!(!grants.validate(&token, 7));
        assert_eq!(grants.active_count(), 0);
    }

    #[test]
    fn cast_grants_expire_and_renew_with_an_absolute_ceiling() {
        let grants = CastGrantStore::new();
        let (token, _) = grants.create_with_lifetime(3, Duration::from_secs(100), Duration::from_secs(250));
        let t0 = Instant::now();
        assert!(grants.validate_at(&token, 3, t0 + Duration::from_secs(99)));
        // Sliding window renews from the last use.
        assert!(grants.validate_at(&token, 3, t0 + Duration::from_secs(199)));
        // …but never past the absolute lifetime ceiling.
        assert!(!grants.validate_at(&token, 3, t0 + Duration::from_secs(251)));
        let (expired, _) = grants.create_with_lifetime(4, Duration::from_secs(10), Duration::from_secs(20));
        assert!(!grants.validate_at(&expired, 4, t0 + Duration::from_secs(11)));
    }

    #[test]
    fn cast_grant_store_evicts_the_oldest_grant_at_capacity() {
        let grants = CastGrantStore::new();
        let mut first_token = String::new();
        for media_id in 0..CAST_GRANT_MAX_ACTIVE as i64 {
            let (token, _) = grants.create_with_lifetime(media_id, Duration::from_secs(600), Duration::from_secs(600));
            if media_id == 0 {
                first_token = token;
            }
        }
        assert_eq!(grants.active_count(), CAST_GRANT_MAX_ACTIVE);
        let (overflow, _) = grants.create_with_lifetime(99, Duration::from_secs(600), Duration::from_secs(600));
        assert_eq!(grants.active_count(), CAST_GRANT_MAX_ACTIVE);
        assert!(!grants.validate(&first_token, 0));
        assert!(grants.validate(&overflow, 99));
    }

    #[test]
    fn cast_query_token_parsing_accepts_single_value_and_rejects_duplicates() {
        assert_eq!(parse_cast_query_token(Some("cast_token=abc")).as_deref(), Some("abc"));
        assert_eq!(parse_cast_query_token(Some("a=1&cast_token=with%2Fslash")).as_deref(), Some("with/slash"));
        assert_eq!(parse_cast_query_token(Some("cast_token=")), None);
        assert_eq!(parse_cast_query_token(Some("other=1")), None);
        assert_eq!(parse_cast_query_token(Some("cast_token=a&cast_token=b")), None);
        assert_eq!(parse_cast_query_token(None), None);
    }

    #[test]
    fn token_free_endpoint_hides_the_credential() {
        let url = "http://192.168.1.9:8765/media/12?cast_token=secretvalue";
        assert_eq!(token_free_endpoint(url), "http://192.168.1.9:8765/media/12?cast_token=***");
        assert_eq!(token_free_endpoint("http://192.168.1.9:8765/media/12"), "http://192.168.1.9:8765/media/12");
    }

    #[test]
    fn cast_extension_gate_matches_default_media_receiver_containers() {
        assert!(is_cast_supported_extension("mp4"));
        assert!(is_cast_supported_extension("M4V"));
        assert!(is_cast_supported_extension("webm"));
        assert!(is_cast_supported_extension("mkv"));
        assert!(!is_cast_supported_extension("avi"));
        assert!(!is_cast_supported_extension("wmv"));
        assert!(!is_cast_supported_extension("3gp"));
        assert!(!is_cast_supported_extension("mpeg"));
    }
}
