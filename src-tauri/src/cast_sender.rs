//! Cast sender integration.
//!
//! The official Google Cast Web Sender SDK needs a supported Chrome
//! environment; the Tauri WebView2 host does not expose the same Cast
//! framework. This module therefore:
//!
//! 1. detects a local Chrome installation (registry first, then well-known
//!    install paths, then `PATH` on other platforms),
//! 2. runs a tiny loopback-only HTTP bridge that serves a dedicated sender
//!    page (`cast-sender/`) and streams commands to it over Server-Sent
//!    Events,
//! 3. launches Chrome in application mode against the loopback origin — a
//!    secure context the Cast SDK accepts — and watches the child process so
//!    a crashed or closed sender window is reported (and automatically
//!    relaunched while a Cast session was active),
//! 4. forwards playback events from the sender page back to the Tauri UI and
//!    persists playback state into SQLite.
//!
//! The bridge exposes no administrative routes and carries no secrets: the
//! sender page only ever receives the scoped, short-lived media URL at the
//! moment the user casts something.

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header::CONTENT_SECURITY_POLICY, header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::media_server::ConnectionTestResult;
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SENDER_HTML: &str = include_str!("../cast-sender/index.html");
const SENDER_JS: &str = include_str!("../cast-sender/sender.js");
const SENDER_BRIDGE_PREFERRED_PORT: u16 = 8790;
const SENDER_BRIDGE_LAST_PORT: u16 = 8819;
const PAGE_CONNECT_GRACE: Duration = Duration::from_secs(12);
const PAGE_DISCONNECT_GRACE: Duration = Duration::from_secs(20);
const MAX_RELAUNCH_ATTEMPTS: u32 = 3;
const DB_PROGRESS_WRITE_INTERVAL: Duration = Duration::from_secs(30);

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Chrome detection and launch
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct ChromeDetection {
    pub installed: bool,
    pub executable_path: Option<String>,
    pub discovery_source: Option<String>,
    pub candidates_checked: Vec<String>,
}

pub fn detect_chrome() -> ChromeDetection {
    let candidates = chrome_candidates();
    let candidates_checked = candidates.iter().map(|(path, _)| path.display().to_string()).collect();
    match resolve_existing_executable(&candidates) {
        Some((path, source)) => ChromeDetection {
            installed: true,
            executable_path: Some(path.display().to_string()),
            discovery_source: Some(source.to_string()),
            candidates_checked,
        },
        None => ChromeDetection {
            installed: false,
            executable_path: None,
            discovery_source: None,
            candidates_checked,
        },
    }
}

type ChromeCandidate = (PathBuf, &'static str);

fn resolve_existing_executable(candidates: &[ChromeCandidate]) -> Option<(PathBuf, &'static str)> {
    candidates
        .iter()
        .find(|(path, _)| path.is_file())
        .map(|(path, source)| (path.clone(), *source))
}

/// Returns Chrome executable candidates in priority order (registry, default
/// install locations, then `PATH`), each tagged with its discovery source.
#[cfg(windows)]
fn chrome_candidates() -> Vec<ChromeCandidate> {
    let mut candidates = Vec::new();
    for (source, subkey) in [
        ("registry-hklm", r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe"),
        ("registry-hkcu", r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe"),
    ] {
        if let Some(path) = windows_registry_chrome_path(subkey) {
            candidates.push((path, source));
        }
    }
    let add_from_env = |candidates: &mut Vec<ChromeCandidate>, variable: &str, source: &'static str| {
        if let Ok(value) = std::env::var(variable) {
            let path = PathBuf::from(value).join(r"Google\Chrome\Application\chrome.exe");
            candidates.push((path, source));
        }
    };
    add_from_env(&mut candidates, "ProgramFiles", "program-files");
    add_from_env(&mut candidates, "ProgramFiles(x86)", "program-files-x86");
    add_from_env(&mut candidates, "LocalAppData", "local-app-data");
    candidates
}

#[cfg(windows)]
fn windows_registry_chrome_path(subkey: &str) -> Option<PathBuf> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    if let Ok(key) = LOCAL_MACHINE.open(subkey) {
        if let Ok(value) = key.get_string("") {
            return Some(PathBuf::from(value));
        }
    }
    if let Ok(key) = CURRENT_USER.open(subkey) {
        if let Ok(value) = key.get_string("") {
            return Some(PathBuf::from(value));
        }
    }
    None
}

#[cfg(not(windows))]
fn chrome_candidates() -> Vec<ChromeCandidate> {
    let mut candidates: Vec<ChromeCandidate> = Vec::new();
    let names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"];
    if let Ok(path_variable) = std::env::var("PATH") {
        for directory in std::env::split_paths(&path_variable) {
            for name in names {
                candidates.push((directory.join(name), "path"));
            }
        }
    }
    for (path, source) in [
        ("/usr/bin/google-chrome", "default-path"),
        ("/usr/bin/google-chrome-stable", "default-path"),
        ("/usr/bin/chromium", "default-path"),
        ("/usr/bin/chromium-browser", "default-path"),
        ("/opt/google/chrome/chrome", "default-path"),
        ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "default-path"),
    ] {
        candidates.push((PathBuf::from(path), source));
    }
    candidates
}

/// Builds the Chrome application-mode command. `--app` opens a dedicated
/// window without browser chrome on a secure loopback origin, which is the
/// supported environment for the official Cast Web Sender SDK.
pub fn chrome_launch_command(executable: &Path, sender_url: &str) -> Command {
    let mut command = Command::new(executable);
    command.arg(format!("--app={sender_url}"));
    command.args(["--new-window", "--no-first-run", "--no-default-browser-check"]);
    command
}

