//! Wake-on-LAN: discover this computer's wakeable adapters and send the
//! standard magic packet (6 × 0xFF followed by 16 × the target MAC) over UDP
//! broadcast. Broadcasts do not cross routers, so a wake request only works
//! from a computer on the same LAN as the sleeping one.
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use serde::{Deserialize, Serialize};

use crate::error::DesktopError;

const WAKE_PORTS: [u16; 2] = [9, 7];
const MAX_INTERFACES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeInterface {
    pub mac_address: String,
    pub ipv4_address: String,
    pub prefix_length: u8,
    pub broadcast_address: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeResult {
    /// False when no local adapter is on any of the target's LANs; nothing is sent.
    pub same_lan: bool,
    pub packets_sent: u32,
}

/// Lists wired and Wi-Fi adapters with a MAC and a private IPv4 LAN address.
#[tauri::command]
pub async fn get_wake_on_lan_interfaces() -> Result<Vec<WakeInterface>, DesktopError> {
    tauri::async_runtime::spawn_blocking(|| Ok(wakeable(&list_interfaces()?)))
        .await
        .map_err(|error| DesktopError::Process(error.to_string()))?
}

/// Sends magic packets to every adapter the target host reported.
#[tauri::command]
pub async fn send_wake_on_lan(targets: Vec<WakeInterface>) -> Result<WakeResult, DesktopError> {
    if targets.is_empty() || targets.len() > MAX_INTERFACES {
        return Err(DesktopError::InvalidRequest(
            "Wake-on-LAN needs between 1 and 8 target adapters".into(),
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let local = wakeable(&list_interfaces()?);
        send_to(&local, &targets)
    })
    .await
    .map_err(|error| DesktopError::Process(error.to_string()))?
}

fn list_interfaces() -> Result<Vec<NetworkInterface>, DesktopError> {
    NetworkInterface::show().map_err(|error| {
        DesktopError::Unavailable(format!("network adapters could not be listed: {error}"))
    })
}

fn send_to(local: &[WakeInterface], targets: &[WakeInterface]) -> Result<WakeResult, DesktopError> {
    let mut packets_sent = 0;
    let mut same_lan = false;
    for target in targets {
        let mac = parse_mac(&target.mac_address)?;
        let (Ok(target_ip), Ok(broadcast)) = (
            target.ipv4_address.parse::<Ipv4Addr>(),
            target.broadcast_address.parse::<Ipv4Addr>(),
        ) else {
            continue;
        };
        let Some(source) = local
            .iter()
            .find(|own| shares_lan(own, target_ip, target.prefix_length))
        else {
            continue;
        };
        same_lan = true;
        let Ok(source_ip) = source.ipv4_address.parse::<Ipv4Addr>() else {
            continue;
        };
        // Bind to the adapter on the shared LAN so the broadcast leaves there.
        let socket = UdpSocket::bind(SocketAddrV4::new(source_ip, 0))?;
        socket.set_broadcast(true)?;
        let packet = magic_packet(mac);
        for destination in [broadcast, Ipv4Addr::BROADCAST] {
            for port in WAKE_PORTS {
                if socket
                    .send_to(&packet, SocketAddrV4::new(destination, port))
                    .is_ok()
                {
                    packets_sent += 1;
                }
            }
        }
    }
    Ok(WakeResult {
        same_lan,
        packets_sent,
    })
}

pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [0xFF; 102];
    for chunk in packet[6..].chunks_exact_mut(6) {
        chunk.copy_from_slice(&mac);
    }
    packet
}

pub fn parse_mac(value: &str) -> Result<[u8; 6], DesktopError> {
    let invalid = || DesktopError::InvalidRequest("invalid MAC address".into());
    let separator = if value.contains('-') { '-' } else { ':' };
    let parts = value.trim().split(separator).collect::<Vec<_>>();
    if parts.len() != 6 {
        return Err(invalid());
    }
    let mut mac = [0; 6];
    for (octet, part) in mac.iter_mut().zip(parts) {
        if part.len() != 2 {
            return Err(invalid());
        }
        *octet = u8::from_str_radix(part, 16).map_err(|_| invalid())?;
    }
    if mac.iter().all(|octet| *octet == 0) || mac[0] & 1 == 1 {
        return Err(invalid());
    }
    Ok(mac)
}

/// True when the local adapter and the target address are on one IPv4 subnet,
/// judged with either side's prefix (they can disagree on a misconfigured LAN).
fn shares_lan(local: &WakeInterface, target: Ipv4Addr, target_prefix: u8) -> bool {
    let Ok(own) = local.ipv4_address.parse::<Ipv4Addr>() else {
        return false;
    };
    [local.prefix_length, target_prefix]
        .into_iter()
        .filter(|prefix| (8..=30).contains(prefix))
        .any(|prefix| network(own, prefix) == network(target, prefix))
}

fn network(address: Ipv4Addr, prefix: u8) -> u32 {
    u32::from(address) & (u32::MAX << (32 - u32::from(prefix)))
}

fn prefix_from_netmask(mask: Ipv4Addr) -> Option<u8> {
    let bits = u32::from(mask);
    let prefix = bits.leading_ones();
    // Reject non-contiguous masks.
    (bits.checked_shl(prefix).unwrap_or(0) == 0)
        .then(|| u8::try_from(prefix).ok())
        .flatten()
}

fn is_overlay_or_virtual(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "utun", "tun", "tap", "wg", "ppp", "zt", "feth", "veth", "vmnet", "vboxnet", "bridge",
        "docker", "awdl", "llw", "lo",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        || [
            "vpn",
            "tailscale",
            "zerotier",
            "wireguard",
            "virtual",
            "vmware",
            "vethernet",
            "hyper-v",
            "loopback",
            "bluetooth",
        ]
        .iter()
        .any(|marker| name.contains(marker))
}

fn wakeable(interfaces: &[NetworkInterface]) -> Vec<WakeInterface> {
    let mut result = Vec::new();
    for interface in interfaces {
        if interface.internal || is_overlay_or_virtual(&interface.name) {
            continue;
        }
        let Some(mac) = interface
            .mac_addr
            .as_deref()
            .and_then(|mac| parse_mac(mac).ok())
        else {
            continue;
        };
        for address in &interface.addr {
            let Addr::V4(v4) = address else { continue };
            let Some(prefix) = v4.netmask.and_then(prefix_from_netmask) else {
                continue;
            };
            if !v4.ip.is_private() || !(8..=30).contains(&prefix) {
                continue;
            }
            let broadcast = Ipv4Addr::from(u32::from(v4.ip) | (u32::MAX >> u32::from(prefix)));
            let entry = WakeInterface {
                mac_address: mac
                    .iter()
                    .map(|octet| format!("{octet:02x}"))
                    .collect::<Vec<_>>()
                    .join(":"),
                ipv4_address: v4.ip.to_string(),
                prefix_length: prefix,
                broadcast_address: broadcast.to_string(),
            };
            if !result.contains(&entry) && result.len() < MAX_INTERFACES {
                result.push(entry);
            }
        }
    }
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn wake(ip: &str, prefix: u8) -> WakeInterface {
        WakeInterface {
            mac_address: "aa:bb:cc:00:11:22".into(),
            ipv4_address: ip.into(),
            prefix_length: prefix,
            broadcast_address: String::new(),
        }
    }

    #[test]
    fn magic_packet_is_six_ff_then_sixteen_macs() {
        let mac = [0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22];
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[..6], &[0xFF; 6]);
        for repeat in packet[6..].chunks(6) {
            assert_eq!(repeat, &mac);
        }
    }

    #[test]
    fn parses_common_mac_formats_and_rejects_unusable_ones() {
        assert_eq!(
            parse_mac("AA-BB-CC-00-11-22").unwrap(),
            [0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22]
        );
        assert!(parse_mac("aa:bb:cc:00:11:22").is_ok());
        for bad in [
            "",
            "aa:bb:cc:00:11",
            "aa:bb:cc:00:11:2",
            "zz:bb:cc:00:11:22",
            "00:00:00:00:00:00",
            "01:00:5e:00:00:01",
        ] {
            assert!(parse_mac(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn detects_a_shared_lan_with_either_prefix() {
        let target = "192.168.1.30".parse().unwrap();
        assert!(shares_lan(&wake("192.168.1.40", 24), target, 24));
        assert!(!shares_lan(&wake("192.168.2.40", 24), target, 24));
        assert!(shares_lan(&wake("192.168.2.40", 16), target, 24));
        assert!(!shares_lan(&wake("10.0.0.2", 8), target, 24));
    }

    #[test]
    fn netmask_converts_to_a_prefix_only_when_contiguous() {
        assert_eq!(
            prefix_from_netmask("255.255.255.0".parse().unwrap()),
            Some(24)
        );
        assert_eq!(
            prefix_from_netmask("255.255.240.0".parse().unwrap()),
            Some(20)
        );
        assert_eq!(prefix_from_netmask("255.0.255.0".parse().unwrap()), None);
    }

    #[test]
    fn keeps_only_physical_private_ipv4_adapters() {
        let ethernet = NetworkInterface::new_afinet(
            "en0",
            "192.168.1.30".parse().unwrap(),
            Some("255.255.255.0".parse().unwrap()),
            None,
            1,
            false,
        )
        .with_mac_addr(Some("AA:BB:CC:00:11:22".into()));
        let vpn = NetworkInterface::new_afinet(
            "utun3",
            "10.8.0.2".parse().unwrap(),
            Some("255.255.255.0".parse().unwrap()),
            None,
            2,
            false,
        )
        .with_mac_addr(Some("02:00:00:00:00:01".into()));
        let public = NetworkInterface::new_afinet(
            "en1",
            "8.8.8.8".parse().unwrap(),
            Some("255.255.255.0".parse().unwrap()),
            None,
            3,
            false,
        )
        .with_mac_addr(Some("02:00:00:00:00:02".into()));
        let no_mac = NetworkInterface::new_afinet(
            "en2",
            "192.168.5.2".parse().unwrap(),
            Some("255.255.255.0".parse().unwrap()),
            None,
            4,
            false,
        );
        let result = wakeable(&[ethernet, vpn, public, no_mac]);
        assert_eq!(
            result,
            vec![WakeInterface {
                mac_address: "aa:bb:cc:00:11:22".into(),
                ipv4_address: "192.168.1.30".into(),
                prefix_length: 24,
                broadcast_address: "192.168.1.255".into(),
            }]
        );
    }

    #[test]
    fn nothing_is_sent_without_a_shared_lan() {
        let mut target = wake("192.168.1.30", 24);
        target.broadcast_address = "192.168.1.255".into();
        let result = send_to(&[wake("10.0.0.2", 24)], &[target]).unwrap();
        assert!(!result.same_lan);
        assert_eq!(result.packets_sent, 0);
    }
}
