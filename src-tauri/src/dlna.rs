//! DLNA/UPnP AV transport for smart TVs (LG webOS, Samsung, Sony, Philips, …).
//!
//! This is a second playback transport next to the Google Cast sender. It does
//! not replace or modify the Cast path: discovery is a real SSDP M-SEARCH,
//! playback uses the renderer's own AVTransport service, and the video is
//! fetched from the existing LAN media server with the same short-lived,
//! media-scoped `cast_token` grant. No credentials, streaming URLs or raw
//! device responses are stored, logged or sent to the frontend.
//!
//! Renderer state is polled with real SOAP reads. Playlist progression is only
//! triggered when the renderer reports a genuine STOPPED/NO_MEDIA_PRESENT
//! transport state after playing, correlated with the Phase 5 playback token.

use crate::{
    cast_discovery::CastDevice,
    cast_sender::{CastEventReport, CastPlaybackSnapshot},
    network::{is_private_lan_ipv4, select_lan_address},
};
use serde::Serialize;
use std::{
    collections::HashSet,
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use url::Url;

const SSDP_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const SSDP_PORT: u16 = 1900;
const RENDERER_TARGET: &str = "urn:schemas-upnp-org:device:MediaRenderer:1";
const AVTRANSPORT_SERVICE: &str = "urn:schemas-upnp-org:service:AVTransport:1";
const RENDERING_SERVICE: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
const MAX_PACKET: usize = 8192;
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

fn clean_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}

fn local_name(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}

pub(crate) fn element_text(xml: &str, wanted: &str) -> Option<String> {
    let mut cursor = 0usize;
    while let Some(offset) = xml[cursor..].find('<') {
        let start = cursor + offset;
        let end = start + xml[start..].find('>')? + 1;
        let tag = &xml[start + 1..end - 1];
        if !tag.starts_with('/') {
            let name = tag
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches('/');
            if local_name(name) == wanted {
                let closing = format!("</{}>", name);
                if let Some(close_offset) = xml[end..].find(&closing) {
                    let value = &xml[end..end + close_offset];
                    return Some(unescape_xml(value.trim()));
                }
            }
        }
        cursor = end;
    }
    None
}

fn service_blocks(xml: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = xml[cursor..].find("<service") {
        let start = cursor + offset;
        let Some(tag_end_offset) = xml[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_offset + 1;
        if xml[start..tag_end].ends_with("/>") {
            cursor = tag_end;
            continue;
        }
        let Some(close_offset) = xml[tag_end..].find("</service>") else {
            break;
        };
        let close = tag_end + close_offset + "</service>".len();
        blocks.push(xml[start..close].to_string());
        cursor = close;
    }
    blocks
}

fn ssdp_location(packet: &str) -> Option<String> {
    let mut lines = packet.lines();
    let status = lines.next()?;
    if !status.contains(" 200 ") {
        return None;
    }
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("location") {
                let location = clean_text(value.trim(), 512);
                if !location.is_empty() {
                    return Some(location);
                }
            }
        }
    }
    None
}

fn private_endpoint(url: &Url) -> Result<(String, u16, String), String> {
    if url.scheme() != "http" {
        return Err("The renderer description must use HTTP.".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "The renderer has no host.".to_string())?;
    let ip = host
        .parse::<Ipv4Addr>()
        .map_err(|_| "The renderer host is not a LAN IPv4 address.".to_string())?;
    if !is_private_lan_ipv4(ip) {
        return Err("The renderer is outside the local network.".to_string());
    }
    let port = url.port_or_known_default().unwrap_or(80);
    let path = format!(
        "{}{}",
        url.path(),
        url.query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    );
    Ok((
        host.to_string(),
        port,
        if path.is_empty() {
            "/".to_string()
        } else {
            path
        },
    ))
}

fn read_http_response(stream: &mut TcpStream) -> Result<(u16, String), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("The renderer closed the connection before responding.".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > 64 * 1024 {
            return Err("The renderer returned an oversized response header.".to_string());
        }
    };
    let header = String::from_utf8_lossy(&buffer[..header_end]);
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let content_length = header
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if key.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    let chunked = header.lines().any(|line| {
        line.split_once(':')
            .map(|(key, value)| {
                key.eq_ignore_ascii_case("transfer-encoding")
                    && value.to_ascii_lowercase().contains("chunked")
            })
            .unwrap_or(false)
    });
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let mut body = buffer[header_end..].to_vec();
    if chunked && content_length == 0 {
        body.clear();
        loop {
            let size_line = loop {
                if let Some(position) = body.windows(2).position(|window| window == b"\r\n") {
                    let line = String::from_utf8_lossy(&body[..position]).to_string();
                    body.drain(..position + 2);
                    break line;
                }
                let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
                if read == 0 {
                    return Err("The renderer returned an incomplete chunked response.".to_string());
                }
                body.extend_from_slice(&chunk[..read]);
            };
            let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| "The renderer returned an invalid chunk size.".to_string())?;
            if size == 0 {
                break;
            }
            while body.len() < size + 2 {
                let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
                if read == 0 {
                    return Err("The renderer returned an incomplete chunk.".to_string());
                }
                body.extend_from_slice(&chunk[..read]);
            }
            let mut decoded = Vec::new();
            decoded.extend_from_slice(&body[..size]);
            body.drain(..size + 2);
            buffer.truncate(header_end);
            buffer.extend_from_slice(&decoded);
        }
        body = buffer[header_end..].to_vec();
    }
    let text = String::from_utf8_lossy(&body).to_string();
    Ok((status, text))
}

