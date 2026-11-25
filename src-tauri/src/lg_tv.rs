//! Local LG webOS control over SSAP.
//!
//! Video playback does not go through SSAP: modern webOS firmware hands
//! `media.viewer/open` to an application that no longer ships. RollaPlay uses
//! DLNA/UPnP AVTransport for playback and this module only adds TV-native
//! controls: pairing, power, volume, media keys, inputs and the browser URL
//! fallback. Everything stays on the LAN; there is no account or cloud call.
//!
//! The pairing key is a bearer credential. It is stored with the same DPAPI
//! protection as the media-server token, is never logged, and is only sent to
//! the TV it was issued for.

use crate::{
    dlna::http_exchange,
    network::{is_private_lan_ipv4, select_lan_address},
    secret,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use native_tls::{Certificate, TlsConnector, TlsStream};
use rand::{RngCore, rngs::OsRng};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream, UdpSocket},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

const SSDP_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const SSDP_PORT: u16 = 1900;
const LG_SECOND_SCREEN: &str = "urn:lge-com:service:webos-second-screen:1";
const MAX_PACKET: usize = 8192;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const PAIR_TIMEOUT: Duration = Duration::from_secs(75);
const MAX_FRAME: usize = 64 * 1024;
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// webOS TVs use a certificate issued by LG's private CA. These are the public
// SHA-256 fingerprints of LG's static TV certificate and its issuer; any other
// certificate is rejected instead of silently trusting a rogue local device.
const LG_CERT_FINGERPRINTS: [&str; 2] = [
    "11:C5:B1:C5:90:77:50:AB:B9:DA:2A:66:65:CC:CE:2B:B2:88:A5:83:F4:5A:33:39:E7:1F:87:BF:2F:80:85:52",
    "E2:BD:64:64:D3:F5:1C:1B:95:B7:69:7D:9D:67:73:C3:3D:94:12:EB:A0:29:9C:56:8C:34:93:7D:3F:E6:8A:A0",
];

