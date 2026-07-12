//! Network interface enumeration abstractions.
//!
//! Phase 1 defines the types. Actual platform-specific enumeration
//! (`getifaddrs` on macOS, `GetAdaptersAddresses` on Windows) is added
//! when the gatherer starts performing real I/O.

use crate::error::P2pError;
use network_interface::{Addr, NetworkInterface as ExternalInterface, NetworkInterfaceConfig};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Enumerates all local network interfaces and maps them to our [`NetworkInterface`] struct.
///
/// # Errors
///
/// Returns a [`P2pError::Internal`] if listing system network interfaces fails.
pub fn enumerate_interfaces() -> Result<Vec<NetworkInterface>, P2pError> {
    let external_interfaces = ExternalInterface::show().map_err(|error| P2pError::Internal {
        reason: format!("Failed to list network interfaces: {error}"),
    })?;

    let mut result = Vec::new();
    for ext_iface in external_interfaces {
        for addr in ext_iface.addr {
            let address = match addr {
                Addr::V4(v4) => IpAddr::V4(v4.ip),
                Addr::V6(v6) => IpAddr::V6(v6.ip),
            };

            let kind = detect_interface_kind(&ext_iface.name);
            let cost = cost_from_kind(kind);

            result.push(NetworkInterface {
                name: ext_iface.name.clone(),
                index: ext_iface.index,
                address,
                kind,
                is_up: true,
                cost,
            });
        }
    }

    Ok(result)
}

fn detect_interface_kind(name: &str) -> InterfaceKind {
    let name_lower = name.to_lowercase();
    if name_lower.contains("loopback") || name_lower == "lo" || name_lower == "lo0" {
        InterfaceKind::Loopback
    } else if name_lower.contains("wlan")
        || name_lower.contains("wifi")
        || name_lower.contains("wi-fi")
        || name_lower.contains("awdl")
    {
        InterfaceKind::WiFi
    } else if name_lower.contains("vpn")
        || name_lower.contains("tun")
        || name_lower.contains("tap")
        || name_lower.contains("utun")
        || name_lower.contains("wg")
        || name_lower.contains("tailscale")
        || name_lower.contains("zerotier")
        || name_lower.contains("ppp")
    {
        InterfaceKind::Vpn
    } else if name_lower.contains("eth") || name_lower.contains("ethernet") {
        InterfaceKind::Ethernet
    } else if name_lower.contains("en") {
        InterfaceKind::Ethernet
    } else {
        InterfaceKind::Unknown
    }
}

fn cost_from_kind(kind: InterfaceKind) -> InterfaceCost {
    match kind {
        InterfaceKind::Ethernet | InterfaceKind::Loopback => InterfaceCost::Low,
        InterfaceKind::WiFi | InterfaceKind::Virtual | InterfaceKind::Unknown => {
            InterfaceCost::Medium
        }
        InterfaceKind::Cellular | InterfaceKind::Vpn => InterfaceCost::High,
    }
}

/// A discovered local network interface suitable for P2P candidate gathering.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    /// OS-assigned interface name (e.g. "en0", "Ethernet 2").
    pub name: String,
    /// OS-assigned interface index.
    pub index: u32,
    /// Bound IP address on this interface.
    pub address: IpAddr,
    /// Interface kind.
    pub kind: InterfaceKind,
    /// Whether the interface is currently up and has a default route.
    pub is_up: bool,
    /// Estimated cost (lower = preferred). Wi-Fi > Ethernet > VPN.
    pub cost: InterfaceCost,
}

/// Broad classification of a network interface.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterfaceKind {
    Ethernet,
    WiFi,
    Cellular,
    Vpn,
    Loopback,
    Virtual,
    Unknown,
}

/// Relative cost of sending traffic over an interface.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterfaceCost {
    /// Wired or very fast local link.
    Low,
    /// Wi-Fi or similar wireless LAN.
    Medium,
    /// Cellular, metered, or VPN overlay.
    High,
}

/// Filter predicate results for interface selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterfaceFilter {
    /// Interface is usable for P2P.
    Accept,
    /// Interface should be excluded with a reason.
    Reject(InterfaceRejectReason),
}