pub(crate) fn http_exchange(
    method: &str,
    url: &str,
    extra_headers: &[(&str, String)],
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let parsed = Url::parse(url).map_err(|_| "The renderer URL is invalid.".to_string())?;
    let (host, port, path) = private_endpoint(&parsed)?;
    let endpoint = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|_| "The renderer address is invalid.".to_string())?
        .next()
        .ok_or_else(|| "The renderer address is invalid.".to_string())?;
    let mut stream = TcpStream::connect_timeout(&endpoint, HTTP_TIMEOUT)
        .map_err(|_| "The renderer did not accept a local connection.".to_string())?;
    let _ = stream.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_TIMEOUT));
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n");
    for (key, value) in extra_headers {
        request.push_str(&format!("{key}: {}\r\n", clean_text(value, 1024)));
    }
    if let Some(body) = body {
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    let _ = stream.flush();
    read_http_response(&mut stream)
}

fn fetch_description(location: &str) -> Option<CastDevice> {
    let (status, xml) = http_exchange("GET", location, &[], None).ok()?;
    if status != 200 {
        return None;
    }
    let parsed = Url::parse(location).ok()?;
    let host = parsed.host_str()?.to_string();
    let address = host.parse::<Ipv4Addr>().ok()?;
    if !is_private_lan_ipv4(address) {
        return None;
    }
    let mut avtransport = None;
    let mut rendering = None;
    for block in service_blocks(&xml) {
        let Some(service_type) = element_text(&block, "serviceType") else {
            continue;
        };
        let Some(control) = element_text(&block, "controlURL") else {
            continue;
        };
        let resolved = parsed.join(&control).ok()?.to_string();
        if service_type.contains(AVTRANSPORT_SERVICE) {
            avtransport = Some(resolved);
        } else if service_type.contains(RENDERING_SERVICE) {
            rendering = Some(resolved);
        }
    }
    let avtransport_url = avtransport?;
    let name = element_text(&xml, "friendlyName")
        .map(|value| clean_text(&value, 120))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("DLNA device at {host}"));
    let model = element_text(&xml, "modelName")
        .map(|value| clean_text(&value, 80))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "DLNA MediaRenderer".to_string());
    let id = element_text(&xml, "UDN")
        .map(|value| clean_text(&value, 160))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| location.to_string());
    Some(CastDevice {
        id,
        name,
        model,
        address: host,
        port: parsed.port_or_known_default().unwrap_or(80),
        source: "SSDP".to_string(),
        transport: "dlna".to_string(),
        avtransport_url: Some(avtransport_url),
        rendering_url: rendering,
        description_url: Some(location.to_string()),
    })
}