const PAIRING_PERMISSIONS: [&str; 13] = [
    "CONTROL_AUDIO",
    "CONTROL_INPUT_MEDIA_PLAYBACK",
    "CONTROL_POWER",
    "READ_POWER_STATE",
    "LAUNCH",
    "READ_APP_STATUS",
    "READ_CURRENT_CHANNEL",
    "READ_INPUT_DEVICE_LIST",
    "READ_NETWORK_STATE",
    "READ_RUNNING_APPS",
    "READ_TV_CHANNEL_LIST",
    "WRITE_NOTIFICATION_TOAST",
    "CONTROL_TV_SCREEN",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LgTvDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub address: String,
    pub port: u16,
    pub secure: bool,
    pub paired: bool,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct LgTvStatus {
    pub selected: Option<LgTvDevice>,
    pub paired: bool,
    pub power_state: Option<String>,
    pub volume: Option<f64>,
    pub muted: Option<bool>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LgTvPairResult {
    pub status: String,
    pub device: LgTvDevice,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LgTvInput {
    pub id: String,
    pub label: String,
    pub connected: bool,
}

struct LgTvManagerInner {
    database: Arc<Mutex<Connection>>,
    selected: Mutex<Option<LgTvDevice>>,
    status: Mutex<LgTvStatus>,
}

#[derive(Clone)]
pub struct LgTvManager(Arc<LgTvManagerInner>);

impl LgTvManager {
    pub fn new(database: Arc<Mutex<Connection>>) -> Self {
        Self(Arc::new(LgTvManagerInner {
            database,
            selected: Mutex::new(None),
            status: Mutex::new(LgTvStatus::default()),
        }))
    }

    pub fn selected(&self) -> Option<LgTvDevice> {
        self.0.selected.lock().ok().and_then(|guard| guard.clone())
    }

    pub fn status_snapshot(&self) -> LgTvStatus {
        self.0
            .status
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn has_key(&self, address: &str) -> bool {
        match self.0.database.lock() {
            Ok(connection) => secret::load_lg_client_key(&connection, address)
                .ok()
                .flatten()
                .is_some(),
            Err(_) => false,
        }
    }

    fn save_key(&self, address: &str, key: &str) -> Result<(), String> {
        let connection = self
            .0
            .database
            .lock()
            .map_err(|_| "The local settings database is unavailable".to_string())?;
        secret::save_lg_client_key(&connection, address, key)
    }

    fn load_key(&self, address: &str) -> Result<Option<String>, String> {
        let connection = self
            .0
            .database
            .lock()
            .map_err(|_| "The local settings database is unavailable".to_string())?;
        secret::load_lg_client_key(&connection, address)
    }

    /// A TV may rotate its pairing key during any registration. Persist it
    /// immediately so later controls keep working; never surface the value.
    fn remember_rotated_key(
        &self,
        device: &LgTvDevice,
        rotated: Option<String>,
    ) -> Result<(), String> {
        if let Some(key) = rotated {
            self.save_key(&device.address, &key)?;
        }
        Ok(())
    }

    fn sync_selected_key(&self) -> Option<LgTvDevice> {
        let Some(mut device) = self.selected() else {
            return None;
        };
        device.paired = self.has_key(&device.address);
        if let Ok(mut selected) = self.0.selected.lock() {
            *selected = Some(device.clone());
        }
        if let Ok(mut status) = self.0.status.lock() {
            status.selected = Some(device.clone());
            status.paired = device.paired;
        }
        Some(device)
    }

    pub fn select(&self, device: LgTvDevice) -> Result<LgTvStatus, String> {
        validate_device(&device)?;
        let mut stored = device.clone();
        stored.paired = self.has_key(&device.address);
        *self
            .0
            .selected
            .lock()
            .map_err(|_| "LG TV state is unavailable".to_string())? = Some(stored.clone());
        *self
            .0
            .status
            .lock()
            .map_err(|_| "LG TV state is unavailable".to_string())? = LgTvStatus {
            selected: Some(stored),
            paired: self.has_key(&device.address),
            ..LgTvStatus::default()
        };
        Ok(self.status_snapshot())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        let had_device = self.selected().is_some();
        *self
            .0
            .selected
            .lock()
            .map_err(|_| "LG TV state is unavailable".to_string())? = None;
        *self
            .0
            .status
            .lock()
            .map_err(|_| "LG TV state is unavailable".to_string())? = LgTvStatus::default();
        let _ = had_device;
        Ok(())
    }

    /// Opens a local SSAP session and starts/continues pairing. This is
    /// intentionally blocking; the Tauri command runs it on a blocking thread
    /// so the UI can show the pending TV confirmation.
    pub fn pair(&self) -> Result<LgTvPairResult, String> {
        let device = self
            .sync_selected_key()
            .ok_or_else(|| "Choose an LG TV first".to_string())?;
        let key = self.load_key(&device.address)?;
        let (mut socket, _secure) = connect_to_device(&device)?;
        socket.stream.set_read_timeout(Some(PAIR_TIMEOUT));
        socket.stream.set_write_timeout(Some(REQUEST_TIMEOUT));
        let new_key = register(&mut socket, key.as_deref())?;
        let message = if let Some(key) = new_key {
            self.save_key(&device.address, &key)?;
            let mut paired = device.clone();
            paired.paired = true;
            *self
                .0
                .selected
                .lock()
                .map_err(|_| "LG TV state is unavailable".to_string())? = Some(paired.clone());
            if let Ok(mut status) = self.0.status.lock() {
                status.selected = Some(paired.clone());
                status.paired = true;
                status.last_error = None;
            }
            LgTvPairResult {
                status: "paired".to_string(),
                device: paired,
                message: "LG TV connected. The pairing key is stored for this Windows user only."
                    .to_string(),
            }
        } else {
            LgTvPairResult {
                status: "prompt".to_string(),
                device: device.clone(),
                message: "Accept the connection prompt on the TV to finish pairing.".to_string(),
            }
        };
        Ok(message)
    }

    pub fn refresh(&self) -> Result<LgTvStatus, String> {
        let device = self
            .sync_selected_key()
            .ok_or_else(|| "Choose an LG TV first".to_string())?;
        let key = self
            .load_key(&device.address)?
            .ok_or_else(|| "Pair with this LG TV first".to_string())?;
        let (mut socket, _secure) = connect_to_device(&device)?;
        let registration = register(&mut socket, Some(&key))?;
        if registration.is_some() {
            return Err("The TV issued a new pairing key. Pair again to save it.".to_string());
        }
        let mut status = self.status_snapshot();
        if let Ok(power) = request(
            &mut socket,
            "request",
            Some("ssap://com.webos.service.tvpower/power/getPowerState"),
            json!({}),
        ) {
            if let Some(state) =
                power
                    .pointer("/payload/state")
                    .and_then(Value::as_str)
                    .map(|raw| match raw {
                        "Active" => "on".to_string(),
                        "Active Standby" => "standby".to_string(),
                        "Screen Off" => "screen_off".to_string(),
                        "Screen Saver" => "screen_saver".to_string(),
                        "Suspend" | "Power Off" => "off".to_string(),
                        other => other.to_string(),
                    })
            {
                status.power_state = Some(state);
            }
        }
        if let Ok(volume) = request(
            &mut socket,
            "request",
            Some("ssap://audio/getVolume"),
            json!({}),
        ) {
            let payload = volume.get("payload");
            let level = payload
                .and_then(|value| value.get("volume"))
                .and_then(Value::as_f64)
                .or_else(|| {
                    payload
                        .and_then(|value| value.get("volumeStatus"))
                        .and_then(|value| value.get("volume"))
                        .and_then(Value::as_f64)
                });
            if let Some(level) = level.filter(|value| value.is_finite() && *value >= 0.0) {
                status.volume = Some(level.clamp(0.0, 100.0));
            }
            let muted = payload
                .and_then(|value| value.get("muted"))
                .and_then(Value::as_bool)
                .or_else(|| {
                    payload
                        .and_then(|value| value.get("volumeStatus"))
                        .and_then(|value| value.get("muteStatus"))
                        .and_then(Value::as_bool)
                });
            if let Some(muted) = muted {
                status.muted = Some(muted);
            }
        }
        status.paired = true;
        status.last_error = None;
        *self
            .0
            .status
            .lock()
            .map_err(|_| "LG TV state is unavailable".to_string())? = status.clone();
        Ok(status)
    }

    pub fn command(&self, action: &str, value: Option<f64>) -> Result<LgTvStatus, String> {
        let device = self
            .sync_selected_key()
            .ok_or_else(|| "Choose an LG TV first".to_string())?;
        let key = self
            .load_key(&device.address)?
            .ok_or_else(|| "Pair with this LG TV first".to_string())?;
        let (uri, payload) = command_payload(action, value)?;
        let (mut socket, _secure) = connect_to_device(&device)?;
        self.remember_rotated_key(&device, register(&mut socket, Some(&key))?)?;
        let response = request(&mut socket, "request", Some(uri), payload)?;
        ensure_success(&response)?;
        self.refresh()
    }

    pub fn inputs(&self) -> Result<Vec<LgTvInput>, String> {
        let device = self
            .sync_selected_key()
            .ok_or_else(|| "Choose an LG TV first".to_string())?;
        let key = self
            .load_key(&device.address)?
            .ok_or_else(|| "Pair with this LG TV first".to_string())?;
        let (mut socket, _secure) = connect_to_device(&device)?;
        self.remember_rotated_key(&device, register(&mut socket, Some(&key))?)?;
        let response = request(
            &mut socket,
            "request",
            Some("ssap://tv/getExternalInputList"),
            json!({}),
        )?;
        ensure_success(&response)?;
        let mut inputs = Vec::new();
        if let Some(items) = response
            .pointer("/payload/devices")
            .and_then(Value::as_array)
        {
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let label = item
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string();
                let connected = item
                    .get("connected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                inputs.push(LgTvInput {
                    id: id.to_string(),
                    label,
                    connected,
                });
            }
        }
        inputs.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(inputs)
    }

    pub fn switch_input(&self, input_id: &str) -> Result<LgTvStatus, String> {
        let device = self
            .sync_selected_key()
            .ok_or_else(|| "Choose an LG TV first".to_string())?;
        let key = self
            .load_key(&device.address)?
            .ok_or_else(|| "Pair with this LG TV first".to_string())?;
        if input_id.is_empty() || input_id.len() > 120 || input_id.contains('\n') {
            return Err("That TV input is not valid".to_string());
        }
        let (mut socket, _secure) = connect_to_device(&device)?;
        self.remember_rotated_key(&device, register(&mut socket, Some(&key))?)?;
        let response = request(
            &mut socket,
            "request",
            Some("ssap://tv/switchInput"),
            json!({ "inputId": input_id }),
        )?;
        ensure_success(&response)?;
        self.refresh()
    }

    /// Opens a plain local URL in the TV browser. This is a fallback only:
    /// video playback stays on DLNA, and no token-bearing URL is accepted.
    pub fn open_browser(&self, target: &str) -> Result<(), String> {
        validate_browser_target(target)?;
        let device = self
            .sync_selected_key()
            .ok_or_else(|| "Choose an LG TV first".to_string())?;
        let key = self
            .load_key(&device.address)?
            .ok_or_else(|| "Pair with this LG TV first".to_string())?;
        let (mut socket, _secure) = connect_to_device(&device)?;
        self.remember_rotated_key(&device, register(&mut socket, Some(&key))?)?;
        let response = request(
            &mut socket,
            "request",
            Some("ssap://system.launcher/open"),
            json!({ "target": target }),
        )?;
        ensure_success(&response)
    }
}

fn validate_device(device: &LgTvDevice) -> Result<(), String> {
    let address = device
        .address
        .parse::<Ipv4Addr>()
        .map_err(|_| "The LG TV has an invalid LAN address".to_string())?;
    if !is_private_lan_ipv4(address) {
        return Err("The LG TV is outside the local network".to_string());
    }
    if ![3000, 3001].contains(&device.port) {
        return Err("The LG TV uses an unexpected control port".to_string());
    }
    Ok(())
}

fn validate_browser_target(target: &str) -> Result<(), String> {
    if target.len() > 512 || target.contains('?') || target.contains('#') || target.contains('\n') {
        return Err("Only a token-free local URL can be opened in the TV browser".to_string());
    }
    let parsed = url::Url::parse(target).map_err(|_| "That URL is not valid".to_string())?;
    if parsed.scheme() != "http" {
        return Err("Only a local HTTP URL can be opened in the TV browser".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "That URL has no host".to_string())?;
    let ip = host
        .parse::<Ipv4Addr>()
        .map_err(|_| "Only a local IPv4 URL can be opened in the TV browser".to_string())?;
    if !is_private_lan_ipv4(ip) {
        return Err("Only a local URL can be opened in the TV browser".to_string());
    }
    Ok(())
}

fn command_payload(action: &str, value: Option<f64>) -> Result<(&'static str, Value), String> {
    match action {
        "power_on" => Ok(("ssap://system/turnOn", json!({}))),
        "power_off" => Ok(("ssap://system/turnOff", json!({}))),
        "play" => Ok(("ssap://media.controls/play", json!({}))),
        "pause" => Ok(("ssap://media.controls/pause", json!({}))),
        "stop" => Ok(("ssap://media.controls/stop", json!({}))),
        "volume_up" => Ok(("ssap://audio/volumeUp", json!({}))),
        "volume_down" => Ok(("ssap://audio/volumeDown", json!({}))),
        "set_volume" => {
            let Some(level) = value.filter(|level| level.is_finite()) else {
                return Err("A volume level is required".to_string());
            };
            let level = if level <= 1.0 { level * 100.0 } else { level }.clamp(0.0, 100.0) as u32;
            Ok(("ssap://audio/setVolume", json!({ "volume": level })))
        }
        "set_mute" => {
            let Some(muted) = value.filter(|value| value.is_finite()) else {
                return Err("A mute value is required".to_string());
            };
            Ok(("ssap://audio/setMute", json!({ "mute": muted != 0.0 })))
        }
        _ => Err("That LG control is not available".to_string()),
    }
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

fn error_from_response(response: &Value) -> Option<String> {
    let message = response
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            response
                .get("payload")
                .and_then(|payload| payload.get("errorText"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            response
                .get("payload")
                .and_then(|payload| payload.get("returnValue"))
                .and_then(Value::as_bool)
                .filter(|ok| !*ok)
                .map(|_| "The TV rejected the request".to_string())
        });
    message.map(|value| clean_text(&value, 240))
}

fn ensure_success(response: &Value) -> Result<(), String> {
    if response.get("type").and_then(Value::as_str) == Some("error") {
        return Err(error_from_response(response)
            .unwrap_or_else(|| "The TV rejected the request".to_string()));
    }
    if let Some(message) = error_from_response(response) {
        return Err(message);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Minimal WebSocket client (RFC 6455 text frames) over local TCP or LG's
// private-CA TLS certificate. No third-party websocket dependency is needed.
// ---------------------------------------------------------------------------

enum SsapStream {
    Plain(TcpStream),
    Secure(TlsStream<TcpStream>),
}

impl Read for SsapStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Secure(stream) => stream.read(buffer),
        }
    }
}

impl Write for SsapStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Secure(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Secure(stream) => stream.flush(),
        }
    }
}

impl SsapStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) {
        match self {
            Self::Plain(stream) => {
                let _ = stream.set_read_timeout(timeout);
            }
            Self::Secure(stream) => {
                let _ = stream.get_ref().set_read_timeout(timeout);
            }
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) {
        match self {
            Self::Plain(stream) => {
                let _ = stream.set_write_timeout(timeout);
            }
            Self::Secure(stream) => {
                let _ = stream.get_ref().set_write_timeout(timeout);
            }
        }
    }
}

