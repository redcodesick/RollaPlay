//! Lightweight Google Cast discovery for the ROLLAPlay Connect panel.
//!
//! The official Web Sender SDK can only open Chrome's own receiver picker; it
//! does not expose a device list. This module performs a real mDNS query for
//! `_googlecast._tcp.local.` so the desktop UI can show the actual receivers on
//! the LAN. Selecting a device still hands off to the official SDK, which is
//! the only supported way to create a Cast session from the web sender.
//!
//! Only device identity/name/model/address are parsed. Raw packets are never
//! logged or persisted.

use crate::network::{select_lan_address, LanAddress};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, UdpSocket},
    time::{Duration, Instant},
};

const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const MDNS_PORT: u16 = 5353;
const CAST_SERVICE: &str = "_googlecast._tcp.local.";
// mDNS responses with several TXT records can exceed the old 4 KiB buffer.
const MAX_PACKET: usize = 9216;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CastDevice {
    /// Stable device id advertised by the receiver, or its instance name.
    pub id: String,
    /// Friendly name from the `fn` TXT record, or the mDNS instance name.
    pub name: String,
    /// Model from the `md` TXT record when advertised.
    pub model: String,
    /// LAN IPv4 address of the receiver.
    pub address: String,
    /// Cast channel port from the SRV record.
    pub port: u16,
    /// Discovery transport: `mDNS` for Google Cast, `SSDP` for DLNA renderers.
    pub source: String,
    /// Which playback transport this device uses: `cast` or `dlna`.
    #[serde(default = "default_transport")]
    pub transport: String,
    /// AVTransport SOAP control URL for DLNA renderers.
    #[serde(default)]
    pub avtransport_url: Option<String>,
    /// RenderingControl SOAP control URL for DLNA renderers.
    #[serde(default)]
    pub rendering_url: Option<String>,
    /// Original SSDP device description URL, used only for diagnostics.
    #[serde(default)]
    pub description_url: Option<String>,
}

fn default_transport() -> String {
    "cast".to_string()
}

#[derive(Default)]
struct DeviceAccumulator {
    instance: String,
    friendly_name: Option<String>,
    model: Option<String>,
    id: Option<String>,
    server: Option<String>,
    address: Option<String>,
    port: Option<u16>,
}