/// Runs a real SSDP discovery pass for UPnP AV MediaRenderers. Called on a
/// blocking thread from the command layer, like the mDNS Cast discovery.
pub fn discover(timeout: Duration) -> Result<Vec<CastDevice>, String> {
    let interfaces = crate::network::detect_network().interfaces;
    let selected = select_lan_address(&interfaces);
    let mut candidates: Vec<Ipv4Addr> = interfaces
        .iter()
        .filter(|interface| {
            interface.is_up && interface.is_running && !interface.is_loopback && !interface.is_tun
        })
        .flat_map(|interface| interface.ipv4_addresses.iter())
        .filter_map(|address| address.parse::<Ipv4Addr>().ok())
        .filter(|ip| is_private_lan_ipv4(*ip))
        .collect();
    if let Some(ip) = selected
        .as_ref()
        .and_then(|address| address.ipv4.parse::<Ipv4Addr>().ok())
    {
        if !candidates.contains(&ip) {
            candidates.insert(0, ip);
        }
    }
    let mut sockets = Vec::new();
    for ip in candidates {
        let Ok(socket) = UdpSocket::bind((ip, 0)) else {
            continue;
        };
        let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
        let _ = socket.set_multicast_loop_v4(true);
        let _ = socket.set_multicast_ttl_v4(4);
        if socket.join_multicast_v4(&SSDP_GROUP, &ip).is_err() {
            continue;
        }
        sockets.push(socket);
    }
    if sockets.is_empty() {
        return Err("Could not open a local socket for DLNA discovery.".to_string());
    }
    for target in [RENDERER_TARGET, "ssdp:all"] {
        let query = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_GROUP}:{SSDP_PORT}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n"
        );
        for socket in &sockets {
            let _ = socket.send_to(query.as_bytes(), (SSDP_GROUP, SSDP_PORT));
        }
    }
    let deadline = Instant::now() + timeout;
    let mut locations = HashSet::new();
    let mut buffer = [0u8; MAX_PACKET];
    while Instant::now() < deadline {
        for socket in &sockets {
            match socket.recv_from(&mut buffer) {
                Ok((length, _)) => {
                    if let Ok(packet) = std::str::from_utf8(&buffer[..length]) {
                        if let Some(location) = ssdp_location(packet) {
                            locations.insert(location);
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }
    let mut devices: Vec<CastDevice> = locations
        .iter()
        .filter_map(|location| fetch_description(location))
        .collect();
    devices.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    devices.dedup_by(|left, right| left.id == right.id && left.address == right.address);
    Ok(devices)
}

fn soap(
    device: &CastDevice,
    service: &str,
    action: &str,
    arguments: &str,
) -> Result<String, String> {
    let endpoint = if service == AVTRANSPORT_SERVICE {
        device.avtransport_url.as_deref()
    } else {
        device.rendering_url.as_deref()
    }
    .ok_or_else(|| "The renderer does not expose the required control service.".to_string())?;
    let envelope = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
         <s:Body><u:{action} xmlns:u=\"{service}\" xmlns=\"{service}\">{arguments}</u:{action}></s:Body></s:Envelope>"
    );
    let soap_action = format!("\"{service}#{action}\"");
    let (status, body) = http_exchange(
        "POST",
        endpoint,
        &[
            ("Content-Type", "text/xml; charset=\"utf-8\"".to_string()),
            ("SOAPACTION", soap_action),
        ],
        Some(&envelope),
    )?;
    if status != 200 {
        return Err(format!("The renderer rejected {action} (HTTP {status})."));
    }
    Ok(body)
}

fn seconds_from_clock(value: &str) -> Option<f64> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let hours = parts[0].parse::<f64>().ok()?;
    let minutes = parts[1].parse::<f64>().ok()?;
    let seconds = parts[2].parse::<f64>().ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

fn clock_from_seconds(value: f64) -> String {
    let total = value.max(0.0) as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

#[derive(Clone, Debug)]
struct ActivePlayback {
    device: CastDevice,
    generation: u64,
    playback_token: String,
    cast_session_id: String,
    media_session_id: i64,
    suppress_completion: bool,
    saw_playing: bool,
}

#[derive(Clone, Debug)]
struct PollResult {
    position_seconds: Option<f64>,
    duration_seconds: Option<f64>,
    player_state: String,
    volume_level: Option<f64>,
    muted: Option<bool>,
    finished: bool,
}

#[derive(Clone)]
pub struct DlnaPlaybackRequest {
    pub device: CastDevice,
    pub url: String,
    pub media_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub position_seconds: f64,
    pub playback_token: String,
    /// Generated once per load by the orchestrator and echoed by every report.
    pub cast_session_id: String,
}

struct DlnaManagerInner {
    app: AppHandle,
    selected: Mutex<Option<CastDevice>>,
    active: Mutex<Option<ActivePlayback>>,
    snapshot: Mutex<CastPlaybackSnapshot>,
    /// Media-scoped grant waiting to be retired by a correlated completion.
    last_grant: Mutex<Option<(i64, String)>>,
    /// Incremented on every load so a stale poller cannot report (or complete)
    /// a previous item after the engine has already advanced.
    generation: AtomicU64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DlnaStatus {
    pub selected: Option<CastDevice>,
    pub snapshot: CastPlaybackSnapshot,
}

#[derive(Clone)]
pub struct DlnaManager(Arc<DlnaManagerInner>);

impl DlnaManager {
    pub fn new(app: AppHandle) -> Self {
        Self(Arc::new(DlnaManagerInner {
            app,
            selected: Mutex::new(None),
            active: Mutex::new(None),
            snapshot: Mutex::new(CastPlaybackSnapshot::default()),
            last_grant: Mutex::new(None),
            generation: AtomicU64::new(0),
        }))
    }

    pub fn selected(&self) -> Option<CastDevice> {
        self.0.selected.lock().ok().and_then(|guard| guard.clone())
    }

    /// True when the user chose a DLNA renderer, even before playback starts.
    pub fn is_active(&self) -> bool {
        self.0
            .selected
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// True only while an authoritative load owns a renderer session. Avoids
    /// replaying a finished item through the UI without a fresh token/poller.
    pub fn has_active_playback(&self) -> bool {
        self.active_report().is_some()
    }

    fn next_generation(&self) -> u64 {
        self.0.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Invalidates every poller for the previous load before active state is
    /// replaced, closing the stop/play handoff race.
    fn invalidate(&self) -> u64 {
        let generation = self.next_generation();
        if let Ok(mut guard) = self.0.active.lock() {
            *guard = None;
        }
        generation
    }

    fn clear_snapshot(&self) {
        if let Ok(mut snapshot) = self.0.snapshot.lock() {
            *snapshot = CastPlaybackSnapshot {
                player_state: Some("STOPPED".to_string()),
                updated_at_epoch_ms: now_ms(),
                ..CastPlaybackSnapshot::default()
            };
        }
    }

    fn clear_grant(&self) {
        if let Ok(mut guard) = self.0.last_grant.lock() {
            *guard = None;
        }
    }

    /// Returns the grant for a completed load only when its DLNA session id
    /// matches. A later load cannot retire the wrong URL.
    pub fn take_media_id_for_session(&self, cast_session_id: &str) -> Option<i64> {
        let mut guard = self.0.last_grant.lock().ok()?;
        let matches = guard
            .as_ref()
            .is_some_and(|(_, session)| session == cast_session_id);
        if matches {
            guard.take().map(|(media_id, _)| media_id)
        } else {
            None
        }
    }

    pub fn snapshot(&self) -> CastPlaybackSnapshot {
        self.0
            .snapshot
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn status(&self) -> DlnaStatus {
        DlnaStatus {
            selected: self.selected(),
            snapshot: self.snapshot(),
        }
    }

    pub fn select(&self, device: CastDevice) -> Result<(), String> {
        if device.transport != "dlna" {
            return Err("That device does not use the DLNA transport.".to_string());
        }
        let address = device
            .address
            .parse::<Ipv4Addr>()
            .map_err(|_| "The renderer has an invalid LAN address.".to_string())?;
        if !is_private_lan_ipv4(address) {
            return Err("The renderer is outside the local network.".to_string());
        }
        for endpoint in [&device.avtransport_url, &device.rendering_url]
            .into_iter()
            .flatten()
        {
            let parsed = Url::parse(endpoint)
                .map_err(|_| "The renderer control URL is invalid.".to_string())?;
            private_endpoint(&parsed)?;
        }
        let already_selected = self
            .selected()
            .map(|current| current.id == device.id && current.address == device.address)
            .unwrap_or(false);
        if already_selected {
            return Ok(());
        }
        let _ = self.stop_internal(true);
        let mut selected = self
            .0
            .selected
            .lock()
            .map_err(|_| "DLNA state is unavailable".to_string())?;
        *selected = Some(device);
        Ok(())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        let had_active = self.active_report().is_some();
        let _ = self.stop_internal(false);
        let mut selected = self
            .0
            .selected
            .lock()
            .map_err(|_| "DLNA state is unavailable".to_string())?;
        let had_selected = selected.is_some();
        *selected = None;
        drop(selected);
        if had_active || had_selected {
            self.emit(CastEventReport {
                kind: "session_ended".to_string(),
                device_name: None,
                cast_state: Some("DISCONNECTED".to_string()),
                session_state: Some("DISCONNECTED".to_string()),
                player_state: None,
                position_seconds: None,
                duration_seconds: None,
                message: None,
                playback_token: None,
                cast_session_id: None,
                media_session_id: None,
                idle_reason: None,
            });
        }
        Ok(())
    }

    pub fn start_playback(&self, request: DlnaPlaybackRequest) -> Result<(), String> {
        if request.device.transport != "dlna" {
            return Err("Playback needs a DLNA renderer.".to_string());
        }
        let _ = self.stop_internal(true);
        let didl = format!(
            "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
             xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\">\
             <item id=\"0\" parentID=\"-1\" restricted=\"1\">\
             <dc:title>{}</dc:title>\
             <upnp:class>object.item.videoItem.movie</upnp:class>\
             <res protocolInfo=\"http-get:*:{}:*\">{}</res>\
             </item></DIDL-Lite>",
            escape_xml(&request.filename),
            escape_xml(&request.mime_type),
            escape_xml(&request.url),
        );
        let set_uri = format!(
            "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
            escape_xml(&request.url),
            escape_xml(&didl),
        );
        if let Err(error) = soap(
            &request.device,
            AVTRANSPORT_SERVICE,
            "SetAVTransportURI",
            &set_uri,
        ) {
            self.emit_load_failed(&request.playback_token, &error);
            return Err(error);
        }
        if request.position_seconds > 1.0 {
            let seek = format!(
                "<InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{}</Target>",
                clock_from_seconds(request.position_seconds)
            );
            if let Err(error) = soap(&request.device, AVTRANSPORT_SERVICE, "Seek", &seek) {
                self.emit_load_failed(&request.playback_token, &error);
                return Err(error);
            }
        }
        if let Err(error) = soap(
            &request.device,
            AVTRANSPORT_SERVICE,
            "Play",
            "<InstanceID>0</InstanceID><Speed>1</Speed>",
        ) {
            self.emit_load_failed(&request.playback_token, &error);
            return Err(error);
        }
        // Invalidate before publishing the new session so any slow SOAP poll
        // from the previous item exits instead of reporting against this one.
        let generation = self.invalidate();
        let active = ActivePlayback {
            device: request.device.clone(),
            generation,
            playback_token: request.playback_token.clone(),
            cast_session_id: request.cast_session_id.clone(),
            media_session_id: 1,
            suppress_completion: false,
            saw_playing: false,
        };
        if let Ok(mut guard) = self.0.active.lock() {
            *guard = Some(active);
        }
        if let Ok(mut guard) = self.0.last_grant.lock() {
            *guard = Some((request.media_id, request.cast_session_id.clone()));
        }
        {
            let mut snapshot = self
                .0
                .snapshot
                .lock()
                .map_err(|_| "DLNA state is unavailable".to_string())?;
            *snapshot = CastPlaybackSnapshot {
                media_file_id: Some(request.media_id),
                filename: Some(request.filename.clone()),
                player_state: Some("PLAYING".to_string()),
                position_seconds: Some(request.position_seconds),
                duration_seconds: None,
                volume_level: None,
                muted: None,
                device_name: Some(request.device.name.clone()),
                cast_state: Some("CONNECTED".to_string()),
                session_state: Some("CONNECTED".to_string()),
                last_error: None,
                updated_at_epoch_ms: now_ms(),
            };
        }
        self.emit_session(&request, "session_started");
        self.emit_session(&request, "media_loaded");
        self.emit(CastEventReport {
            kind: "media_status".to_string(),
            device_name: Some(request.device.name.clone()),
            cast_state: Some("CONNECTED".to_string()),
            session_state: Some("CONNECTED".to_string()),
            player_state: Some("PLAYING".to_string()),
            position_seconds: Some(request.position_seconds),
            duration_seconds: None,
            message: None,
            playback_token: Some(request.playback_token.clone()),
            cast_session_id: Some(request.cast_session_id.clone()),
            media_session_id: Some(1),
            idle_reason: None,
        });
        self.spawn_poller(generation);
        Ok(())
    }

    pub fn control(
        &self,
        action: &str,
        position_seconds: Option<f64>,
        volume: Option<f64>,
        muted: Option<bool>,
    ) -> Result<(), String> {
        let Some(device) = self.selected() else {
            return Err("No DLNA renderer is selected.".to_string());
        };
        if matches!(action, "play" | "pause" | "toggle_play" | "stop" | "seek")
            && !self.has_active_playback()
        {
            return Err(
                "Start playback from the library before controlling this renderer.".to_string(),
            );
        }
        match action {
            "play" => {
                soap(
                    &device,
                    AVTRANSPORT_SERVICE,
                    "Play",
                    "<InstanceID>0</InstanceID><Speed>1</Speed>",
                )?;
                self.set_player_state("PLAYING");
            }
            "pause" => {
                soap(
                    &device,
                    AVTRANSPORT_SERVICE,
                    "Pause",
                    "<InstanceID>0</InstanceID>",
                )?;
                self.set_player_state("PAUSED");
            }
            "toggle_play" => {
                let playing = self
                    .snapshot()
                    .player_state
                    .as_deref()
                    .map(|state| state == "PLAYING")
                    .unwrap_or(false);
                if playing {
                    return self.control("pause", None, None, None);
                }
                return self.control("play", None, None, None);
            }
            "stop" => {
                let _ = self.stop_internal(true);
            }
            "seek" => {
                let Some(position) =
                    position_seconds.filter(|value| value.is_finite() && *value >= 0.0)
                else {
                    return Err("A valid seek position is required.".to_string());
                };
                let arguments = format!(
                    "<InstanceID>0</InstanceID><Unit>REL_TIME</Unit><Target>{}</Target>",
                    clock_from_seconds(position)
                );
                soap(&device, AVTRANSPORT_SERVICE, "Seek", &arguments)?;
                if let Ok(mut snapshot) = self.0.snapshot.lock() {
                    snapshot.position_seconds = Some(position);
                    snapshot.updated_at_epoch_ms = now_ms();
                }
            }
            "set_volume" => {
                let Some(level) = volume.filter(|value| value.is_finite() && *value >= 0.0) else {
                    return Err("A valid volume level is required.".to_string());
                };
                let percent =
                    if level <= 1.0 { level * 100.0 } else { level }.clamp(0.0, 100.0) as u32;
                let arguments = format!(
                    "<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>{percent}</DesiredVolume>"
                );
                soap(&device, RENDERING_SERVICE, "SetVolume", &arguments)?;
                if let Ok(mut snapshot) = self.0.snapshot.lock() {
                    snapshot.volume_level = Some(percent as f64 / 100.0);
                    snapshot.updated_at_epoch_ms = now_ms();
                }
            }
            "set_mute" => {
                let Some(muted) = muted else {
                    return Err("A mute value is required.".to_string());
                };
                let arguments = format!(
                    "<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredMute>{}</DesiredMute>",
                    u8::from(muted)
                );
                soap(&device, RENDERING_SERVICE, "SetMute", &arguments)?;
                if let Ok(mut snapshot) = self.0.snapshot.lock() {
                    snapshot.muted = Some(muted);
                    snapshot.updated_at_epoch_ms = now_ms();
                }
            }
            _ => return Err("That control is not available for DLNA playback.".to_string()),
        }
        Ok(())
    }

    fn set_player_state(&self, state: &str) {
        let Some(active) = self.active_report() else {
            return;
        };
        if let Ok(mut snapshot) = self.0.snapshot.lock() {
            snapshot.player_state = Some(state.to_string());
            snapshot.updated_at_epoch_ms = now_ms();
        }
        self.emit(CastEventReport {
            kind: "media_status".to_string(),
            device_name: Some(active.device.name.clone()),
            cast_state: Some("CONNECTED".to_string()),
            session_state: Some("CONNECTED".to_string()),
            player_state: Some(state.to_string()),
            position_seconds: None,
            duration_seconds: None,
            message: None,
            playback_token: Some(active.playback_token),
            cast_session_id: Some(active.cast_session_id),
            media_session_id: Some(active.media_session_id),
            idle_reason: None,
        });
    }

    fn active_report(&self) -> Option<ActivePlayback> {
        self.0.active.lock().ok().and_then(|guard| guard.clone())
    }

    fn emit(&self, report: CastEventReport) {
        let _ = self.0.app.emit("cast-playback-event", report);
    }

    fn emit_session(&self, request: &DlnaPlaybackRequest, kind: &str) {
        self.emit(CastEventReport {
            kind: kind.to_string(),
            device_name: Some(request.device.name.clone()),
            cast_state: Some("CONNECTED".to_string()),
            session_state: Some("CONNECTED".to_string()),
            player_state: Some("PLAYING".to_string()),
            position_seconds: None,
            duration_seconds: None,
            message: None,
            playback_token: Some(request.playback_token.clone()),
            cast_session_id: Some(request.cast_session_id.clone()),
            media_session_id: Some(1),
            idle_reason: None,
        });
    }

    fn emit_load_failed(&self, playback_token: &str, message: &str) {
        self.clear_grant();
        self.clear_snapshot();
        if let Ok(mut snapshot) = self.0.snapshot.lock() {
            snapshot.last_error = Some(clean_text(message, 240));
            snapshot.updated_at_epoch_ms = now_ms();
        }
        self.emit(CastEventReport {
            kind: "load_failed".to_string(),
            device_name: None,
            cast_state: None,
            session_state: None,
            player_state: None,
            position_seconds: None,
            duration_seconds: None,
            message: Some(clean_text(message, 240)),
            playback_token: Some(playback_token.to_string()),
            cast_session_id: None,
            media_session_id: None,
            idle_reason: None,
        });
    }

    fn stop_internal(&self, _suppress_completion: bool) -> Result<(), String> {
        let active = self.active_report();
        // Invalidate first: a poller blocked in SOAP must not wake up and
        // report against a session that no longer exists.
        let _ = self.invalidate();
        self.clear_grant();
        if let Some(active) = active {
            let _ = soap(
                &active.device,
                AVTRANSPORT_SERVICE,
                "Stop",
                "<InstanceID>0</InstanceID>",
            );
        }
        self.clear_snapshot();
        Ok(())
    }

    fn spawn_poller(&self, generation: u64) {
        // One poller per load. The generation is published with the new session
        // before this poller starts, so a slow SOAP poll from the previous item
        // exits in the stop/play handoff instead of reporting against this one.
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            // An early first poll catches very short clips; a premature
            // STOPPED is still ignored until real PLAYING state is observed.
            let mut first_poll = true;
            loop {
                let delay = if first_poll {
                    Duration::from_millis(350)
                } else {
                    POLL_INTERVAL
                };
                first_poll = false;
                tokio::time::sleep(delay).await;
                if manager.0.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let Some(active) = manager.active_report() else {
                    return;
                };
                if active.generation != generation {
                    return;
                }
                let poll_device = active.device.clone();
                let result = tokio::task::spawn_blocking(move || poll_once(&poll_device)).await;
                if manager.0.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                let Ok(Ok(result)) = result else { continue };
                let mut should_complete = false;
                let mut completed_generation: Option<u64> = None;
                {
                    let Ok(mut guard) = manager.0.active.lock() else {
                        return;
                    };
                    let Some(state) = guard.as_mut() else {
                        return;
                    };
                    if state.generation != generation
                        || state.cast_session_id != active.cast_session_id
                    {
                        return;
                    }
                    if result.player_state == "PLAYING" {
                        state.saw_playing = true;
                    }
                    if result.finished && state.saw_playing && !state.suppress_completion {
                        completed_generation =
                            Some(manager.0.generation.fetch_add(1, Ordering::SeqCst) + 1);
                        *guard = None;
                        should_complete = true;
                    }
                    if let Ok(mut snapshot) = manager.0.snapshot.lock() {
                        snapshot.position_seconds = result.position_seconds;
                        snapshot.duration_seconds = result.duration_seconds;
                        snapshot.player_state = Some(result.player_state.clone());
                        snapshot.volume_level = result.volume_level;
                        snapshot.muted = result.muted;
                        snapshot.updated_at_epoch_ms = now_ms();
                    }
                }
                if !should_complete && manager.0.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                manager.emit(CastEventReport {
                    kind: "media_status".to_string(),
                    device_name: Some(active.device.name.clone()),
                    cast_state: Some("CONNECTED".to_string()),
                    session_state: Some("CONNECTED".to_string()),
                    player_state: Some(result.player_state),
                    position_seconds: result.position_seconds,
                    duration_seconds: result.duration_seconds,
                    message: None,
                    playback_token: Some(active.playback_token.clone()),
                    cast_session_id: Some(active.cast_session_id.clone()),
                    media_session_id: Some(active.media_session_id),
                    idle_reason: None,
                });
                if should_complete {
                    manager.emit(CastEventReport {
                        kind: "media_completed".to_string(),
                        device_name: Some(active.device.name.clone()),
                        cast_state: Some("CONNECTED".to_string()),
                        session_state: Some("CONNECTED".to_string()),
                        player_state: Some("STOPPED".to_string()),
                        position_seconds: result.position_seconds,
                        duration_seconds: result.duration_seconds,
                        message: Some("Video completed.".to_string()),
                        playback_token: Some(active.playback_token.clone()),
                        cast_session_id: Some(active.cast_session_id),
                        media_session_id: Some(active.media_session_id),
                        idle_reason: Some("FINISHED".to_string()),
                    });
                    if completed_generation
                        .is_some_and(|value| manager.0.generation.load(Ordering::SeqCst) == value)
                    {
                        manager.clear_snapshot();
                    }
                    return;
                }
            }
        });
    }
}

fn poll_once(device: &CastDevice) -> Result<PollResult, String> {
    let transport = soap(
        device,
        AVTRANSPORT_SERVICE,
        "GetTransportInfo",
        "<InstanceID>0</InstanceID>",
    )?;
    let current_state = element_text(&transport, "CurrentTransportState")
        .or_else(|| element_text(&transport, "TransportState"))
        .unwrap_or_default();
    let player_state = match current_state.as_str() {
        "PLAYING" => "PLAYING",
        "PAUSED_PLAYBACK" | "PAUSED" => "PAUSED",
        "STOPPED" | "NO_MEDIA_PRESENT" => "STOPPED",
        "TRANSITIONING" => "BUFFERING",
        _ => "IDLE",
    };
    let position = soap(
        device,
        AVTRANSPORT_SERVICE,
        "GetPositionInfo",
        "<InstanceID>0</InstanceID>",
    )
    .ok();
    let position_seconds = position
        .as_deref()
        .and_then(|xml| element_text(xml, "RelTime"))
        .and_then(|value| seconds_from_clock(&value));
    let duration_seconds = position
        .as_deref()
        .and_then(|xml| element_text(xml, "TrackDuration"))
        .and_then(|value| seconds_from_clock(&value));
    let volume = soap(
        device,
        RENDERING_SERVICE,
        "GetVolume",
        "<InstanceID>0</InstanceID><Channel>Master</Channel>",
    )
    .ok()
    .and_then(|xml| element_text(&xml, "CurrentVolume"))
    .and_then(|value| value.parse::<f64>().ok())
    .map(|value| (value / 100.0).clamp(0.0, 1.0));
    let muted = soap(
        device,
        RENDERING_SERVICE,
        "GetMute",
        "<InstanceID>0</InstanceID><Channel>Master</Channel>",
    )
    .ok()
    .and_then(|xml| element_text(&xml, "CurrentMute"))
    .and_then(|value| value.parse::<u8>().ok())
    .map(|value| value == 1);
    Ok(PollResult {
        position_seconds,
        duration_seconds,
        player_state: player_state.to_string(),
        volume_level: volume,
        muted,
        finished: matches!(player_state, "STOPPED"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssdp_location_only_from_200_responses() {
        let ok = "HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.50:8080/desc.xml\r\nST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n\r\n";
        assert_eq!(
            ssdp_location(ok).as_deref(),
            Some("http://192.168.1.50:8080/desc.xml")
        );
        assert_eq!(
            ssdp_location("HTTP/1.1 404 Not Found\r\nLOCATION: http://192.168.1.50/x\r\n\r\n"),
            None
        );
    }

    #[test]
    fn parses_clock_values_and_formats_seeks() {
        assert_eq!(seconds_from_clock("01:02:03"), Some(3723.0));
        assert_eq!(seconds_from_clock("00:00:04.500"), Some(4.5));
        assert_eq!(clock_from_seconds(3723.0), "01:02:03");
    }

    #[test]
    fn extracts_namespaced_upnp_elements() {
        let xml = "<s:Envelope><s:Body><u:GetTransportInfoResponse><CurrentTransportState>PLAYING</CurrentTransportState></u:GetTransportInfoResponse></s:Body></s:Envelope>";
        assert_eq!(
            element_text(xml, "CurrentTransportState").as_deref(),
            Some("PLAYING")
        );
    }

    #[test]
    fn xml_escaping_protects_soap_arguments() {
        assert_eq!(escape_xml("a&b<c>\"d\""), "a&amp;b&lt;c&gt;&quot;d&quot;");
        assert_eq!(unescape_xml("a&amp;b&lt;c&gt;"), "a&b<c>");
    }
}