struct SsapSocket {
    stream: SsapStream,
}

fn certificate_fingerprint(certificate: &Certificate) -> Result<String, String> {
    let der = certificate
        .to_der()
        .map_err(|_| "The LG TV certificate could not be read".to_string())?;
    let digest = Sha256::digest(der);
    Ok(digest
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":"))
}

fn connect_once(address: &str, port: u16, secure: bool) -> Result<SsapSocket, String> {
    let ip = address
        .parse::<Ipv4Addr>()
        .map_err(|_| "The LG TV has an invalid LAN address".to_string())?;
    let tcp = TcpStream::connect_timeout(&std::net::SocketAddr::from((ip, port)), CONNECT_TIMEOUT)
        .map_err(|_| "The LG TV refused a local control connection".to_string())?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(CONNECT_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(CONNECT_TIMEOUT));

    let mut stream = if secure {
        let connector = TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()
            .map_err(|_| "Local TLS could not be initialized".to_string())?;
        let tls = connector
            .connect(address, tcp)
            .map_err(|_| "The LG TV did not complete a local TLS handshake".to_string())?;
        let certificate = tls
            .peer_certificate()
            .map_err(|_| "The LG TV certificate could not be read".to_string())?
            .ok_or_else(|| "The LG TV did not present a certificate".to_string())?;
        let fingerprint = certificate_fingerprint(&certificate)?;
        if !LG_CERT_FINGERPRINTS.contains(&fingerprint.as_str()) {
            return Err("The LG TV presented an unrecognized certificate".to_string());
        }
        SsapStream::Secure(tls)
    } else {
        SsapStream::Plain(tcp)
    };

    let mut key_bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut key_bytes);
    let key = STANDARD.encode(key_bytes);
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {address}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nOrigin: http://{address}:{port}\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| "The LG TV closed the control handshake".to_string())?;
    stream
        .flush()
        .map_err(|_| "The LG TV closed the control handshake".to_string())?;
    let (header, _leftover) = read_headers(&mut stream)?;
    if !header.starts_with("HTTP/1.1 101") && !header.starts_with("HTTP/1.0 101") {
        return Err("The LG TV did not accept the control handshake".to_string());
    }
    let expected_accept = {
        let mut sha1 = Sha1::new();
        sha1.update(key.as_bytes());
        sha1.update(WS_GUID.as_bytes());
        STANDARD.encode(sha1.finalize())
    };
    let accepted = header.lines().any(|line| {
        line.split_once(':')
            .map(|(name, value)| {
                name.eq_ignore_ascii_case("sec-websocket-accept") && value.trim() == expected_accept
            })
            .unwrap_or(false)
    });
    if !accepted {
        return Err("The LG TV returned an invalid control handshake".to_string());
    }
    Ok(SsapSocket { stream })
}

