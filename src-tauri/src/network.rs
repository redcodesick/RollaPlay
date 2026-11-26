use netdev::get_interfaces;
use serde::Serialize;
use std::net::Ipv4Addr;
#[cfg(windows)]
use std::{os::windows::process::CommandExt, process::Command};

#[derive(Debug, Clone, Serialize)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub interface_type: String,
    pub ipv4_addresses: Vec<String>,
    pub ipv4_prefix_lengths: Vec<u8>,
    pub is_up: bool,
    pub is_running: bool,
    pub is_loopback: bool,
    pub is_tun: bool,
    pub is_physical: bool,
    pub is_default_route: bool,
    pub transmit_speed_bits: Option<u64>,
    pub receive_speed_bits: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LanAddress {
    pub ipv4: String,
    pub interface_name: String,
    pub display_name: Option<String>,
    pub is_default_route: bool,
    pub prefix_length: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkDiagnostics {
    pub selected: Option<LanAddress>,
    pub interfaces: Vec<NetworkInterfaceInfo>,
}

/// Allow-listed link-quality tiers. Numeric channel is included because it is
/// router configuration, not device identity; everything else is a fixed tier.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LinkQuality {
    pub adapter_kind: String,
    pub band: String,
    pub channel: Option<u16>,
    pub channel_width: String,
    pub signal: String,
    pub link_speed: String,
}

impl Default for LinkQuality {
    fn default() -> Self {
        Self {
            adapter_kind: "unknown".to_string(),
            band: "unknown".to_string(),
            channel: None,
            channel_width: "unavailable".to_string(),
            signal: "unknown".to_string(),
            link_speed: "unknown".to_string(),
        }
    }
}

fn speed_tier(bits_per_second: Option<u64>) -> String {
    let mbps = match bits_per_second {
        Some(value) if value > 0 => value / 1_000_000,
        _ => return "unknown".to_string(),
    };
    match mbps {
        300.. => "high".to_string(),
        100..=299 => "moderate".to_string(),
        40..=99 => "low".to_string(),
        1..=39 => "very_low".to_string(),
        _ => "unknown".to_string(),
    }
}

fn signal_tier(percent: u8) -> String {
    match percent {
        70..=100 => "excellent".to_string(),
        50..=69 => "good".to_string(),
        30..=49 => "fair".to_string(),
        0..=29 => "weak".to_string(),
        _ => "unknown".to_string(),
    }
}

fn band_from_channel(channel: u16) -> String {
    match channel {
        1..=14 => "2_4_ghz".to_string(),
        15..=233 => "5_or_6_ghz".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Reads only channel, signal, and rate from the selected WLAN section. SSID,
/// BSSID, MAC address, profile name, and every raw line are discarded here.
fn parse_wlan_fields(output: &str, interface_name: &str) -> (Option<u16>, Option<u8>, Option<u64>) {
    let mut matched = false;
    let mut channel = None;
    let mut signal = None;
    let mut rate_mbps: Option<f64> = None;
    for line in output.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if key == "name" {
            matched = value.eq_ignore_ascii_case(interface_name);
            if matched {
                channel = None;
                signal = None;
                rate_mbps = None;
            }
            continue;
        }
        if !matched {
            continue;
        }
        match key.as_str() {
            "channel" => channel = value.parse::<u16>().ok(),
            "signal" => signal = value.trim_end_matches('%').trim().parse::<u8>().ok(),
            "receive rate (mbps)" | "transmit rate (mbps)" => {
                let parsed = value.parse::<f64>().ok().unwrap_or(0.0);
                if parsed > 0.0 {
                    rate_mbps = Some(rate_mbps.unwrap_or(0.0).max(parsed));
                }
            }
            _ => {}
        }
    }
    let rate_bits = rate_mbps.map(|rate| (rate * 1_000_000.0) as u64);
    (channel, signal, rate_bits)
}

#[cfg(windows)]
fn wlan_fields(interface_name: &str) -> (Option<u16>, Option<u8>, Option<u64>) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(output) = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return (None, None, None);
    };
    if !output.status.success() {
        return (None, None, None);
    }
    parse_wlan_fields(&String::from_utf8_lossy(&output.stdout), interface_name)
}

#[cfg(not(windows))]
fn wlan_fields(_interface_name: &str) -> (Option<u16>, Option<u8>, Option<u64>) {
    (None, None, None)
}