/// Runs one live mDNS discovery pass. The command layer calls this on a
/// blocking thread; the timeout keeps the UI responsive when no device replies.
pub fn discover(timeout: Duration) -> Result<Vec<CastDevice>, String> {
    let interfaces = crate::network::detect_network().interfaces;
    let selected: Option<LanAddress> = select_lan_address(&interfaces);
    let mut candidates: Vec<Ipv4Addr> = interfaces
        .iter()
        .filter(|interface| interface.is_up && interface.is_running && !interface.is_loopback && !interface.is_tun)
        .flat_map(|interface| interface.ipv4_addresses.iter())
        .filter_map(|address| address.parse::<Ipv4Addr>().ok())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast())
        .collect();
    if let Some(ip) = selected.as_ref().and_then(|address| address.ipv4.parse::<Ipv4Addr>().ok()) {
        if !candidates.contains(&ip) {
            candidates.insert(0, ip);
        }
    }

    // Bind one socket per LAN address. Windows chooses the outgoing interface
    // from the socket's local address, so a single 0.0.0.0 socket can query a
    // VPN/virtual adapter while the receiver sits on the physical LAN.
    let mut sockets: Vec<(Ipv4Addr, UdpSocket)> = Vec::new();
    for ip in &candidates {
        let Ok(socket) = UdpSocket::bind((*ip, MDNS_PORT)).or_else(|_| UdpSocket::bind((*ip, 0))) else { continue };
        let _ = socket.set_read_timeout(Some(Duration::from_millis(80)));
        let _ = socket.set_multicast_loop_v4(true);
        // mDNS requires IP TTL 255. Chromecast receivers may ignore queries
        // with a lower TTL, which shows up as an empty device list.
        let _ = socket.set_multicast_ttl_v4(255);
        if socket.join_multicast_v4(&MDNS_GROUP, ip).is_err() {
            continue;
        }
        sockets.push((*ip, socket));
    }
    if sockets.is_empty() {
        if let Ok(socket) = UdpSocket::bind(("0.0.0.0", MDNS_PORT)).or_else(|_| UdpSocket::bind("0.0.0.0:0")) {
            let _ = socket.set_read_timeout(Some(Duration::from_millis(80)));
            let _ = socket.set_multicast_loop_v4(true);
            let _ = socket.set_multicast_ttl_v4(255);
            let _ = socket.join_multicast_v4(&MDNS_GROUP, &Ipv4Addr::UNSPECIFIED);
            sockets.push((Ipv4Addr::UNSPECIFIED, socket));
        }
    }
    if sockets.is_empty() {
        return Err("Could not open a local socket for Cast discovery.".to_string());
    }

    // Send both a multicast query and a unicast-response query. Some receivers
    // answer only one of them depending on their mDNS implementation.
    let multicast_query = build_query(false);
    let unicast_query = build_query(true);
    let mut sent_any = false;
    for (_, socket) in &sockets {
        let unicast_sent = socket.send_to(&unicast_query, (MDNS_GROUP, MDNS_PORT)).is_ok();
        let multicast_sent = socket.send_to(&multicast_query, (MDNS_GROUP, MDNS_PORT)).is_ok();
        sent_any |= unicast_sent || multicast_sent;
    }
    if !sent_any {
        return Err("Could not send Cast discovery on this network.".to_string());
    }

    let started = Instant::now();
    let deadline = started + timeout;
    let mut repeat_at = started + Duration::from_millis(900);
    let mut found: HashMap<String, DeviceAccumulator> = HashMap::new();
    let mut host_addresses: HashMap<String, String> = HashMap::new();
    let mut buffer = [0u8; MAX_PACKET];
    while Instant::now() < deadline {
        if found.is_empty() && Instant::now() >= repeat_at {
            // A second query catches receivers that answer only after their
            // multicast socket has warmed up.
            for (_, socket) in &sockets {
                let _ = socket.send_to(&unicast_query, (MDNS_GROUP, MDNS_PORT));
                let _ = socket.send_to(&multicast_query, (MDNS_GROUP, MDNS_PORT));
            }
            repeat_at = Instant::now() + Duration::from_secs(2);
        }
        for (_, socket) in &mut sockets {
            match socket.recv_from(&mut buffer) {
                Ok((length, _)) => parse_message(&buffer[..length], &mut found, &mut host_addresses),
                Err(error) => match error.kind() {
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => continue,
                    _ => continue,
                },
            }
        }
    }

    let mut devices: Vec<CastDevice> = found
        .into_values()
        .filter_map(|entry| {
            let instance = clean_text(&entry.instance, 120)?;
            let friendly = entry
                .friendly_name
                .as_deref()
                .and_then(|value| clean_text(value, 120))
                .unwrap_or_else(|| instance.clone());
            let address = entry
                .address
                .as_deref()
                .or_else(|| entry.server.as_deref().and_then(|host| host_addresses.get(&host.to_ascii_lowercase()).map(String::as_str)))
                .and_then(|value| clean_text(value, 64))?;
            let address_ip = address.parse::<Ipv4Addr>().ok()?;
            if address_ip.is_unspecified() || address_ip.is_loopback() || address_ip.is_multicast() {
                return None;
            }
            Some(CastDevice {
                id: clean_text(entry.id.as_deref().unwrap_or(&instance), 120).unwrap_or(instance),
                name: friendly,
                model: entry
                    .model
                    .as_deref()
                    .and_then(|value| clean_text(value, 80))
                    .unwrap_or_else(|| "Google Cast".to_string()),
                address,
                port: entry.port.unwrap_or(8009),
                source: "mDNS".to_string(),
                transport: default_transport(),
                avtransport_url: None,
                rendering_url: None,
                description_url: None,
            })
        })
        .collect();
    devices.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    devices.dedup_by(|left, right| left.id == right.id && left.address == right.address);
    Ok(devices)
}