fn read_headers(stream: &mut SsapStream) -> Result<(String, Vec<u8>), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|_| "The LG TV closed the control handshake".to_string())?;
        if read == 0 {
            return Err("The LG TV closed the control handshake".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > 16 * 1024 {
            return Err("The LG TV returned an oversized control handshake".to_string());
        }
    };
    let header = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let leftover = buffer[header_end..].to_vec();
    Ok((header, leftover))
}

fn connect_to_device(device: &LgTvDevice) -> Result<(SsapSocket, bool), String> {
    let mut candidates = vec![(device.port, device.secure)];
    let alternate = if device.secure {
        (3000_u16, false)
    } else {
        (3001_u16, true)
    };
    if !candidates.contains(&alternate) {
        candidates.push(alternate);
    }
    let mut errors = Vec::new();
    for (port, secure) in candidates {
        match connect_once(&device.address, port, secure) {
            Ok(socket) => return Ok((socket, secure)),
            Err(error) => errors.push(error),
        }
    }
    let _ = errors;
    Err("The LG TV did not accept a local control connection. Check that webOS network control is enabled.".to_string())
}

fn send_frame<W: Write>(stream: &mut W, opcode: u8, payload: &[u8]) -> Result<(), String> {
    if payload.len() > MAX_FRAME {
        return Err("The LG control message is too large".to_string());
    }
    let mut frame = vec![0x80 | opcode];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mut mask = [0_u8; 4];
    OsRng.fill_bytes(&mut mask);
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream
        .write_all(&frame)
        .map_err(|_| "The LG TV closed the control connection".to_string())?;
    stream
        .flush()
        .map_err(|_| "The LG TV closed the control connection".to_string())?;
    Ok(())
}

