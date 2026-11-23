use netdev::get_interfaces;
use serde::Serialize;
use std::net::Ipv4Addr;

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
}