// ---------------------------------------------------------------------------
// Shared snapshot and report types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize)]
pub struct CastPlaybackSnapshot {
    pub media_file_id: Option<i64>,
    pub filename: Option<String>,
    pub player_state: Option<String>,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub volume_level: Option<f64>,
    pub muted: Option<bool>,
    pub device_name: Option<String>,
    pub cast_state: Option<String>,
    pub session_state: Option<String>,
    pub last_error: Option<String>,
    pub updated_at_epoch_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CastSenderStatus {
    pub chrome_installed: bool,
    pub chrome_path: Option<String>,
    pub chrome_source: Option<String>,
    pub bridge_state: String,
    pub bridge_port: Option<u16>,
    pub sender_page_connected: bool,
    pub sender_page_last_seen_ms_ago: Option<u64>,
    pub sender_window_state: String,
    pub sender_pid: Option<u32>,
    pub relaunch_attempts: u32,
    pub last_error: Option<String>,
    pub last_launch_attempt_epoch_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CastDiagnosticsSnapshot {
    pub sender: CastSenderStatus,
    pub discovery: CastPlaybackSnapshot,
    pub media_url_probe: Option<ConnectionTestResult>,
    pub active_cast_grants: usize,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct CastEventReport {
    pub kind: String,
    pub device_name: Option<String>,
    pub cast_state: Option<String>,
    pub session_state: Option<String>,
    pub player_state: Option<String>,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub message: Option<String>,
    /// Correlation token generated by the Phase 5 queue engine for the active
    /// load. Actual Cast completion events echo it back; stale events are
    /// ignored by the engine.
    pub playback_token: Option<String>,
    /// Google Cast receiver session id, when reported by the SDK.
    pub cast_session_id: Option<String>,
    /// Receiver media session id, when reported by the SDK.
    pub media_session_id: Option<i64>,
    /// `chrome.cast.media.IdleReason` that produced a completion event.
    pub idle_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Bridge server
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CastSnapshotInternal {
    public: CastPlaybackSnapshot,
    pending_load: Option<(i64, String)>,
    last_progress_db_write: Option<Instant>,
    /// Session id of the sender that currently owns the Cast session. Used to
    /// reject stale session-ended/failed events from a second sender window.
    active_cast_session_id: Option<String>,
}

#[derive(Clone)]
struct CastBridgeState {
    app: AppHandle,
    database: Arc<Mutex<Connection>>,
    snapshot: Arc<Mutex<CastSnapshotInternal>>,
    broadcast: broadcast::Sender<String>,
    sender_connections: Arc<AtomicUsize>,
    sender_last_seen: Arc<Mutex<Option<Instant>>>,
    last_error: Arc<Mutex<Option<String>>>,
    media_url_probe: Arc<Mutex<Option<ConnectionTestResult>>>,
}

struct SenderConnectionGuard {
    connections: Arc<AtomicUsize>,
    last_seen: Arc<Mutex<Option<Instant>>>,
}

impl Drop for SenderConnectionGuard {
    fn drop(&mut self) {
        self.connections.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn sender_bridge_router(state: CastBridgeState) -> Router {
    Router::new()
        .route("/", get(sender_page))
        .route("/sender.js", get(sender_script))
        .route("/events", get(sender_events))
        .route("/playback-event", post(playback_event))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state)
}

/// The Cast loader injects protocol-relative `//www.gstatic.com/...` scripts.
/// On this HTTP loopback origin those become `http://www.gstatic.com/...`,
/// which a strict `https:`-only `script-src` blocks. `upgrade-insecure-requests`
/// promotes them to HTTPS before the CSP check, so the SDK loads without
/// weakening the allow-list to permit plain HTTP scripts.
fn secure_page_response(body: &'static str, content_type: &'static str) -> Response {
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'self'; upgrade-insecure-requests; \
                     script-src 'self' https://www.gstatic.com; \
                     style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; \
                     media-src 'self'; frame-ancestors 'none'; base-uri 'none'",
                ),
            ),
        ],
        body,
    )
        .into_response()
}

async fn sender_page() -> Response {
    secure_page_response(SENDER_HTML, "text/html; charset=utf-8")
}

async fn sender_script() -> Response {
    secure_page_response(SENDER_JS, "application/javascript; charset=utf-8")
}

async fn sender_events(
    State(state): State<CastBridgeState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>> + Send + 'static> {
    state.sender_connections.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut seen) = state.sender_last_seen.lock() {
        *seen = Some(Instant::now());
    }
    let guard = SenderConnectionGuard {
        connections: Arc::clone(&state.sender_connections),
        last_seen: Arc::clone(&state.sender_last_seen),
    };
    let playback = {
        let snapshot = state.snapshot.lock().map(|snapshot| snapshot.public.clone());
        snapshot.unwrap_or_default()
    };
    let hello = serde_json::json!({
        "type": "hello",
        "playback": playback,
        "server_time_epoch_ms": now_epoch_ms(),
    });
    let stream = async_stream::stream! {
        let _guard = guard;
        yield Ok::<_, std::convert::Infallible>(Event::default().data(hello.to_string()));
        let mut receiver = BroadcastStream::new(state.broadcast.subscribe());
        loop {
            match receiver.next().await {
                Some(Ok(message)) => yield Ok(Event::default().data(message)),
                Some(Err(_)) => continue,
                None => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10)).text("keep-alive"))
}

async fn playback_event(State(state): State<CastBridgeState>, Json(event): Json<Value>) -> Response {
    if let Ok(mut seen) = state.sender_last_seen.lock() {
        *seen = Some(Instant::now());
    }
    handle_playback_event(&state, event).await;
    StatusCode::NO_CONTENT.into_response()
}