fn read_exact<R: Read>(stream: &mut R, length: usize) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0_u8; length];
    stream
        .read_exact(&mut buffer)
        .map_err(|_| "The LG TV stopped answering control requests".to_string())?;
    Ok(buffer)
}

fn read_frame<R: Read>(stream: &mut R) -> Result<(u8, Vec<u8>, bool), String> {
    let header = read_exact(stream, 2)?;
    let fin = header[0] & 0x80 != 0;
    let opcode = header[0] & 0x0f;
    if header[1] & 0x80 != 0 {
        return Err("The LG TV sent a masked control frame".to_string());
    }
    let length = match header[1] & 0x7f {
        126 => {
            let bytes = read_exact(stream, 2)?;
            u16::from_be_bytes([bytes[0], bytes[1]]) as usize
        }
        127 => {
            let bytes = read_exact(stream, 8)?;
            let value = u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| "Invalid control frame".to_string())?,
            );
            usize::try_from(value).map_err(|_| "The LG control frame is too large".to_string())?
        }
        value => value as usize,
    };
    if length > MAX_FRAME {
        return Err("The LG control frame is too large".to_string());
    }
    let payload = if length == 0 {
        Vec::new()
    } else {
        read_exact(stream, length)?
    };
    Ok((opcode, payload, fin))
}