/// Decoded DNS names have no trailing dot; compare without one so PTR targets
/// and TXT owners are recognized whether or not a responder encodes the dot.
fn is_cast_service_name(name: &str) -> bool {
    let normalized = name.trim_end_matches('.').to_ascii_lowercase();
    normalized == "_googlecast._tcp.local" || normalized.ends_with("._googlecast._tcp.local")
}

fn clean_text(value: &str, max_chars: usize) -> Option<String> {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn encode_name(name: &str, output: &mut Vec<u8>) {
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() {
            continue;
        }
        let encoded = label.as_bytes();
        output.push(encoded.len().min(63) as u8);
        output.extend_from_slice(&encoded[..encoded.len().min(63)]);
    }
    output.push(0);
}

fn build_query(unicast_response: bool) -> Vec<u8> {
    let mut query = Vec::with_capacity(64);
    query.extend_from_slice(&0u16.to_be_bytes());
    query.extend_from_slice(&0u16.to_be_bytes());
    query.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    query.extend_from_slice(&[0u8; 6]);
    encode_name(CAST_SERVICE, &mut query);
    query.extend_from_slice(&12u16.to_be_bytes()); // PTR
    // 0x8000 is the mDNS "unicast response" hint. Send both forms because
    // receivers differ in which one they answer.
    let qclass = if unicast_response { 0x8001u16 } else { 0x0001u16 };
    query.extend_from_slice(&qclass.to_be_bytes());
    query
}

fn read_u16(buffer: &[u8], offset: usize) -> Option<u16> {
    let bytes = buffer.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

/// Reads a DNS name, including RFC 1035 compression pointers. Returns the
/// decoded name and the offset immediately after the original (uncompressed)
/// name so the caller can keep walking the record.
fn read_name(buffer: &[u8], offset: usize) -> Option<(String, usize)> {
    let mut cursor = offset;
    let mut next_offset = None;
    let mut jumped = false;
    let mut labels = Vec::new();
    let mut hops = 0usize;

    loop {
        let length = *buffer.get(cursor)? as usize;
        if length & 0xc0 == 0xc0 {
            let pointer_low = *buffer.get(cursor + 1)? as usize;
            let pointer = ((length & 0x3f) << 8) | pointer_low;
            if !jumped {
                next_offset = Some(cursor + 2);
                jumped = true;
            }
            cursor = pointer;
            hops += 1;
            if hops > 32 {
                return None;
            }
            continue;
        }
        if length == 0 {
            if !jumped {
                next_offset = Some(cursor + 1);
            }
            break;
        }
        if length > 63 || cursor + 1 + length > buffer.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&buffer[cursor + 1..cursor + 1 + length]).to_string());
        cursor += 1 + length;
    }

    Some((labels.join("."), next_offset?))
}

fn txt_values(data: &[u8]) -> Vec<(String, String)> {
    let mut values = Vec::new();
    let mut cursor = 0usize;
    while cursor < data.len() {
        let length = data[cursor] as usize;
        cursor += 1;
        if length == 0 || cursor + length > data.len() {
            break;
        }
        let chunk = &data[cursor..cursor + length];
        cursor += length;
        if let Some(split) = chunk.iter().position(|byte| *byte == b'=') {
            let key = String::from_utf8_lossy(&chunk[..split]).to_ascii_lowercase();
            let value = String::from_utf8_lossy(&chunk[split + 1..]).to_string();
            values.push((key, value));
        }
    }
    values
}