pub fn link_quality_for(interfaces: &[NetworkInterfaceInfo], selected_name: &str) -> LinkQuality {
    let Some(interface) = interfaces.iter().find(|item| item.name == selected_name) else {
        return LinkQuality::default();
    };
    let interface_type = interface.interface_type.to_ascii_lowercase();
    let adapter_kind = if interface_type.contains("wifi") || interface_type.contains("wireless") {
        "wireless"
    } else if interface_type.contains("ethernet") {
        "wired"
    } else {
        "unknown"
    };
    let reported_speed = interface
        .transmit_speed_bits
        .max(interface.receive_speed_bits);
    if adapter_kind == "wireless" {
        let (channel, signal, wlan_speed) = wlan_fields(selected_name);
        LinkQuality {
            adapter_kind: adapter_kind.to_string(),
            band: channel
                .map(band_from_channel)
                .unwrap_or_else(|| "unknown".to_string()),
            channel,
            channel_width: "unavailable".to_string(),
            signal: signal
                .map(signal_tier)
                .unwrap_or_else(|| "unknown".to_string()),
            link_speed: speed_tier(reported_speed.or(wlan_speed)),
        }
    } else {
        LinkQuality {
            adapter_kind: adapter_kind.to_string(),
            band: if adapter_kind == "wired" {
                "wired".to_string()
            } else {
                "unknown".to_string()
            },
            channel: None,
            channel_width: if adapter_kind == "wired" {
                "not_applicable".to_string()
            } else {
                "unavailable".to_string()
            },
            signal: if adapter_kind == "wired" {
                "wired".to_string()
            } else {
                "unknown".to_string()
            },
            link_speed: speed_tier(reported_speed),
        }
    }
}

pub fn detect_network() -> NetworkDiagnostics {
    let interfaces = get_interfaces()
        .into_iter()
        .map(|interface| NetworkInterfaceInfo {
            name: interface.name.clone(),
            display_name: interface.friendly_name.clone(),
            description: interface.description.clone(),
            interface_type: format!("{:?}", interface.if_type),
            ipv4_addresses: interface.ipv4.iter().map(|address| address.addr().to_string()).collect(),
            ipv4_prefix_lengths: interface.ipv4.iter().map(|address| address.prefix_len()).collect(),
            is_up: interface.is_up(),
            is_running: interface.is_running(),
            is_loopback: interface.is_loopback(),
            is_tun: interface.is_tun(),
            is_physical: interface.is_physical(),
            is_default_route: interface.default,
            transmit_speed_bits: interface.transmit_speed,
            receive_speed_bits: interface.receive_speed,
        })
        .collect::<Vec<_>>();
    let selected = select_lan_address(&interfaces);
    NetworkDiagnostics { selected, interfaces }
}

pub fn select_lan_address(interfaces: &[NetworkInterfaceInfo]) -> Option<LanAddress> {
    interfaces
        .iter()
        .filter(|interface| interface.is_up && interface.is_running && !interface.is_loopback && !interface.is_tun)
        .flat_map(|interface| {
            interface.ipv4_addresses.iter().zip(&interface.ipv4_prefix_lengths).filter_map(move |(address, prefix)| {
                let ip = address.parse::<Ipv4Addr>().ok()?;
                if !(1..=32).contains(prefix) || !is_private_lan_ipv4(ip) {
                    return None;
                }
                Some((interface, ip, *prefix))
            })
        })
        .max_by_key(|(interface, ip, _)| {
            let physical = if interface.is_physical { 1 } else { 0 };
            let default_route = if interface.is_default_route { 1 } else { 0 };
            let interface_type = interface.interface_type.to_ascii_lowercase();
            let preferred_type = if interface_type.contains("ethernet") || interface_type.contains("wifi") || interface_type.contains("wireless") { 1 } else { 0 };
            let address_class = if ip.octets()[0] == 10 || ip.octets()[0] == 192 { 1 } else { 0 };
            default_route * 100 + physical * 20 + preferred_type * 5 + address_class
        })
        .map(|(interface, ip, prefix_length)| LanAddress {
            ipv4: ip.to_string(),
            interface_name: interface.name.clone(),
            display_name: interface.display_name.clone(),
            is_default_route: interface.is_default_route,
            prefix_length,
        })
}

pub fn is_private_lan_ipv4(ip: Ipv4Addr) -> bool {
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !(ip.octets()[0] == 169 && ip.octets()[1] == 254)
        && ip.is_private()
}

#[cfg(test)]
mod tests {
    use super::{is_private_lan_ipv4, select_lan_address, LanAddress, NetworkInterfaceInfo};
    use std::net::Ipv4Addr;