fn read_response<S: Read + Write>(stream: &mut S) -> Result<Value, String> {
    let mut assembled = Vec::new();
    loop {
        let (opcode, payload, fin) = read_frame(stream)?;
        match opcode {
            0x9 => {
                send_frame(stream, 0xA, &payload)?;
                continue;
            }
            0xA => continue,
            0x8 => return Err("The LG TV closed the control session".to_string()),
            0x0 | 0x1 => {
                assembled.extend_from_slice(&payload);
                if assembled.len() > MAX_FRAME {
                    return Err("The LG control message is too large".to_string());
                }
                if !fin {
                    continue;
                }
                return serde_json::from_slice(&assembled)
                    .map_err(|_| "The LG TV returned invalid control data".to_string());
            }
            _ => return Err("The LG TV returned an unsupported control frame".to_string()),
        }
    }
}

fn request(
    socket: &mut SsapSocket,
    kind: &str,
    uri: Option<&str>,
    payload: Value,
) -> Result<Value, String> {
    let id = format!("rollaplay-{}", Uuid::new_v4());
    let mut message = json!({ "id": id, "type": kind, "payload": payload });
    if let Some(uri) = uri {
        message["uri"] = json!(uri);
    }
    let text = serde_json::to_string(&message)
        .map_err(|_| "The LG control message could not be encoded".to_string())?;
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    socket.stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    send_frame(&mut socket.stream, 0x1, text.as_bytes())?;
    loop {
        if Instant::now() > deadline {
            return Err("The LG TV did not answer the control request".to_string());
        }
        let response = read_response(&mut socket.stream)?;
        if response.get("id").and_then(Value::as_str) == Some(id.as_str()) {
            return Ok(response);
        }
    }
}

