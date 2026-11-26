//! Redacted, allow-listed network compatibility diagnostics.
//!
//! This module never receives credentials, URLs, file paths, raw device
//! responses, or user-supplied text. It turns a small set of booleans and
//! counters into fixed UI states and fixed guidance strings.

use crate::network::LinkQuality;
use serde::Serialize;
use std::{
    collections::VecDeque,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Ok,
    Warning,
    Failed,
    Unknown,
    NotRun,
}

#[derive(Serialize, Clone, Debug)]
pub struct CompatibilityCheck {
    pub state: CheckState,
    pub detail: String,
}

/// A fixed-code event. Both fields are allow-listed literals chosen by the
/// backend; no raw SDK text, URL, address, path, or error string is accepted.
#[derive(Serialize, Clone, Debug)]
pub struct DiagnosticEvent {
    pub code: String,
    pub detail: String,
    pub at_epoch_ms: u64,
}

const MAX_EVENTS: usize = 40;
static EVENTS: OnceLock<Mutex<VecDeque<DiagnosticEvent>>> = OnceLock::new();

fn event_log() -> &'static Mutex<VecDeque<DiagnosticEvent>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn record_event(code: &'static str, detail: &'static str) {
    let Ok(mut events) = event_log().lock() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    // Repeated one-second status posts must not fill the log with duplicates.
    if let Some(last) = events.back() {
        if last.code == code && now.saturating_sub(last.at_epoch_ms) < 5_000 {
            return;
        }
    }
    events.push_back(DiagnosticEvent {
        code: code.to_string(),
        detail: detail.to_string(),
        at_epoch_ms: now,
    });
    while events.len() > MAX_EVENTS {
        events.pop_front();
    }
}

pub fn diagnostic_events() -> Vec<DiagnosticEvent> {
    event_log()
        .lock()
        .map(|events| events.iter().cloned().collect())
        .unwrap_or_default()
}

#[derive(Serialize, Clone, Debug)]
pub struct NetworkCompatibilityDiagnostics {
    pub checked_at_epoch_ms: u64,
    pub transport: String,
    pub lan_adapter: CompatibilityCheck,
    pub local_server: CompatibilityCheck,
    pub receiver_discovery: CompatibilityCheck,
    pub receiver_network: CompatibilityCheck,
    pub receiver_match: CompatibilityCheck,
    pub receiver_control: CompatibilityCheck,
    pub media_url: CompatibilityCheck,
    pub receiver_media: CompatibilityCheck,
    pub link_quality: LinkQuality,
    pub stream_throughput: CompatibilityCheck,
    pub stream_stalls: CompatibilityCheck,
    pub playback: CompatibilityCheck,
    pub queue: CompatibilityCheck,
    pub media_format: CompatibilityCheck,
    pub guidance: Vec<String>,
    pub events: Vec<DiagnosticEvent>,
}

#[derive(Clone, Debug, Default)]
pub struct CompatibilityInputs {
    pub transport: String,
    pub lan_selected: bool,
    pub server_running: bool,
    pub local_probe_ok: bool,
    pub receiver_count: usize,
    pub same_subnet_receivers: usize,
    pub different_subnet_receivers: usize,
    pub receiver_control: Option<bool>,
    pub media_url_probe: Option<bool>,
    pub receiver_media_requests: usize,
    pub receiver_media_bytes: u64,
    pub receiver_media_seconds: u64,
    pub receiver_stalled_requests: usize,
    pub selected_receiver_matches_session: Option<bool>,
    pub link_quality: LinkQuality,
    pub playback_error: bool,
    pub playback_state: String,
    pub current_entry: bool,
    pub next_entry: bool,
    pub repeat_all: bool,
    pub media_format: Option<(String, String, bool)>,
    /// Fixed hint from a local header probe; no codec string is accepted here.
    pub av1_webm: bool,
}

fn check(state: CheckState, detail: impl Into<String>) -> CompatibilityCheck {
    CompatibilityCheck {
        state,
        detail: detail.into(),
    }
}

