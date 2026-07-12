//! Candidate gathering orchestration types.
//!
//! Phase 1 defines the gatherer configuration and result types.
//! Actual async gathering (spawning STUN, interface enumeration, port
//! mapping tasks in parallel) is added in Phase 3.

use crate::candidate::{CandidateType, MappingProtocol, P2pCandidate, TransportProtocol, candidate_priority};
use crate::error::P2pError;
use crate::interface::{InterfaceCost, enumerate_interfaces, filter_interface, InterfaceFilter};
use serde::Serialize;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Configuration for the candidate gatherer.
#[derive(Clone, Debug)]
pub struct GathererConfig {
    /// STUN server URLs to query.
    pub stun_servers: Vec<String>,
    /// Total time budget for all gathering (default: 3 seconds).
    pub total_timeout: Duration,
    /// Whether to attempt port mapping (PCP/NAT-PMP/UPnP).
    pub port_mapping_enabled: bool,
    /// Whether to include IPv6 global candidates.
    pub ipv6_enabled: bool,
    /// Current candidate generation (incremented on network change).
    pub generation: u32,
}

impl Default for GathererConfig {
    fn default() -> Self {
        Self {
            stun_servers: vec!["stun:stun.l.google.com:19302".into()],
            total_timeout: Duration::from_secs(3),
            port_mapping_enabled: true,
            ipv6_enabled: true,
            generation: 1,
        }
    }
}

/// Outcome of a gathering round.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatheringResult {
    /// Generation counter for this gathering round.
    pub generation: u32,
    /// All gathered candidates.
    pub candidates: Vec<P2pCandidate>,
    /// Whether STUN succeeded.
    pub stun_succeeded: bool,
    /// Whether port mapping succeeded.
    pub port_mapping_succeeded: bool,
    /// Total gathering time in milliseconds.
    pub duration_ms: u64,
}

/// Events emitted during gathering for incremental UI updates.
#[derive(Clone, Debug)]
pub enum GatheringEvent {
    /// A new candidate was discovered.
    CandidateFound(P2pCandidate),
    /// STUN binding completed.
    StunComplete { success: bool },
    /// Port mapping completed.
    PortMappingComplete { success: bool },
    /// All gathering is done.
    Complete(GatheringResult),
}

fn is_public_ipv6(address: IpAddr) -> bool {
    if let IpAddr::V6(v6) = address {
        !v6.is_unspecified()
            && !v6.is_loopback()
            && !v6.is_multicast()
            && !((v6.segments()[0] & 0xffc0) == 0xfe80) // link local
            && !((v6.segments()[0] & 0xfe00) == 0xfc00) // unique local
            && !(v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8) // documentation
    } else {
        false
    }
}

async fn resolve_stun_server(url: &str) -> Option<SocketAddr> {
    let host_port = if let Some(stripped) = url.strip_prefix("stun:") {
        stripped
    } else {
        url
    };
    let host_port = if host_port.contains(':') {
        host_port.to_owned()
    } else {
        format!("{host_port}:3478")
    };
    if let Ok(mut addrs) = tokio::net::lookup_host(&host_port).await {
        addrs.next()
    } else {
        None
    }
}