fn register(socket: &mut SsapSocket, key: Option<&str>) -> Result<Option<String>, String> {
    let mut payload = json!({
        "forcePairing": false,
        "pairingType": "PROMPT",
        "manifest": {
            "manifestVersion": 1,
            "appVersion": "1.0",
            "permissions": PAIRING_PERMISSIONS,
        },
    });
    if let Some(key) = key {
        payload["client-key"] = json!(key);
    }
    let response = request(socket, "register", None, payload)?;
    if response.get("type").and_then(Value::as_str) == Some("error") {
        let message = error_from_response(&response)
            .unwrap_or_else(|| "The TV rejected the pairing request".to_string());
        if message.to_ascii_lowercase().contains("blacklist") {
            return Err("This webOS firmware rejected RollaPlay’s pairing manifest".to_string());
        }
        return Err(message);
    }
    Ok(response
        .get("payload")
        .and_then(|payload| payload.get("client-key"))
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .map(str::to_string))
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn ssdp_header(packet: &str, name: &str) -> Option<String> {
    packet.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case(name) {
            let value = clean_text(value.trim(), 512);
            if !value.is_empty() {
                return Some(value);
            }
        }
        None
    })
}

fn ports_from_usn(usn: &str) -> (u16, bool) {
    let digits: Vec<u16> = usn
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u16>().ok())
        .collect();
    if digits.len() >= 2 {
        let last = digits[digits.len() - 1];
        let previous = digits[digits.len() - 2];
        if [3000, 3001].contains(&last) && [3000, 3001].contains(&previous) {
            return (last, last == 3001);
        }
    }
    if digits.last() == Some(&3000) {
        return (3000, false);
    }
    (3001, true)
}

fn device_from_ssdp(packet: &str, source: Ipv4Addr) -> Option<LgTvDevice> {
    let location = ssdp_header(packet, "location")?;
    let parsed = url::Url::parse(&location).ok()?;
    let host = parsed.host_str()?;
    let address = host.parse::<Ipv4Addr>().unwrap_or(source);
    if !is_private_lan_ipv4(address) {
        return None;
    }
    let usn = ssdp_header(packet, "usn").unwrap_or_else(|| location.clone());
    let (port, secure) = ports_from_usn(&usn);
    let (status, description) =
        http_exchange("GET", &location, &[], None).unwrap_or((0, String::new()));
    let name = if status == 200 {
        crate::dlna::element_text(&description, "friendlyName")
            .map(|value| clean_text(&value, 120))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "LG webOS TV".to_string())
    } else {
        ssdp_header(packet, "server")
            .map(|value| clean_text(&value, 120))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "LG webOS TV".to_string())
    };
    let model = if status == 200 {
        crate::dlna::element_text(&description, "modelName")
            .map(|value| clean_text(&value, 80))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "webOS".to_string())
    } else {
        "webOS".to_string()
    };
    Some(LgTvDevice {
        id: format!("lge:{usn}"),
        name,
        model,
        address: address.to_string(),
        port,
        secure,
        paired: false,
    })
}