/// Why an interface was excluded from candidate gathering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterfaceRejectReason {
    Loopback,
    LinkLocal,
    Multicast,
    InterfaceDown,
    NoDefaultRoute,
    ScopeInvalid,
    Disabled,
}

/// Determines whether an interface address is usable for P2P candidate
/// gathering, applying the rules from plan section 9.1.
#[must_use]
pub fn filter_interface(iface: &NetworkInterface) -> InterfaceFilter {
    if iface.kind == InterfaceKind::Loopback {
        return InterfaceFilter::Reject(InterfaceRejectReason::Loopback);
    }
    if !iface.is_up {
        return InterfaceFilter::Reject(InterfaceRejectReason::InterfaceDown);
    }
    match iface.address {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return InterfaceFilter::Reject(InterfaceRejectReason::Loopback);
            }
            if v4.is_link_local() {
                return InterfaceFilter::Reject(InterfaceRejectReason::LinkLocal);
            }
            if v4.is_multicast() {
                return InterfaceFilter::Reject(InterfaceRejectReason::Multicast);
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return InterfaceFilter::Reject(InterfaceRejectReason::Loopback);
            }
            if v6.is_multicast() {
                return InterfaceFilter::Reject(InterfaceRejectReason::Multicast);
            }
            // Link-local IPv6 (fe80::/10) cannot identify a LAN peer across
            // subnets without a valid scope.
            if (v6.segments()[0] & 0xffc0) == 0xfe80 && iface.index == 0 {
                return InterfaceFilter::Reject(InterfaceRejectReason::ScopeInvalid);
            }
        }
    }
    InterfaceFilter::Accept
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn make_iface(addr: IpAddr, kind: InterfaceKind, up: bool) -> NetworkInterface {
        NetworkInterface {
            name: "test0".into(),
            index: 1,
            address: addr,
            kind,
            is_up: up,
            cost: InterfaceCost::Low,
        }
    }

    #[test]
    fn loopback_rejected() {
        let iface = make_iface(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            InterfaceKind::Loopback,
            true,
        );
        assert_eq!(
            filter_interface(&iface),
            InterfaceFilter::Reject(InterfaceRejectReason::Loopback)
        );
    }

    #[test]
    fn down_interface_rejected() {
        let iface = make_iface(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            InterfaceKind::Ethernet,
            false,
        );
        assert_eq!(
            filter_interface(&iface),
            InterfaceFilter::Reject(InterfaceRejectReason::InterfaceDown)
        );
    }

    #[test]
    fn private_ipv4_accepted() {
        let iface = make_iface(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
            InterfaceKind::Ethernet,
            true,
        );
        assert_eq!(filter_interface(&iface), InterfaceFilter::Accept);
    }

    #[test]
    fn global_ipv6_accepted() {
        let iface = make_iface(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            InterfaceKind::Ethernet,
            true,
        );
        assert_eq!(filter_interface(&iface), InterfaceFilter::Accept);
    }

    #[test]
    fn link_local_ipv4_rejected() {
        let iface = make_iface(
            IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)),
            InterfaceKind::WiFi,
            true,
        );
        assert_eq!(
            filter_interface(&iface),
            InterfaceFilter::Reject(InterfaceRejectReason::LinkLocal)
        );
    }

    #[test]
    fn link_local_ipv6_without_scope_rejected() {
        let mut iface = make_iface(
            IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)),
            InterfaceKind::WiFi,
            true,
        );
        iface.index = 0; // no valid scope
        assert_eq!(
            filter_interface(&iface),
            InterfaceFilter::Reject(InterfaceRejectReason::ScopeInvalid)
        );
    }

    #[test]
    fn test_enumerate_interfaces_contains_valid_entries() {
        let result = enumerate_interfaces();
        assert!(
            result.is_ok(),
            "enumerate_interfaces failed: {:?}",
            result.err()
        );
        let list = result.unwrap();
        assert!(!list.is_empty(), "interface list is empty");
        for iface in list {
            assert!(!iface.name.is_empty(), "interface name is empty");
        }
    }
}