    fn interface(name: &str, ip: &str, default: bool, physical: bool) -> NetworkInterfaceInfo {
        NetworkInterfaceInfo {
            name: name.to_string(),
            display_name: Some(name.to_string()),
            description: None,
            interface_type: "Ethernet".to_string(),
            ipv4_addresses: vec![ip.to_string()],
            ipv4_prefix_lengths: vec![24],
            is_up: true,
            is_running: true,
            is_loopback: false,
            is_tun: false,
            is_physical: physical,
            is_default_route: default,
            transmit_speed_bits: Some(300_000_000),
            receive_speed_bits: Some(300_000_000),
        }
    }

    #[test]
    fn only_rfc1918_unicast_addresses_are_lan_candidates() {
        assert!(is_private_lan_ipv4(Ipv4Addr::new(10, 20, 30, 40)));
        assert!(is_private_lan_ipv4(Ipv4Addr::new(172, 20, 0, 2)));
        assert!(is_private_lan_ipv4(Ipv4Addr::new(192, 168, 1, 9)));
        assert!(!is_private_lan_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_private_lan_ipv4(Ipv4Addr::LOCALHOST));
        assert!(!is_private_lan_ipv4(Ipv4Addr::new(169, 254, 1, 10)));
    }

    #[test]
    fn lan_selection_prefers_the_active_default_physical_adapter() {
        let interfaces = vec![
            interface("virtual", "192.168.88.2", false, false),
            interface("Wi-Fi", "192.168.1.22", true, true),
            interface("public", "8.8.8.8", true, true),
        ];
        assert_eq!(select_lan_address(&interfaces).unwrap(), LanAddress {
            ipv4: "192.168.1.22".to_string(),
            interface_name: "Wi-Fi".to_string(),
            display_name: Some("Wi-Fi".to_string()),
            is_default_route: true,
            prefix_length: 24,
        });
    }

    #[test]
    fn down_loopback_and_tunnel_adapters_are_not_selected() {
        let mut down = interface("down", "192.168.1.4", true, true);
        down.is_up = false;
        let mut loopback = interface("loopback", "192.168.1.5", true, true);
        loopback.is_loopback = true;
        let mut tunnel = interface("tunnel", "192.168.1.6", true, true);
        tunnel.is_tun = true;
        assert!(select_lan_address(&[down, loopback, tunnel]).is_none());
    }

    #[test]
    fn link_quality_reports_tiers_without_network_identity() {
        let sample = "Name                  : Wi-Fi\nSSID                  : PrivateNetworkName\nBSSID                 : aa:bb:cc:dd:ee:ff\nChannel               : 6\nSignal                : 42%\nReceive rate (Mbps)   : 65.0\nTransmit rate (Mbps)  : 65.0\n";
        let (channel, signal, speed) = super::parse_wlan_fields(sample, "Wi-Fi");
        assert_eq!(channel, Some(6));
        assert_eq!(signal, Some(42));
        assert_eq!(speed, Some(65_000_000));
        let serialized = serde_json::to_string(&super::LinkQuality {
            adapter_kind: "wireless".to_string(),
            band: super::band_from_channel(channel.unwrap()),
            channel,
            channel_width: "unavailable".to_string(),
            signal: super::signal_tier(signal.unwrap()),
            link_speed: super::speed_tier(speed),
        })
        .unwrap();
        assert!(!serialized.contains("PrivateNetworkName"));
        assert!(!serialized.contains("aa:bb:cc:dd:ee:ff"));
        assert!(serialized.contains("2_4_ghz"));
        assert!(serialized.contains("fair"));
        assert!(serialized.contains("low"));
    }

    #[test]
    fn link_quality_ignores_other_wlan_sections() {
        let sample = "Name                  : OtherAdapter\nChannel               : 149\nSignal                : 95%\nName                  : Wi-Fi\nChannel               : 11\nSignal                : 81%\n";
        let (channel, signal, _) = super::parse_wlan_fields(sample, "Wi-Fi");
        assert_eq!(channel, Some(11));
        assert_eq!(signal, Some(81));
    }

    #[test]
    fn wired_adapters_report_wired_tiers_only() {
        let interfaces = vec![interface("Ethernet", "192.168.1.20", true, true)];
        let quality = super::link_quality_for(&interfaces, "Ethernet");
        assert_eq!(quality.adapter_kind, "wired");
        assert_eq!(quality.signal, "wired");
        assert_eq!(quality.link_speed, "high");
        assert_eq!(quality.channel_width, "not_applicable");
        assert!(quality.channel.is_none());
    }
}