/// Byte counts come from the real local media server; only a coarse tier is
/// exposed so throughput cannot be confused with a raw address or URL.
fn throughput_tier(bytes: u64, seconds: u64) -> &'static str {
    if bytes == 0 {
        return "none";
    }
    let bits_per_second = bytes.saturating_mul(8) / seconds.max(1);
    match bits_per_second {
        8_000_000.. => "high",
        2_000_000.. => "moderate",
        _ => "low",
    }
}

fn stall_tier(stalled_requests: usize) -> &'static str {
    match stalled_requests {
        0 => "none",
        1..=2 => "occasional",
        _ => "frequent",
    }
}

/// Rebuilds link quality from allow-listed values only. A caller can never make
/// arbitrary text appear in the report, even if it supplies a malformed struct.
fn safe_link_quality(input: &LinkQuality) -> LinkQuality {
    let adapter_kind = match input.adapter_kind.as_str() {
        "wireless" => "wireless",
        "wired" => "wired",
        _ => "unknown",
    };
    LinkQuality {
        adapter_kind: adapter_kind.to_string(),
        band: match input.band.as_str() {
            "2_4_ghz" | "5_or_6_ghz" | "wired" => input.band.clone(),
            _ => "unknown".to_string(),
        },
        channel: input.channel.filter(|channel| (1..=233).contains(channel)),
        channel_width: match input.channel_width.as_str() {
            "unavailable" | "not_applicable" => input.channel_width.clone(),
            _ => "unavailable".to_string(),
        },
        signal: match input.signal.as_str() {
            "excellent" | "good" | "fair" | "weak" | "wired" => input.signal.clone(),
            _ => "unknown".to_string(),
        },
        link_speed: match input.link_speed.as_str() {
            "high" | "moderate" | "low" | "very_low" => input.link_speed.clone(),
            _ => "unknown".to_string(),
        },
    }
}

/// Probes only the receiver's advertised control port. The result is reduced to
/// a boolean; the address is never echoed back to the UI.
pub async fn probe_receiver_control(address: &str, port: u16) -> Option<bool> {
    if port == 0 {
        return None;
    }
    let ip = address.parse::<Ipv4Addr>().ok()?;
    let target = SocketAddr::V4(SocketAddrV4::new(ip, port));
    match tokio::time::timeout(
        Duration::from_millis(1500),
        tokio::net::TcpStream::connect(target),
    )
    .await
    {
        Ok(Ok(_)) => Some(true),
        Ok(Err(_)) | Err(_) => Some(false),
    }
}