fn record_entry<'a>(
    found: &'a mut HashMap<String, DeviceAccumulator>,
    instance: &str,
) -> &'a mut DeviceAccumulator {
    found
        .entry(instance.to_ascii_lowercase())
        .or_insert_with(|| DeviceAccumulator {
            instance: instance.to_string(),
            ..DeviceAccumulator::default()
        })
}

fn parse_message(
    buffer: &[u8],
    found: &mut HashMap<String, DeviceAccumulator>,
    host_addresses: &mut HashMap<String, String>,
) {
    if buffer.len() < 12 {
        return;
    }
    let questions = match read_u16(buffer, 4) {
        Some(value) => value as usize,
        None => return,
    };
    let answers = match read_u16(buffer, 6) {
        Some(value) => value as usize,
        None => return,
    };
    let authority = read_u16(buffer, 8).unwrap_or(0) as usize;
    let additional = read_u16(buffer, 10).unwrap_or(0) as usize;
    let mut offset = 12usize;

    for _ in 0..questions {
        let Some((_, next)) = read_name(buffer, offset) else { return };
        offset = next;
        offset += 4;
        if offset > buffer.len() {
            return;
        }
    }

    for _ in 0..answers.saturating_add(authority).saturating_add(additional) {
        let Some((owner, next)) = read_name(buffer, offset) else { return };
        offset = next;
        let Some(record_type) = read_u16(buffer, offset) else { return };
        let class = read_u16(buffer, offset + 2).unwrap_or(0);
        let data_length = match read_u16(buffer, offset + 8) {
            Some(value) => value as usize,
            None => return,
        };
        offset += 10;
        if offset + data_length > buffer.len() {
            return;
        }
        let data = &buffer[offset..offset + data_length];
        let owner_lower = owner.to_ascii_lowercase();
        // mDNS sets the cache-flush bit (0x8000) on many response records.
        if class & 0x7fff == 1 {
            match record_type {
                12 => {
                    // PTR: service -> <instance>._googlecast._tcp.local.
                    // Compression pointers in RDATA are offsets from the start
                    // of the whole DNS message, not from the start of RDATA.
                    if let Some((target, _)) = read_name(buffer, offset) {
                        if is_cast_service_name(&target) {
                            let entry = record_entry(found, &target);
                            if entry.instance.is_empty() {
                                entry.instance = target;
                            }
                        }
                    }
                }
                33 => {
                    // SRV: priority, weight, port, target host.
                    if let (Some(port), Some((target, _))) =
                        (read_u16(data, 4), read_name(buffer, offset + 6))
                    {
                        let entry = record_entry(found, &owner);
                        entry.port = Some(port);
                        entry.server = Some(target.clone());
                    }
                }
                16 => {
                    if is_cast_service_name(&owner) {
                        let entry = record_entry(found, &owner);
                        for (key, value) in txt_values(data) {
                            match key.as_str() {
                                "fn" => entry.friendly_name = Some(value),
                                "md" => entry.model = Some(value),
                                "id" => entry.id = Some(value),
                                _ => {}
                            }
                        }
                    }
                }
                1 => {
                    if data.len() >= 4 {
                        let address = Ipv4Addr::new(data[0], data[1], data[2], data[3]).to_string();
                        if is_cast_service_name(&owner) {
                            record_entry(found, &owner).address = Some(address);
                        } else {
                            host_addresses.insert(owner_lower, address);
                        }
                    }
                }
                _ => {}
            }
        }
        offset += data_length;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_name(buffer: &mut Vec<u8>, name: &str) {
        encode_name(name, buffer);
    }

    fn append_record(buffer: &mut Vec<u8>, owner: &str, kind: u16, data: &[u8]) {
        append_name(buffer, owner);
        buffer.extend_from_slice(&kind.to_be_bytes());
        buffer.extend_from_slice(&1u16.to_be_bytes());
        buffer.extend_from_slice(&0u32.to_be_bytes());
        buffer.extend_from_slice(&(data.len() as u16).to_be_bytes());
        buffer.extend_from_slice(data);
    }

    fn response(count: u16) -> Vec<u8> {
        let mut buffer = vec![0u8; 12];
        buffer[6..8].copy_from_slice(&count.to_be_bytes());
        buffer
    }

    /// TXT RDATA is a sequence of length-prefixed strings, not NUL-separated.
    fn txt_data(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (key, value) in pairs {
            let chunk = format!("{key}={value}");
            data.push(chunk.len() as u8);
            data.extend_from_slice(chunk.as_bytes());
        }
        data
    }

    #[test]
    fn dns_names_follow_compression_pointers() {
        let mut buffer = Vec::new();
        append_name(&mut buffer, "_googlecast._tcp.local");
        let pointer_at = buffer.len();
        buffer.extend_from_slice(&0xc000u16.to_be_bytes());
        // The first label length byte sits at offset 0.
        buffer[pointer_at + 1] = 0;
        let (name, _) = read_name(&buffer, pointer_at).unwrap();
        assert_eq!(name, "_googlecast._tcp.local");
    }

    #[test]
    fn txt_values_read_entity_safe_pairs() {
        let raw = txt_data(&[
            ("fn", "Xiaomi Mi Box"),
            ("md", "Mi Box"),
            ("id", "abc123"),
        ]);
        assert_eq!(
            txt_values(&raw),
            vec![
                ("fn".to_string(), "Xiaomi Mi Box".to_string()),
                ("md".to_string(), "Mi Box".to_string()),
                ("id".to_string(), "abc123".to_string()),
            ]
        );
    }

    #[test]
    fn parse_message_extracts_a_real_receiver() {
        let instance = "Xiaomi Mi Box._googlecast._tcp.local";
        let mut ptr = Vec::new();
        append_name(&mut ptr, instance);
        let mut srv = vec![0u8; 6];
        srv[4..6].copy_from_slice(&8009u16.to_be_bytes());
        append_name(&mut srv, "mi-box.local");
        let txt = txt_data(&[
            ("fn", "Xiaomi Mi Box"),
            ("md", "Mi Box"),
            ("id", "mi-box-1"),
        ]);

        let mut buffer = response(4);
        append_record(&mut buffer, CAST_SERVICE, 12, &ptr);
        append_record(&mut buffer, instance, 33, &srv);
        append_record(&mut buffer, instance, 16, &txt);
        append_record(&mut buffer, "mi-box.local", 1, &[192, 168, 1, 42]);

        let mut found = HashMap::new();
        let mut hosts = HashMap::new();
        parse_message(&buffer, &mut found, &mut hosts);
        assert_eq!(found.len(), 1);
        let entry = found.values().next().unwrap();
        assert_eq!(entry.friendly_name.as_deref(), Some("Xiaomi Mi Box"));
        assert_eq!(entry.port, Some(8009));
        assert_eq!(hosts.get("mi-box.local").map(String::as_str), Some("192.168.1.42"));
    }

    #[test]
    fn query_sends_both_multicast_and_unicast_response_hints() {
        let multicast = build_query(false);
        let unicast = build_query(true);
        assert_eq!(&multicast[multicast.len() - 2..], &[0x00, 0x01]);
        assert_eq!(&unicast[unicast.len() - 2..], &[0x80, 0x01]);
    }

    #[test]
    fn service_names_match_with_or_without_a_trailing_dot() {
        assert!(is_cast_service_name("_googlecast._tcp.local"));
        assert!(is_cast_service_name("_googlecast._tcp.local."));
        assert!(is_cast_service_name("Xiaomi Mi Box._googlecast._tcp.local"));
        assert!(is_cast_service_name("Xiaomi Mi Box._googlecast._tcp.local."));
        assert!(!is_cast_service_name("_airplay._tcp.local"));
    }

    #[test]
    fn clean_text_removes_control_characters() {
        assert_eq!(clean_text("  Living\u{0007} Room  ", 80).unwrap(), "Living Room");
        assert!(clean_text("\u{0000}", 80).is_none());
    }
}