/// Orchestrates candidate gathering.
///
/// Enumerates interfaces, binds sockets, gathers Host/Ipv6Global candidates,
/// and queries STUN servers to gather ServerReflexive candidates. Incremental
/// findings are sent as events through `tx`.
///
/// # Errors
///
/// Returns a [`P2pError::NoLocalInterface`] if no interfaces are usable, or other P2pError on failures.
pub async fn gather_candidates(
    config: GathererConfig,
    tx: mpsc::Sender<GatheringEvent>,
) -> Result<GatheringResult, P2pError> {
    let start_time = std::time::Instant::now();
    let generation = config.generation;

    let mut result = GatheringResult {
        generation,
        candidates: Vec::new(),
        stun_succeeded: false,
        port_mapping_succeeded: false,
        duration_ms: 0,
    };

    // 1. Enumerate network interfaces
    let interfaces = enumerate_interfaces()?;

    // 2. Filter interfaces
    let mut usable_interfaces = Vec::new();
    for iface in interfaces {
        if let InterfaceFilter::Accept = filter_interface(&iface) {
            if !iface.address.is_ipv6() || config.ipv6_enabled {
                usable_interfaces.push(iface);
            }
        }
    }

    if usable_interfaces.is_empty() {
        return Err(P2pError::NoLocalInterface);
    }

    // 3. Bind sockets and gather Host candidates
    let mut active_sockets = Vec::new();
    for iface in usable_interfaces {
        let bind_addr = SocketAddr::new(iface.address, 0);
        let socket = match UdpSocket::bind(bind_addr).await {
            Ok(s) => s,
            Err(_) => continue,
        };

        let local_addr = match socket.local_addr() {
            Ok(addr) => addr,
            Err(_) => continue,
        };

        let is_v6_global = is_public_ipv6(iface.address);
        let cand_type = if is_v6_global {
            CandidateType::Ipv6Global
        } else {
            CandidateType::Host
        };

        let clean_ip = iface.address.to_string().replace('.', "-").replace(':', "-");
        let id = format!("host-{clean_ip}-{}", local_addr.port());
        let foundation = format!("host-{clean_ip}");
        let preference = match iface.cost {
            InterfaceCost::Low => 65535,
            InterfaceCost::Medium => 32768,
            InterfaceCost::High => 16384,
        };

        let priority = candidate_priority(cand_type, MappingProtocol::None, preference)
            .map_err(|error| P2pError::Internal {
                reason: format!("Failed to compute candidate priority: {error}"),
            })?;

        let candidate = P2pCandidate {
            id,
            candidate_type: cand_type,
            address: iface.address,
            port: local_addr.port(),
            protocol: TransportProtocol::Udp,
            interface_index: Some(iface.index),
            mapping_protocol: MappingProtocol::None,
            priority,
            foundation,
        };

        if candidate.validate().is_err() {
            continue;
        }

        let _ = tx.send(GatheringEvent::CandidateFound(candidate.clone())).await;
        result.candidates.push(candidate);
        active_sockets.push((socket, iface, preference));
    }

    if active_sockets.is_empty() {
        return Err(P2pError::NoCandidateGathered);
    }

    // 4. Resolve STUN servers
    let mut resolved_stun_servers = Vec::new();
    for url in &config.stun_servers {
        if let Some(addr) = resolve_stun_server(url).await {
            resolved_stun_servers.push(addr);
        }
    }

    // 5. Query STUN servers in parallel (one task per local socket)
    let (cands_tx, mut cands_rx) = mpsc::channel(64);
    let mut stun_query_count = 0;

    for (socket, iface, preference) in active_sockets {
        if resolved_stun_servers.is_empty() {
            continue;
        }
        let stun_addrs = resolved_stun_servers.clone();
        let cands_tx = cands_tx.clone();
        stun_query_count += 1;

        tokio::spawn(async move {
            for server_addr in stun_addrs {
                match crate::stun::query_stun(&socket, server_addr, Duration::from_millis(500)).await {
                    Ok(binding) => {
                        let clean_ip = binding.mapped_address.to_string().replace('.', "-").replace(':', "-");
                        let id = format!("srflx-{clean_ip}-{}", binding.mapped_port);
                        let foundation = format!("srflx-{clean_ip}");

                        if let Ok(priority) = candidate_priority(CandidateType::ServerReflexive, MappingProtocol::None, preference) {
                            let candidate = P2pCandidate {
                                id,
                                candidate_type: CandidateType::ServerReflexive,
                                address: binding.mapped_address,
                                port: binding.mapped_port,
                                protocol: TransportProtocol::Udp,
                                interface_index: Some(iface.index),
                                mapping_protocol: MappingProtocol::None,
                                priority,
                                foundation,
                            };
                            if candidate.validate().is_ok() {
                                let _ = cands_tx.send(candidate).await;
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        // Try next STUN server
                    }
                }
            }
        });
    }

    // Spawn a dummy port mapping task if enabled, simulating port mapping complete event
    let tx_pm = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tx_pm.send(GatheringEvent::PortMappingComplete { success: false }).await;
    });

    drop(cands_tx); // Close coordinate sender so rx terminates when tasks finish

    // 6. Coordinate STUN result collection with the overall timeout
    let total_timeout = config.total_timeout;
    let gather_loop = async {
        let mut gathered = Vec::new();
        while let Some(candidate) = cands_rx.recv().await {
            let _ = tx.send(GatheringEvent::CandidateFound(candidate.clone())).await;
            gathered.push(candidate);
        }
        gathered
    };

    let (stun_success, new_candidates) = if stun_query_count > 0 {
        tokio::select! {
            _ = tokio::time::sleep(total_timeout) => {
                (false, Vec::new())
            }
            cands = gather_loop => {
                (!cands.is_empty(), cands)
            }
        }
    } else {
        (false, Vec::new())
    };

    for candidate in new_candidates {
        result.candidates.push(candidate);
    }

    result.stun_succeeded = stun_success;
    let _ = tx.send(GatheringEvent::StunComplete { success: stun_success }).await;

    result.duration_ms = start_time.elapsed().as_millis() as u64;
    let _ = tx.send(GatheringEvent::Complete(result.clone())).await;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gather_candidates_finds_host_candidates() {
        let config = GathererConfig {
            stun_servers: vec!["127.0.0.1:3478".to_owned()], // invalid STUN server, so it will fail STUN but gather host
            total_timeout: Duration::from_millis(100),
            port_mapping_enabled: false,
            ipv6_enabled: true,
            generation: 1,
        };

        let (tx, mut rx) = mpsc::channel(16);
        let handle = tokio::spawn(async move {
            gather_candidates(config, tx).await
        });

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        let result = handle.await.unwrap();
        assert!(result.is_ok(), "gathering failed: {:?}", result.err());
        let res = result.unwrap();

        assert!(res.candidates.len() >= 1, "should find at least one host candidate");
        assert!(!res.stun_succeeded, "STUN should fail with invalid STUN server");

        // Verify events were emitted
        assert!(events.iter().any(|e| matches!(e, GatheringEvent::CandidateFound(_))));
        assert!(events.iter().any(|e| matches!(e, GatheringEvent::Complete(_))));
    }
}