pub fn build_diagnostics(
    input: &CompatibilityInputs,
    checked_at_epoch_ms: u64,
) -> NetworkCompatibilityDiagnostics {
    let link_quality = safe_link_quality(&input.link_quality);
    let lan_adapter = if input.lan_selected {
        check(
            CheckState::Ok,
            "A private LAN adapter is selected for streaming.",
        )
    } else {
        check(
            CheckState::Failed,
            "No active private IPv4 adapter was found.",
        )
    };
    let local_server = if !input.server_running {
        check(CheckState::Failed, "The LAN media server is not running.")
    } else if input.local_probe_ok {
        check(
            CheckState::Ok,
            "The PC can reach its own LAN media endpoint.",
        )
    } else {
        check(
            CheckState::Warning,
            "The LAN media endpoint did not answer a local probe.",
        )
    };
    let receiver_discovery = if input.receiver_count > 0 {
        check(
            CheckState::Ok,
            format!("{} receiver(s) answered discovery.", input.receiver_count),
        )
    } else {
        check(
            CheckState::Warning,
            "No receivers answered discovery. Power on the TV and check its network.",
        )
    };
    let receiver_network = if input.same_subnet_receivers > 0 {
        check(
            CheckState::Ok,
            "At least one receiver is on this PC's IPv4 subnet.",
        )
    } else if input.different_subnet_receivers > 0 {
        check(CheckState::Warning, "Receivers were found on another subnet. Guest Wi-Fi, VLANs, or AP isolation can block media.")
    } else {
        check(
            CheckState::Unknown,
            "Receiver subnet could not be determined.",
        )
    };
    let receiver_match = match input.selected_receiver_matches_session {
        Some(true) => check(
            CheckState::Ok,
            "The selected receiver matches the active playback session.",
        ),
        Some(false) => check(
            CheckState::Warning,
            "More than one receiver is available; the media result may belong to another device.",
        ),
        None => check(
            CheckState::Unknown,
            "No active receiver session was available to correlate.",
        ),
    };
    let receiver_control = match input.receiver_control {
        Some(true) => check(
            CheckState::Ok,
            "The receiver answered on its advertised control port.",
        ),
        Some(false) => check(
            CheckState::Failed,
            "The receiver did not answer on its advertised control port.",
        ),
        None => check(
            CheckState::Unknown,
            "No receiver was available for a control-port check.",
        ),
    };
    let media_url = match input.media_url_probe {
        Some(true) => check(
            CheckState::Ok,
            "A local ranged probe of the media URL passed.",
        ),
        Some(false) => check(
            CheckState::Failed,
            "A local ranged probe of the media URL failed.",
        ),
        None => check(CheckState::NotRun, "No media URL probe has run yet."),
    };
    let receiver_media = if input.receiver_media_requests > 0 && input.receiver_media_bytes > 0 {
        check(
            CheckState::Ok,
            format!(
                "{} request(s) from the receiver transferred video data.",
                input.receiver_media_requests
            ),
        )
    } else if input.receiver_media_requests > 0 {
        check(
            CheckState::Warning,
            "The receiver made a request but no video data was recorded.",
        )
    } else {
        check(
            CheckState::Unknown,
            "No media request from a receiver was recorded.",
        )
    };
    let throughput = throughput_tier(input.receiver_media_bytes, input.receiver_media_seconds);
    let stream_throughput = match throughput {
        "high" => check(
            CheckState::Ok,
            "Receiver fetch throughput is in the high tier.",
        ),
        "moderate" => check(
            CheckState::Ok,
            "Receiver fetch throughput is in the moderate tier.",
        ),
        "low" => check(
            CheckState::Warning,
            "Receiver fetch throughput is in the low tier.",
        ),
        _ => check(
            CheckState::Unknown,
            "No receiver fetch throughput was recorded.",
        ),
    };
    let stalls = stall_tier(input.receiver_stalled_requests);
    let stream_stalls = match stalls {
        "none" => check(
            CheckState::Ok,
            "No interrupted receiver fetches were recorded.",
        ),
        "occasional" => check(
            CheckState::Warning,
            "Some receiver fetches were interrupted before completion.",
        ),
        _ => check(
            CheckState::Failed,
            "Receiver fetches were interrupted repeatedly.",
        ),
    };
    let playback = if input.playback_error {
        check(
            CheckState::Failed,
            "The receiver reported a load or playback error.",
        )
    } else if matches!(input.playback_state.as_str(), "playing" | "buffering") {
        check(
            CheckState::Ok,
            "Playback is active according to the receiver.",
        )
    } else if input.playback_state.is_empty() {
        check(CheckState::Unknown, "Playback state is not available.")
    } else {
        check(CheckState::Warning, "Playback is not currently active.")
    };
    let queue = if !input.current_entry {
        if input.next_entry {
            check(CheckState::Ok, "A queued video is ready to play.")
        } else {
            check(CheckState::Unknown, "No active queue entry.")
        }
    } else if input.next_entry || input.repeat_all {
        check(CheckState::Ok, "Another queue entry is available.")
    } else {
        check(CheckState::Warning, "This is the final queue entry.")
    };
    // Only the compatibility verdict is reported. Container and MIME strings
    // are intentionally not echoed back, so even a malformed database value
    // cannot become visible UI text.
    let media_format = match input.media_format.as_ref() {
        Some((_, _, true)) if input.av1_webm => check(
            CheckState::Warning,
            "The current WebM uses AV1; receiver playback and status reporting vary.",
        ),
        Some((_, _, true)) => check(
            CheckState::Ok,
            "The current container is a common streaming format; codec support still depends on the receiver.",
        ),
        Some((_, _, false)) => check(
            CheckState::Warning,
            "The current container is not accepted by every receiver.",
        ),
        None => check(CheckState::Unknown, "No current media file to inspect."),
    };

    let mut guidance = Vec::new();
    let mut add = |message: &str| {
        if !guidance.iter().any(|existing| existing == message) {
            guidance.push(message.to_string());
        }
    };
    if !input.lan_selected {
        add("Use a private IPv4 network. VPNs and virtual adapters can make this PC choose the wrong interface.");
    }
    if !input.server_running {
        add("Start the local media server in Network and Cast before casting.");
    } else if !input.local_probe_ok {
        add("Check the streaming port and firewall; the PC itself could not reach the media endpoint.");
    }
    if input.receiver_count == 0 {
        add("Power on the receiver and confirm discovery is enabled on the same router.");
    }
    if input.different_subnet_receivers > 0 && input.same_subnet_receivers == 0 {
        add("Put the receiver and this PC on the same SSID and IPv4 subnet. Avoid guest Wi-Fi, VLANs, and AP isolation.");
    }
    if input.receiver_control == Some(false) {
        add("Move the receiver closer to the router and disable client isolation if the control port does not answer.");
    }
    if input.media_url_probe == Some(false) {
        add("Confirm the video is indexed and available; the media URL failed a local ranged probe.");
    }
    if input.receiver_count > 0 && input.receiver_media_requests == 0 {
        add("Control commands can work while media fails. Test playing the video again after fixing LAN reachability.");
    }
    if input.receiver_control == Some(true) && input.receiver_media_requests == 0 {
        add("Control is reachable but the TV did not request video bytes. Weak Wi-Fi or range can allow commands while blocking media; try moving closer, 5 GHz, or Ethernet.");
    }
    if input.receiver_media_requests > 0 && input.receiver_media_bytes == 0 {
        add("The TV connected to the media server but transferred no video data. Check Wi-Fi signal, 5 GHz or Ethernet, and AP isolation.");
    }
    if matches!(input.selected_receiver_matches_session, Some(false)) {
        add("Several receivers answered discovery. Select the active receiver before judging its network path.");
    }
    if link_quality.adapter_kind == "wireless" && link_quality.signal == "weak" {
        add("This PC reports a weak Wi-Fi signal tier. Move closer to the router or use 5 GHz/Ethernet for steadier video.");
    }
    if link_quality.band == "2_4_ghz" && matches!(throughput, "low" | "moderate") {
        add("The link is on 2.4 GHz with limited headroom. Try channels 1, 6, or 11, a 20 MHz channel width, or switch this PC and the TV to 5 GHz.");
    }
    if link_quality.channel_width == "unavailable" && link_quality.adapter_kind == "wireless" {
        add("Windows does not expose channel width here. Check the router settings for 20/40/80 MHz if 2.4 GHz remains unstable.");
    }
    if matches!(throughput, "low") && input.receiver_media_bytes > 0 {
        add("Video bytes are reaching the TV slowly. Reduce distance and interference before changing codecs or seeking.");
    }
    if stalls == "frequent" {
        add("Receiver fetches are stalling repeatedly. Check AP isolation, Wi-Fi range, router load, and QoS before retrying playback.");
    }
    if throughput == "high"
        && input.playback_state == "idle"
        && input.current_entry
        && !input.playback_error
    {
        add("Video bytes are flowing but the receiver reports idle. This looks more like receiver status or format handling than a weak network link.");
    }
    if input.playback_error {
        add("Use Reconnect & resume after fixing the network, or Next to skip the failed item.");
    }
    if input
        .media_format
        .as_ref()
        .map(|(_, _, supported)| !supported)
        .unwrap_or(false)
    {
        add("Use MP4, M4V, or WebM with H.264/AAC for the widest receiver compatibility.");
    }
    if input.av1_webm {
        add("AV1 WebM is less consistent on Cast receivers. If the TV plays but the app shows Idle, try H.264/AAC MP4 or VP9 WebM.");
    }

    // Fixed event codes only. No status text, error string, address, path, or
    // URL is ever accepted here, so the in-memory log stays safe to display.
    if !input.lan_selected {
        record_event("LAN_ADAPTER_MISSING", "No private LAN adapter is selected.");
    }
    if link_quality.signal == "weak" {
        record_event("WEAK_SIGNAL", "PC Wi-Fi signal tier is weak.");
    }
    if link_quality.band == "2_4_ghz" && matches!(throughput, "low" | "moderate") {
        record_event(
            "LOW_2_4_GHZ_HEADROOM",
            "2.4 GHz link has limited throughput headroom.",
        );
    }
    if input.receiver_control == Some(true) && input.receiver_media_bytes == 0 {
        record_event(
            "CONTROL_OK_MEDIA_STALLED",
            "Control answered but no video data was fetched.",
        );
    }
    if stalls == "frequent" {
        record_event(
            "MEDIA_FETCH_STALLS",
            "Receiver fetches were interrupted repeatedly.",
        );
    }
    if throughput == "high"
        && input.playback_state == "idle"
        && input.current_entry
        && !input.playback_error
    {
        record_event(
            "PLAYBACK_IDLE_WITH_MEDIA",
            "Bytes are flowing while the receiver reports idle.",
        );
    }
    if matches!(input.selected_receiver_matches_session, Some(false)) {
        record_event(
            "RECEIVER_AMBIGUOUS",
            "Several receivers were discovered; correlation is uncertain.",
        );
    }

    let transport = match input.transport.as_str() {
        "dlna" => "dlna",
        "cast" => "cast",
        _ => "unknown",
    }
    .to_string();

    NetworkCompatibilityDiagnostics {
        checked_at_epoch_ms,
        transport,
        lan_adapter,
        local_server,
        receiver_discovery,
        receiver_network,
        receiver_match,
        receiver_control,
        media_url,
        receiver_media,
        link_quality: link_quality.clone(),
        stream_throughput,
        stream_stalls,
        playback,
        queue,
        media_format,
        guidance,
        events: diagnostic_events(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_built_from_allow_listed_inputs_only() {
        // Even if a caller accidentally supplies sensitive-looking text, it
        // must not survive into the serialized report.
        let sensitive =
            "192.168.44.7 cast_token http://example.test/secret C:\\Users\\sic\\secret.key";
        let input = CompatibilityInputs {
            transport: sensitive.to_string(),
            lan_selected: true,
            server_running: true,
            local_probe_ok: true,
            receiver_count: 1,
            same_subnet_receivers: 1,
            different_subnet_receivers: 0,
            receiver_control: Some(true),
            media_url_probe: Some(true),
            receiver_media_requests: 1,
            receiver_media_bytes: 4096,
            receiver_media_seconds: 2,
            receiver_stalled_requests: 0,
            selected_receiver_matches_session: Some(true),
            link_quality: LinkQuality {
                adapter_kind: sensitive.to_string(),
                band: sensitive.to_string(),
                channel: Some(6),
                channel_width: sensitive.to_string(),
                signal: sensitive.to_string(),
                link_speed: sensitive.to_string(),
            },
            playback_error: false,
            playback_state: sensitive.to_string(),
            current_entry: true,
            next_entry: true,
            repeat_all: false,
            media_format: Some((sensitive.to_string(), sensitive.to_string(), true)),
            av1_webm: false,
        };
        let report = build_diagnostics(&input, 1234);
        assert_eq!(report.transport, "unknown");
        assert_eq!(report.lan_adapter.state, CheckState::Ok);
        assert_eq!(report.receiver_network.state, CheckState::Ok);
        assert_eq!(report.receiver_media.state, CheckState::Ok);
        assert_eq!(report.media_format.state, CheckState::Ok);
        let serialized = serde_json::to_string(&report).unwrap();
        for needle in ["192.168.44.7", "cast_token", "http://", "secret.key"] {
            assert!(
                !serialized.contains(needle),
                "sensitive value escaped into report: {needle}"
            );
        }
    }

    #[test]
    fn control_without_media_points_to_weak_wifi_or_range() {
        let input = CompatibilityInputs {
            transport: "cast".to_string(),
            lan_selected: true,
            server_running: true,
            local_probe_ok: true,
            receiver_count: 1,
            same_subnet_receivers: 1,
            receiver_control: Some(true),
            media_url_probe: Some(true),
            receiver_media_requests: 0,
            playback_error: true,
            playback_state: "idle".to_string(),
            current_entry: true,
            ..Default::default()
        };
        let report = build_diagnostics(&input, 1234);
        assert_eq!(report.receiver_control.state, CheckState::Ok);
        assert_eq!(report.receiver_media.state, CheckState::Unknown);
        assert!(report
            .guidance
            .iter()
            .any(|line| line.contains("Weak Wi-Fi or range")));
    }

    #[test]
    fn av1_webm_produces_a_fixed_compatibility_warning() {
        let input = CompatibilityInputs {
            transport: "cast".to_string(),
            current_entry: true,
            media_format: Some(("WEBM".to_string(), "video/webm".to_string(), true)),
            av1_webm: true,
            ..Default::default()
        };
        let report = build_diagnostics(&input, 1234);
        assert_eq!(report.media_format.state, CheckState::Warning);
        assert_eq!(
            report.media_format.detail,
            "The current WebM uses AV1; receiver playback and status reporting vary."
        );
        assert!(report.guidance.iter().any(|line| line.contains("AV1 WebM")));
    }

    #[test]
    fn different_subnet_receivers_produce_fixed_guidance() {
        let input = CompatibilityInputs {
            receiver_count: 2,
            same_subnet_receivers: 0,
            different_subnet_receivers: 2,
            receiver_control: Some(false),
            receiver_media_requests: 0,
            ..Default::default()
        };
        let report = build_diagnostics(&input, 1234);
        assert_eq!(report.receiver_network.state, CheckState::Warning);
        assert_eq!(report.receiver_control.state, CheckState::Failed);
        assert!(report
            .guidance
            .iter()
            .any(|line| line.contains("same SSID")));
        assert!(report
            .guidance
            .iter()
            .any(|line| line.contains("client isolation")));
    }

    #[test]
    fn link_quality_is_reduced_to_allow_listed_values() {
        let input = CompatibilityInputs {
            link_quality: LinkQuality {
                adapter_kind: "wireless".to_string(),
                band: "2_4_ghz".to_string(),
                channel: Some(6),
                channel_width: "unavailable".to_string(),
                signal: "weak".to_string(),
                link_speed: "low".to_string(),
            },
            receiver_media_requests: 1,
            receiver_media_bytes: 100_000,
            receiver_media_seconds: 1,
            ..Default::default()
        };
        let report = build_diagnostics(&input, 1234);
        assert_eq!(report.link_quality.signal, "weak");
        assert_eq!(report.link_quality.channel, Some(6));
        assert_eq!(report.stream_throughput.state, CheckState::Warning);
        assert!(report
            .guidance
            .iter()
            .any(|line| line.contains("weak Wi-Fi")));
        assert!(report
            .guidance
            .iter()
            .any(|line| line.contains("channels 1, 6, or 11")));
    }

    #[test]
    fn stalled_receiver_fetches_warn_without_raw_status() {
        let input = CompatibilityInputs {
            receiver_media_requests: 3,
            receiver_media_bytes: 100_000,
            receiver_media_seconds: 1,
            receiver_stalled_requests: 3,
            ..Default::default()
        };
        let report = build_diagnostics(&input, 1234);
        assert_eq!(report.stream_stalls.state, CheckState::Failed);
        assert_eq!(report.stream_throughput.state, CheckState::Warning);
        assert!(report
            .guidance
            .iter()
            .any(|line| line.contains("stalling repeatedly")));
    }

    #[test]
    fn event_log_records_only_fixed_codes() {
        record_event("TEST_FIXED_EVENT", "Fixed detail.");
        let events = diagnostic_events();
        assert!(events
            .iter()
            .any(|event| event.code == "TEST_FIXED_EVENT" && event.detail == "Fixed detail."));
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(!serialized.contains("192.168."));
        assert!(!serialized.contains("http://"));
    }
}