fn clamp_string(value: Option<&Value>, max: usize) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.chars().take(max).collect())
}

fn clamp_number(value: Option<&Value>) -> Option<f64> {
    let value = value?.as_f64()?;
    value.is_finite().then_some(value)
}

/// A session event may only end the session that is currently active. Events
/// without a session id are accepted only when no session is being tracked.
fn session_event_matches(active: Option<&String>, reported: Option<&String>) -> bool {
    match (active, reported) {
        (Some(active), Some(reported)) => active == reported,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

async fn handle_playback_event(state: &CastBridgeState, event: Value) {
    let kind = clamp_string(event.get("type"), 40).unwrap_or_else(|| "unknown".to_string());
    let device_name = clamp_string(event.get("device_name"), 120);
    let cast_state = clamp_string(event.get("state").or_else(|| event.get("cast_state")), 60);
    let session_state = clamp_string(event.get("session_state"), 60);
    let player_state = clamp_string(event.get("player_state"), 40);
    let position = clamp_number(event.get("position_seconds"));
    let duration = clamp_number(event.get("duration_seconds"));
    let volume = clamp_number(event.get("volume_level"));
    let muted = event.get("muted").and_then(Value::as_bool);
    let message = clamp_string(event.get("message"), 400);
    let error_code = clamp_string(event.get("error_code"), 80);
    let playback_token = clamp_string(event.get("playback_token"), 80);
    let cast_session_id = clamp_string(event.get("cast_session_id"), 120);
    let media_session_id = event.get("media_session_id").and_then(Value::as_i64);
    let idle_reason = clamp_string(event.get("idle_reason"), 40);

    let mut report = CastEventReport {
        kind: kind.clone(),
        device_name: device_name.clone(),
        cast_state: cast_state.clone(),
        session_state: session_state.clone(),
        player_state: player_state.clone(),
        position_seconds: position,
        duration_seconds: duration,
        message: message.clone().or(error_code.clone()),
        playback_token,
        cast_session_id: cast_session_id.clone(),
        media_session_id,
        idle_reason,
    };

    let mut database_write: Option<(&'static str, f64)> = None;
    {
        let mut snapshot = state.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.public.updated_at_epoch_ms = now_epoch_ms();
        // Only the sender that owns the active session may end it. A second
        // sender window sees no devices and posts session_start_failed; without
        // this guard it would erase the device name of the sender that is
        // actually casting.
        let session_matches = session_event_matches(
            snapshot.active_cast_session_id.as_ref(),
            cast_session_id.as_ref(),
        );
        match kind.as_str() {
            "sender_ready" => {}
            "cast_state" => {
                snapshot.public.cast_state = cast_state.clone();
                // A reconnecting sender can report cast state before a session
                // event; keep the receiver visible when the SDK supplies it.
                if device_name.is_some() {
                    snapshot.public.device_name = device_name.clone();
                }
            }
            "session_started" | "session_resumed" => {
                if cast_session_id.is_some() {
                    snapshot.active_cast_session_id = cast_session_id.clone();
                }
                snapshot.public.device_name = device_name.clone();
                snapshot.public.session_state = Some(
                    session_state
                        .clone()
                        .unwrap_or_else(|| "SESSION_STARTED".to_string()),
                );
                snapshot.public.last_error = None;
            }
            "session_start_failed" => {
                if session_matches {
                    snapshot.public.device_name = None;
                    snapshot.public.session_state = session_state.clone().or_else(|| Some("SESSION_START_FAILED".to_string()));
                    snapshot.public.player_state = Some("IDLE".to_string());
                    let failure = message
                        .clone()
                        .or(error_code.clone())
                        .unwrap_or_else(|| "The Cast session did not start.".to_string());
                    snapshot.public.last_error = Some(failure.clone());
                    if let Ok(mut last) = state.last_error.lock() {
                        *last = Some(failure);
                    }
                }
            }
            "session_ended" => {
                if session_matches {
                    snapshot.public.device_name = None;
                    snapshot.active_cast_session_id = None;
                    snapshot.public.session_state = session_state.clone().or_else(|| Some("SESSION_ENDED".to_string()));
                    snapshot.public.player_state = Some("IDLE".to_string());
                    snapshot.public.position_seconds = Some(0.0);
                    snapshot.public.duration_seconds = None;
                    database_write = Some(("finalize", position.unwrap_or(0.0)));
                }
            }
            "media_loaded" => {
                if device_name.is_some() {
                    snapshot.public.device_name = device_name.clone();
                }
                if let Some((media_file_id, filename)) = snapshot.pending_load.take() {
                    snapshot.public.media_file_id = Some(media_file_id);
                    snapshot.public.filename = Some(filename);
                    database_write = Some(("loaded", media_file_id as f64));
                }
                snapshot.public.last_error = None;
            }
            "media_status" => {
                // Periodic status is also a device heartbeat. This keeps the
                // connected receiver visible even if a session event was lost.
                if device_name.is_some() {
                    snapshot.public.device_name = device_name.clone();
                }
                if player_state.is_some() {
                    snapshot.public.player_state = player_state.clone();
                }
                if let Some(position) = position {
                    snapshot.public.position_seconds = Some(position);
                }
                if let Some(duration_value) = duration {
                    snapshot.public.duration_seconds = Some(duration_value);
                }
                if let Some(volume_value) = volume {
                    snapshot.public.volume_level = Some(volume_value);
                }
                if let Some(muted_value) = muted {
                    snapshot.public.muted = Some(muted_value);
                }
                report.duration_seconds = snapshot.public.duration_seconds;
                let should_write = matches!(
                    player_state.as_deref(),
                    Some("PAUSED") | Some("IDLE") | Some("STOPPED")
                ) || snapshot
                    .last_progress_db_write
                    .map(|last| last.elapsed() >= DB_PROGRESS_WRITE_INTERVAL)
                    .unwrap_or(true);
                if should_write && snapshot.public.media_file_id.is_some() && position.is_some() {
                    snapshot.last_progress_db_write = Some(Instant::now());
                    database_write = Some(("progress", position.unwrap_or(0.0)));
                }
            }
            "media_completed" => {
                // The real receiver reported IdleReason.FINISHED. Token and
                // session correlation are validated by the Phase 5 engine.
                snapshot.public.player_state = Some("IDLE".to_string());
                if let Some(position) = position {
                    snapshot.public.position_seconds = Some(position);
                }
            }
            "load_failed" | "sender_error" => {
                let failure = message
                    .clone()
                    .or(error_code.clone())
                    .unwrap_or_else(|| "The receiver reported an error.".to_string());
                snapshot.public.last_error = Some(failure.clone());
                if let Ok(mut last) = state.last_error.lock() {
                    *last = Some(failure);
                }
            }
            _ => {}
        }
    }

    if let Some((action, value)) = database_write {
        let database = Arc::clone(&state.database);
        let result = tokio::task::spawn_blocking(move || match action {
            "loaded" => {
                let connection = database.lock().map_err(|_| "Local database is unavailable".to_string())?;
                record_cast_media_loaded(&connection, value as i64).map(|_| ())
            }
            "progress" => apply_watch_progress(&database, value),
            "finalize" => finalize_watch_progress(&database, value),
            _ => Ok(()),
        })
        .await;
        if let Ok(Err(message)) = result {
            if let Ok(mut last) = state.last_error.lock() {
                *last = Some(message);
            }
        }
    }

    let _ = state.app.emit("cast-playback-event", report);
}

// ---------------------------------------------------------------------------
// SQLite playback persistence
// ---------------------------------------------------------------------------

/// Stores that a Cast session started playing a media file: the playback state
/// row keeps the current media reference (a disconnect must not lose it) and a
/// fresh history row tracks the session. Returns the history row id.
pub fn record_cast_media_loaded(connection: &Connection, media_file_id: i64) -> Result<i64, String> {
    connection
        .execute(
            "UPDATE playback_state SET current_media_file_id = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = 1",
            [media_file_id],
        )
        .map_err(|_| "Could not save the Cast playback position".to_string())?;
    connection
        .execute("INSERT INTO playback_history(media_file_id) VALUES (?1)", [media_file_id])
        .map_err(|_| "Could not record Cast playback history".to_string())?;
    Ok(connection.last_insert_rowid())
}

fn history_row_for_current_media(connection: &Connection, media_file_id: Option<i64>) -> Option<i64> {
    let media_file_id = media_file_id?;
    connection
        .query_row(
            "SELECT id FROM playback_history WHERE media_file_id = ?1 ORDER BY id DESC LIMIT 1",
            [media_file_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
}

fn apply_watch_progress(connection: &Arc<Mutex<Connection>>, position_seconds: f64) -> Result<(), String> {
    let connection = connection.lock().map_err(|_| "Local database is unavailable".to_string())?;
    let media_file_id = connection
        .query_row(
            "SELECT current_media_file_id FROM playback_state WHERE id = 1",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|_| "Could not read the Cast playback state".to_string())?
        .flatten();
    let Some(history_row) = history_row_for_current_media(&connection, media_file_id) else {
        return Ok(());
    };
    connection
        .execute(
            "UPDATE playback_history SET watched_seconds = MAX(watched_seconds, ?1) WHERE id = ?2",
            params![watched_seconds_value(position_seconds), history_row],
        )
        .map_err(|_| "Could not save Cast playback progress".to_string())?;
    connection
        .execute("UPDATE playback_state SET updated_at = CURRENT_TIMESTAMP WHERE id = 1", [])
        .map_err(|_| "Could not save Cast playback progress".to_string())?;
    Ok(())
}

fn finalize_watch_progress(connection: &Arc<Mutex<Connection>>, position_seconds: f64) -> Result<(), String> {
    apply_watch_progress(connection, position_seconds)
}

fn watched_seconds_value(position_seconds: f64) -> i64 {
    position_seconds.clamp(0.0, 86_400.0).floor() as i64
}

/// Restores the last Cast media reference so the sender page and the UI can
/// display what was playing before an application restart.
pub fn read_cast_current_media(connection: &Connection) -> Result<Option<(i64, String)>, String> {
    connection
        .query_row(
            "SELECT f.id, f.filename FROM playback_state p
             JOIN media_files f ON f.id = p.current_media_file_id
             WHERE p.id = 1 AND f.is_available = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| "Could not read the Cast playback state".to_string())
}

// ---------------------------------------------------------------------------
// Cast sender manager
// ---------------------------------------------------------------------------

struct RunningBridge {
    port: u16,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

struct ChromeChild {
    child: Child,
    pid: u32,
    launched_at: Instant,
}

pub struct CastSenderManager {
    inner: Arc<CastSenderManagerInner>,
}

struct CastSenderManagerInner {
    app: AppHandle,
    database: Arc<Mutex<Connection>>,
    snapshot: Arc<Mutex<CastSnapshotInternal>>,
    broadcast: broadcast::Sender<String>,
    sender_connections: Arc<AtomicUsize>,
    sender_last_seen: Arc<Mutex<Option<Instant>>>,
    last_error: Arc<Mutex<Option<String>>>,
    media_url_probe: Arc<Mutex<Option<ConnectionTestResult>>>,
    bridge: tokio::sync::Mutex<Option<RunningBridge>>,
    chrome: Mutex<Option<ChromeChild>>,
    chrome_window_state: RwLock<String>,
    relaunch_attempts: RwLock<u32>,
    last_launch_attempt_epoch_ms: RwLock<Option<u64>>,
    shutdown_started: AtomicBool,
}

impl Clone for CastSenderManager {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl CastSenderManager {
    pub fn new(app: AppHandle, database: Arc<Mutex<Connection>>) -> Self {
        let (broadcast, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(CastSenderManagerInner {
                app,
                database,
                snapshot: Arc::new(Mutex::new(CastSnapshotInternal::default())),
                broadcast,
                sender_connections: Arc::new(AtomicUsize::new(0)),
                sender_last_seen: Arc::new(Mutex::new(None)),
                last_error: Arc::new(Mutex::new(None)),
                media_url_probe: Arc::new(Mutex::new(None)),
                bridge: tokio::sync::Mutex::new(None),
                chrome: Mutex::new(None),
                chrome_window_state: RwLock::new("not_launched".to_string()),
                relaunch_attempts: RwLock::new(0),
                last_launch_attempt_epoch_ms: RwLock::new(None),
                shutdown_started: AtomicBool::new(false),
            }),
        }
    }

    pub fn snapshot(&self) -> CastPlaybackSnapshot {
        self.inner
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .public
            .clone()
    }

    /// Browser process id of the live managed sender, used to embed its window.
    pub fn sender_pid(&self) -> Option<u32> {
        self.inner
            .chrome
            .try_lock()
            .ok()
            .and_then(|mut guard| {
                guard.as_mut().and_then(|child| match child.child.try_wait() {
                    Ok(None) => Some(child.pid),
                    _ => None,
                })
            })
    }

    /// Marks a media load as pending so the next `media_loaded` event can be
    /// attributed to it.
    pub fn set_pending_load(&self, media_file_id: i64, filename: String) {
        let mut snapshot = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.pending_load = Some((media_file_id, filename));
    }

    pub fn status(&self) -> CastSenderStatus {
        let detection = detect_chrome();
        let (bridge_state, bridge_port) = match self.inner.bridge.try_lock() {
            Ok(guard) => match guard.as_ref() {
                Some(running) if !running.task.is_finished() => ("running".to_string(), Some(running.port)),
                _ => ("stopped".to_string(), None),
            },
            Err(_) => ("starting".to_string(), None),
        };
        let connections = self.inner.sender_connections.load(Ordering::SeqCst);
        let last_seen_ms_ago = self
            .inner
            .sender_last_seen
            .try_lock()
            .ok()
            .and_then(|seen| *seen)
            .map(|instant| instant.elapsed().as_millis() as u64);
        let sender_pid = self
            .inner
            .chrome
            .try_lock()
            .ok()
            .and_then(|mut guard| {
                guard.as_mut().and_then(|child| match child.child.try_wait() {
                    Ok(None) => Some(child.pid),
                    _ => None,
                })
            });
        CastSenderStatus {
            chrome_installed: detection.installed,
            chrome_path: detection.executable_path,
            chrome_source: detection.discovery_source,
            bridge_state,
            bridge_port,
            sender_page_connected: connections > 0,
            sender_page_last_seen_ms_ago: last_seen_ms_ago,
            sender_window_state: self
                .inner
                .chrome_window_state
                .read()
                .map(|state| state.clone())
                .unwrap_or_else(|_| "unknown".to_string()),
            sender_pid,
            relaunch_attempts: self.inner.relaunch_attempts.read().map(|attempts| *attempts).unwrap_or(0),
            last_error: self.inner.last_error.try_lock().ok().and_then(|error| error.clone()),
            last_launch_attempt_epoch_ms: self.inner.last_launch_attempt_epoch_ms.read().map(|value| *value).unwrap_or(None),
        }
    }

    pub fn diagnostics(&self, active_cast_grants: usize) -> CastDiagnosticsSnapshot {
        CastDiagnosticsSnapshot {
            sender: self.status(),
            discovery: self.snapshot(),
            media_url_probe: self
                .inner
                .media_url_probe
                .try_lock()
                .ok()
                .and_then(|probe| probe.clone()),
            active_cast_grants,
        }
    }

    pub fn set_media_url_probe(&self, probe: ConnectionTestResult) {
        if let Ok(mut stored) = self.inner.media_url_probe.lock() {
            *stored = Some(probe);
        }
    }

    pub fn send_command(&self, command: Value) -> Result<(), String> {
        if self.inner.sender_connections.load(Ordering::SeqCst) == 0 {
            return Err("The Cast sender window is not connected. Launch it from the Cast panel.".to_string());
        }
        self.inner
            .broadcast
            .send(command.to_string())
            .map_err(|_| "The Cast sender window is not connected. Launch it from the Cast panel.".to_string())
            .map(|_| ())
    }

    /// Idempotently prepares the sender: detects Chrome, starts the loopback
    /// bridge, opens the application-mode window when needed, and waits until
    /// the sender page actually connects.
    pub async fn launch(&self) -> Result<CastSenderStatus, String> {
        if self.inner.shutdown_started.load(Ordering::SeqCst) {
            return Err("The application is shutting down".to_string());
        }
        let detection = detect_chrome();
        if !detection.installed {
            let message = "Google Chrome is not installed on this PC. Install Chrome from https://www.google.com/chrome and try again.";
            self.record_sender_error(message);
            self.set_window_state("failed");
            return Err(message.to_string());
        }
        let port = self.ensure_bridge().await?;
        if self.inner.sender_connections.load(Ordering::SeqCst) > 0 {
            self.set_window_state("running");
            self.reset_relaunch_attempts();
            return Ok(self.status());
        }
        let executable = PathBuf::from(detection.executable_path.unwrap_or_default());
        self.start_chrome_window(&executable, port)?;
        let deadline = Instant::now() + PAGE_CONNECT_GRACE;
        while Instant::now() < deadline {
            if self.inner.shutdown_started.load(Ordering::SeqCst) {
                return Err("The application is shutting down".to_string());
            }
            if self.inner.sender_connections.load(Ordering::SeqCst) > 0 {
                self.set_window_state("running");
                self.reset_relaunch_attempts();
                self.clear_last_error();
                let _ = self.inner.app.emit("cast-sender-status-changed", self.status());
                return Ok(self.status());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let message = "The Cast sender window opened but did not connect. Check that Chrome is not blocking the local sender page, then try again.";
        self.record_sender_error(message);
        self.set_window_state("failed");
        let _ = self.inner.app.emit("cast-sender-status-changed", self.status());
        Err(message.to_string())
    }

    fn start_chrome_window(&self, executable: &Path, port: u16) -> Result<(), String> {
        let mut guard = self
            .inner
            .chrome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(child) = guard.as_mut() {
            if matches!(child.child.try_wait(), Ok(None)) {
                // The browser process is still alive; whether a window is
                // visible is decided by the watcher and page connectivity.
                return Ok(());
            }
            *guard = None;
        }
        let sender_url = format!("http://127.0.0.1:{port}/");
        let mut command = chrome_launch_command(executable, &sender_url);
        let child = command.spawn().map_err(|error| {
            format!(
                "Chrome could not be started ({}). Try launching it once manually, then cast again.",
                error.kind()
            )
        })?;
        let pid = child.id();
        *guard = Some(ChromeChild { child, pid, launched_at: Instant::now() });
        drop(guard);
        self.set_window_state("launching");
        if let Ok(mut attempt) = self.inner.last_launch_attempt_epoch_ms.write() {
            *attempt = Some(now_epoch_ms());
        }
        self.spawn_watcher();
        Ok(())
    }

    fn spawn_watcher(&self) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.watch_chrome_window().await;
        });
    }

    /// Watches the Chrome child process and the sender page connection to
    /// distinguish: healthy window, user-closed window, and a crashed sender
    /// (auto-relaunched up to three times while a Cast session was active).
    async fn watch_chrome_window(&self) {
        let mut ever_connected = false;
        let mut unconnected_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.inner.shutdown_started.load(Ordering::SeqCst) {
                self.kill_chrome_child();
                return;
            }
            let connected = self.inner.sender_connections.load(Ordering::SeqCst) > 0;
            if connected {
                ever_connected = true;
                unconnected_since = None;
                let was_running = self
                    .inner
                    .chrome_window_state
                    .read()
                    .map(|state| state.as_str() == "running")
                    .unwrap_or(false);
                if !was_running {
                    self.set_window_state("running");
                    self.reset_relaunch_attempts();
                    self.clear_last_error();
                    let _ = self.inner.app.emit("cast-sender-status-changed", self.status());
                }
            } else if ever_connected {
                unconnected_since.get_or_insert(Instant::now());
            }
            let (child_exited, launch_age) = {
                let mut guard = self
                    .inner
                    .chrome
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match guard.as_mut() {
                    Some(child) => (
                        child.child.try_wait().ok().flatten().is_some(),
                        child.launched_at.elapsed(),
                    ),
                    None => return,
                }
            };
            let session_active = self.is_cast_session_active();
            if child_exited {
                if connected || launch_age < PAGE_CONNECT_GRACE {
                    // The launcher process exits when Chrome hands the window
                    // to an existing browser instance; only act once the grace
                    // window passes without a connection.
                    continue;
                }
                let attempts = self.current_relaunch_attempts();
                if session_active && attempts < MAX_RELAUNCH_ATTEMPTS {
                    self.relaunch_chrome().await;
                    continue;
                }
                self.set_window_state(if ever_connected && !session_active { "closed" } else { "failed" });
                self.record_sender_error(if session_active {
                    "The Cast sender window closed and could not be relaunched. Open it again from the Cast panel."
                } else {
                    "The Cast sender window was closed. Cast playback stopped."
                });
                let _ = self.inner.app.emit("cast-sender-status-changed", self.status());
                return;
            }
            // Child alive but the page disappeared for too long: treat the
            // sender page as crashed and relaunch.
            if let Some(since) = unconnected_since {
                if ever_connected && since.elapsed() > PAGE_DISCONNECT_GRACE {
                    if session_active && self.current_relaunch_attempts() < MAX_RELAUNCH_ATTEMPTS {
                        self.relaunch_chrome().await;
                        unconnected_since = None;
                        ever_connected = false;
                    } else if !session_active {
                        self.set_window_state("closed");
                        self.record_sender_error("The Cast sender page stopped responding. Cast playback stopped.");
                        let _ = self.inner.app.emit("cast-sender-status-changed", self.status());
                        return;
                    }
                }
            }
        }
    }

    fn is_cast_session_active(&self) -> bool {
        let snapshot = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .public
            .clone();
        matches!(
            snapshot.session_state.as_deref(),
            Some("SESSION_STARTED") | Some("SESSION_RESUMED")
                | Some("SESSION_STATE_STARTED") | Some("SESSION_STATE_RESUMED")
        ) || matches!(
            snapshot.player_state.as_deref(),
            Some("PLAYING") | Some("PAUSED") | Some("BUFFERING")
        )
    }

    async fn relaunch_chrome(&self) {
        let attempts = {
            let mut attempts = self
                .inner
                .relaunch_attempts
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *attempts += 1;
            *attempts
        };
        self.set_window_state("launching");
        self.record_sender_error(&format!(
            "The Cast sender window stopped responding. Relaunching automatically (attempt {attempts})."
        ));
        let _ = self.inner.app.emit("cast-sender-status-changed", self.status());
        let detection = detect_chrome();
        if !detection.installed {
            self.set_window_state("failed");
            self.record_sender_error("Chrome disappeared from this PC. Install Chrome and try casting again.");
            return;
        }
        let Some(port) = self.current_bridge_port().await else {
            return;
        };
        let executable = PathBuf::from(detection.executable_path.unwrap_or_default());
        if let Err(message) = self.start_chrome_window(&executable, port) {
            self.record_sender_error(&message);
            self.set_window_state("failed");
        }
    }

    async fn current_bridge_port(&self) -> Option<u16> {
        self.inner
            .bridge
            .lock()
            .await
            .as_ref()
            .map(|running| running.port)
    }

    async fn ensure_bridge(&self) -> Result<u16, String> {
        let mut bridge = self.inner.bridge.lock().await;
        if let Some(running) = bridge.as_ref() {
            if !running.task.is_finished() {
                return Ok(running.port);
            }
            running.shutdown.cancel();
            *bridge = None;
        }
        let mut bound = None;
        let mut last_error = None;
        for port in SENDER_BRIDGE_PREFERRED_PORT..=SENDER_BRIDGE_LAST_PORT {
            match tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => {
                    bound = Some((listener, port));
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let (listener, port) = bound.ok_or_else(|| {
            format!(
                "No local sender port was available ({SENDER_BRIDGE_PREFERRED_PORT}–{SENDER_BRIDGE_LAST_PORT}): {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "all ports busy".to_string())
            )
        })?;
        let state = CastBridgeState {
            app: self.inner.app.clone(),
            database: Arc::clone(&self.inner.database),
            snapshot: Arc::clone(&self.inner.snapshot),
            broadcast: self.inner.broadcast.clone(),
            sender_connections: Arc::clone(&self.inner.sender_connections),
            sender_last_seen: Arc::clone(&self.inner.sender_last_seen),
            last_error: Arc::clone(&self.inner.last_error),
            media_url_probe: Arc::clone(&self.inner.media_url_probe),
        };
        let router = sender_bridge_router(state);
        let shutdown = CancellationToken::new();
        let graceful = shutdown.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service())
                .with_graceful_shutdown(graceful.cancelled_owned())
                .await;
        });
        *bridge = Some(RunningBridge { port, shutdown, task });
        Ok(port)
    }

    fn kill_chrome_child(&self) {
        let mut guard = self
            .inner
            .chrome
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mut child) = guard.take() {
            let _ = child.child.kill();
            let _ = child.child.wait();
        }
    }

    /// Notifies the sender page to close itself, then stops the bridge and the
    /// Chrome child process.
    pub async fn shutdown(&self) {
        self.inner.shutdown_started.store(true, Ordering::SeqCst);
        let _ = self
            .inner
            .broadcast
            .send(serde_json::json!({ "type": "control", "action": "close_sender" }).to_string());
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.kill_chrome_child();
        let bridge = self.inner.bridge.lock().await.take();
        if let Some(bridge) = bridge {
            bridge.shutdown.cancel();
        }
        self.set_window_state("not_launched");
    }

    fn set_window_state(&self, state: &str) {
        if let Ok(mut current) = self.inner.chrome_window_state.write() {
            *current = state.to_string();
        }
    }

    fn current_relaunch_attempts(&self) -> u32 {
        self.inner.relaunch_attempts.read().map(|attempts| *attempts).unwrap_or(0)
    }

    fn reset_relaunch_attempts(&self) {
        if let Ok(mut attempts) = self.inner.relaunch_attempts.write() {
            *attempts = 0;
        }
    }

    fn record_sender_error(&self, message: &str) {
        if let Ok(mut error) = self.inner.last_error.lock() {
            *error = Some(message.to_string());
        }
        let mut snapshot = self
            .inner
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshot.public.last_error = Some(message.to_string());
    }

    fn clear_last_error(&self) {
        if let Ok(mut error) = self.inner.last_error.lock() {
            *error = None;
        }
        if let Ok(mut snapshot) = self.inner.snapshot.lock() {
            snapshot.public.last_error = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_database;

    #[test]
    fn chrome_candidate_resolution_picks_the_first_existing_executable() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("missing-chrome");
        let second = temporary.path().join("chrome.exe");
        std::fs::write(&second, b"MZ fake chrome executable").unwrap();
        let candidates = vec![(first, "program-files"), (second.clone(), "program-files-x86")];
        let resolved = resolve_existing_executable(&candidates).unwrap();
        assert_eq!(resolved.0, second);
        assert_eq!(resolved.1, "program-files-x86");
        assert!(resolve_existing_executable(&[]).is_none());
    }

    #[test]
    fn chrome_launch_command_uses_application_mode_with_safe_flags() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = temporary.path().join("chrome.exe");
        std::fs::write(&executable, b"MZ fake chrome executable").unwrap();
        let command = chrome_launch_command(&executable, "http://127.0.0.1:8790/");
        assert_eq!(command.get_program(), executable.as_os_str());
        let arguments: Vec<String> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(arguments[0], "--app=http://127.0.0.1:8790/");
        assert!(arguments.contains(&"--new-window".to_string()));
        assert!(arguments.contains(&"--no-first-run".to_string()));
        assert!(arguments.contains(&"--no-default-browser-check".to_string()));
    }

    #[test]
    fn sender_page_embeds_official_cast_framework_and_no_secrets() {
        assert!(SENDER_HTML.contains("https://www.gstatic.com/cv/js/sender/v1/cast_sender.js"));
        assert!(SENDER_HTML.contains("/sender.js"));
        assert!(!SENDER_HTML.contains("cast_token"));
        assert!(SENDER_JS.contains("DEFAULT_MEDIA_RECEIVER_APP_ID"));
        assert!(SENDER_JS.contains("loadMedia"));
        assert!(SENDER_JS.contains("setVolumeLevel"));
        assert!(SENDER_JS.contains("endCurrentSession"));
        assert!(SENDER_JS.contains("RemotePlayerController"));
    }

    #[test]
    fn stale_sender_session_events_cannot_clear_the_active_receiver() {
        let active = Some("session-123".to_string());
        let same = Some("session-123".to_string());
        let other = Some("session-999".to_string());
        assert!(session_event_matches(active.as_ref(), same.as_ref()));
        assert!(!session_event_matches(active.as_ref(), other.as_ref()));
        // A second sender's failure has no session id; it must not win while a
        // real session is active.
        assert!(!session_event_matches(active.as_ref(), None));
        // With no session tracked, the first failure/end event is authoritative.
        assert!(session_event_matches(None, None));
        assert!(session_event_matches(None, other.as_ref()));
    }

    #[test]
    fn sender_page_csp_upgrades_the_cast_loaders_insecure_script_urls() {
        // Google's loader injects //www.gstatic.com/... scripts; on this HTTP
        // origin they must be upgraded to HTTPS before the CSP allow-list is
        // applied, otherwise the Cast framework cannot load at all.
        let response = secure_page_response(SENDER_HTML, "text/html; charset=utf-8");
        let policy = response
            .headers()
            .get(CONTENT_SECURITY_POLICY)
            .expect("sender page must set a CSP")
            .to_str()
            .expect("CSP must be valid UTF-8")
            .to_string();
        assert!(policy.contains("upgrade-insecure-requests"), "CSP was: {policy}");
        assert!(policy.contains("script-src 'self' https://www.gstatic.com"));
        assert!(!policy.contains("http://www.gstatic.com"));
    }

    #[test]
    fn playback_event_clamping_bounds_untrusted_strings() {
        let long = "x".repeat(1000);
        let event = serde_json::json!({
            "type": "media_status",
            "player_state": long,
            "device_name": long,
            "position_seconds": 12.5,
            "duration_seconds": "not-a-number",
            "muted": true,
        });
        assert_eq!(clamp_string(event.get("player_state"), 40).map(|value| value.len()), Some(40));
        assert_eq!(clamp_string(event.get("device_name"), 120).map(|value| value.len()), Some(120));
        assert_eq!(clamp_number(event.get("position_seconds")), Some(12.5));
        assert_eq!(clamp_number(event.get("duration_seconds")), None);
        assert_eq!(event.get("muted").and_then(Value::as_bool), Some(true));
        assert_eq!(clamp_string(event.get("missing"), 10), None);
        assert_eq!(clamp_string(Some(&serde_json::json!("")), 10), None);
    }

    #[test]
    fn watched_seconds_are_floored_and_bounded() {
        assert_eq!(watched_seconds_value(0.4), 0);
        assert_eq!(watched_seconds_value(91.9), 91);
        assert_eq!(watched_seconds_value(-5.0), 0);
        assert_eq!(watched_seconds_value(999_999.0), 86_400);
    }

    fn fixture_connection() -> (tempfile::TempDir, Connection) {
        let temporary = tempfile::tempdir().unwrap();
        let connection = open_database(&temporary.path().join("data")).unwrap();
        connection
            .execute(
                "INSERT INTO media_directories(path, is_active, availability_state) VALUES ('C:/media', 1, 'available')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO media_files(directory_id, path, filename, extension, size_bytes, modified_at)
                 VALUES (1, 'C:/media/movie.mp4', 'movie.mp4', 'mp4', 123, '2024-01-01 00:00:00')",
                [],
            )
            .unwrap();
        let media_file_id = connection.last_insert_rowid();
        (temporary, connection)
    }

    #[test]
    fn cast_playback_state_is_persisted_and_restored() {
        let (_temporary, connection) = fixture_connection();
        assert!(read_cast_current_media(&connection).unwrap().is_none());
        let media_file_id: i64 = connection
            .query_row("SELECT id FROM media_files", [], |row| row.get(0))
            .unwrap();
        let history_row = record_cast_media_loaded(&connection, media_file_id).unwrap();
        assert!(history_row > 0);
        let restored = read_cast_current_media(&connection).unwrap().unwrap();
        assert_eq!(restored.0, media_file_id);
        assert_eq!(restored.1, "movie.mp4");
        let shared = Arc::new(Mutex::new(connection));
        apply_watch_progress(&shared, 75.9).unwrap();
        finalize_watch_progress(&shared, 80.0).unwrap();
        let connection = shared.lock().unwrap();
        let watched: i64 = connection
            .query_row(
                "SELECT watched_seconds FROM playback_history WHERE id = ?1",
                [history_row],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(watched, 80);
        let state_media: i64 = connection
            .query_row("SELECT current_media_file_id FROM playback_state WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state_media, media_file_id);
    }

    #[test]
    fn disconnect_keeps_the_current_media_reference_for_the_playlist() {
        let (_temporary, connection) = fixture_connection();
        let media_file_id: i64 = connection
            .query_row("SELECT id FROM media_files", [], |row| row.get(0))
            .unwrap();
        record_cast_media_loaded(&connection, media_file_id).unwrap();
        // A disconnect only finalizes progress; nothing clears playback_state.
        let shared = Arc::new(Mutex::new(connection));
        finalize_watch_progress(&shared, 42.0).unwrap();
        let connection = shared.lock().unwrap();
        let state_media: Option<i64> = connection
            .query_row("SELECT current_media_file_id FROM playback_state WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state_media, Some(media_file_id));
    }
}