/// Real SSDP discovery for LG webOS second-screen TVs. Runs on a blocking
/// thread from the command layer, like mDNS Cast discovery.
pub fn discover(timeout: Duration) -> Result<Vec<LgTvDevice>, String> {
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
        return Err("Could not open a local socket for LG TV discovery".to_string());
    }
    let query = format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_GROUP}:{SSDP_PORT}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {LG_SECOND_SCREEN}\r\n\r\n"
    );
    for socket in &sockets {
        let _ = socket.send_to(query.as_bytes(), (SSDP_GROUP, SSDP_PORT));
    }
    let deadline = Instant::now() + timeout;
    let mut buffer = [0_u8; MAX_PACKET];
    let mut devices: Vec<LgTvDevice> = Vec::new();
    while Instant::now() < deadline {
        for socket in &sockets {
            match socket.recv_from(&mut buffer) {
                Ok((length, source)) => {
                    let Ok(packet) = std::str::from_utf8(&buffer[..length]) else {
                        continue;
                    };
                    if !packet.starts_with("HTTP/1.1 200") && !packet.starts_with("HTTP/1.0 200") {
                        continue;
                    }
                    let Some(source_ip) = source.ip().to_string().parse::<Ipv4Addr>().ok() else {
                        continue;
                    };
                    if let Some(device) = device_from_ssdp(packet, source_ip) {
                        if !devices
                            .iter()
                            .any(|current| current.address == device.address)
                        {
                            devices.push(device);
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }
    devices.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_second_screen_usbn_ports() {
        assert_eq!(
            ports_from_usn("uuid:abc::urn:lge-com:serviceId:webos-second-screen-3000-3001"),
            (3001, true)
        );
        assert_eq!(
            ports_from_usn("uuid:abc::urn:lge-com:serviceId:webos-second-screen-3000"),
            (3000, false)
        );
        assert_eq!(ports_from_usn("uuid:abc"), (3001, true));
    }

    #[test]
    fn websocket_text_frame_round_trips() {
        let payload = br#"{"type":"response","id":"1","payload":{"returnValue":true}}"#;
        // The TV sends unmasked server frames; this client sends masked ones.
        let mut wire = vec![0x81];
        if payload.len() < 126 {
            wire.push(payload.len() as u8);
        } else {
            wire.push(126);
            wire.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        }
        wire.extend_from_slice(payload);
        let (opcode, decoded, fin) =
            read_frame(&mut Cursor::new(wire)).expect("frame should decode");
        assert_eq!(opcode, 0x1);
        assert_eq!(decoded, payload);
        assert!(fin);
    }

    #[test]
    fn websocket_accept_uses_rfc_sample() {
        let mut sha1 = Sha1::new();
        sha1.update(b"dGhlIHNhbXBsZSBub25jZQ==");
        sha1.update(WS_GUID.as_bytes());
        assert_eq!(
            STANDARD.encode(sha1.finalize()),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn accepts_only_token_free_private_browser_targets() {
        assert!(validate_browser_target("http://192.168.1.40:7777/").is_ok());
        assert!(
            validate_browser_target("http://192.168.1.40:7777/media/1?cast_token=secret").is_err()
        );
        assert!(validate_browser_target("https://192.168.1.40/").is_err());
        assert!(validate_browser_target("http://8.8.8.8/").is_err());
    }

    #[test]
    fn maps_controls_to_real_ssap_endpoints() {
        let (uri, payload) = command_payload("set_volume", Some(42.0)).unwrap();
        assert_eq!(uri, "ssap://audio/setVolume");
        assert_eq!(payload.get("volume").and_then(Value::as_u64), Some(42));
        assert!(command_payload("open_media", None).is_err());
    }
}
