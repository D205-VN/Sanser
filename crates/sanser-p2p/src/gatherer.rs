//! Candidate gathering orchestration types.
//!
//! Phase 1 defines the gatherer configuration and result types.
//! Actual async gathering (spawning STUN, interface enumeration, port
//! mapping tasks in parallel) is added in Phase 3.

use crate::candidate::{
    CandidateSet, CandidateType, MappingProtocol, P2pCandidate, TransportProtocol,
    candidate_priority,
};
use crate::error::P2pError;
use crate::interface::{InterfaceCost, InterfaceFilter, enumerate_interfaces, filter_interface};
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

const fn interface_preference(cost: InterfaceCost) -> u16 {
    match cost {
        InterfaceCost::Low => u16::MAX,
        InterfaceCost::Medium => 32_768,
        InterfaceCost::High => 16_384,
    }
}

fn host_candidate(
    interface: &crate::interface::NetworkInterface,
    port: u16,
) -> Result<Option<P2pCandidate>, P2pError> {
    let candidate_type = if is_public_ipv6(interface.address) {
        CandidateType::Ipv6Global
    } else {
        CandidateType::Host
    };
    let clean_ip = interface
        .address
        .to_string()
        .replace('.', "-")
        .replace(':', "-");
    let priority = candidate_priority(
        candidate_type,
        MappingProtocol::None,
        interface_preference(interface.cost),
    )
    .map_err(|error| P2pError::Internal {
        reason: format!("failed to compute candidate priority: {error}"),
    })?;
    let candidate = P2pCandidate {
        id: format!("host-{clean_ip}-{port}"),
        candidate_type,
        address: interface.address,
        port,
        protocol: TransportProtocol::Udp,
        interface_index: Some(interface.index),
        mapping_protocol: MappingProtocol::None,
        priority,
        foundation: format!("host-{clean_ip}"),
    };
    Ok(candidate.validate().is_ok().then_some(candidate))
}

fn server_reflexive_candidate(
    binding: &crate::stun::StunBinding,
    interface_index: Option<u32>,
    preference: u16,
) -> Result<Option<P2pCandidate>, P2pError> {
    let clean_ip = binding
        .mapped_address
        .to_string()
        .replace('.', "-")
        .replace(':', "-");
    let priority = candidate_priority(
        CandidateType::ServerReflexive,
        MappingProtocol::None,
        preference,
    )
    .map_err(|error| P2pError::Internal {
        reason: format!("failed to compute server-reflexive priority: {error}"),
    })?;
    let candidate = P2pCandidate {
        id: format!("srflx-{clean_ip}-{}", binding.mapped_port),
        candidate_type: CandidateType::ServerReflexive,
        address: binding.mapped_address,
        port: binding.mapped_port,
        protocol: TransportProtocol::Udp,
        interface_index,
        mapping_protocol: MappingProtocol::None,
        priority,
        foundation: format!("srflx-{clean_ip}"),
    };
    Ok(candidate.validate().is_ok().then_some(candidate))
}

fn port_mapped_candidate(
    external: SocketAddr,
    interface_index: Option<u32>,
    preference: u16,
) -> Result<Option<P2pCandidate>, P2pError> {
    let clean_ip = external
        .ip()
        .to_string()
        .replace('.', "-")
        .replace(':', "-");
    let priority = candidate_priority(CandidateType::PortMapped, MappingProtocol::Upnp, preference)
        .map_err(|error| P2pError::Internal {
            reason: format!("failed to compute UPnP candidate priority: {error}"),
        })?;
    let candidate = P2pCandidate {
        id: format!("upnp-{clean_ip}-{}", external.port()),
        candidate_type: CandidateType::PortMapped,
        address: external.ip(),
        port: external.port(),
        protocol: TransportProtocol::Udp,
        interface_index,
        mapping_protocol: MappingProtocol::Upnp,
        priority,
        foundation: format!("upnp-{clean_ip}"),
    };
    Ok(candidate.validate().is_ok().then_some(candidate))
}

const MAX_SIGNAL_CANDIDATES: usize = 16;

async fn resolve_stun_server(url: &str, ipv4: bool) -> Option<SocketAddr> {
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
        addrs.find(|address| address.is_ipv4() == ipv4)
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

        let preference = interface_preference(iface.cost);
        let Some(candidate) = host_candidate(&iface, local_addr.port())? else {
            continue;
        };

        let _ = tx
            .send(GatheringEvent::CandidateFound(candidate.clone()))
            .await;
        result.candidates.push(candidate);
        active_sockets.push((socket, iface, preference));
    }

    if active_sockets.is_empty() {
        return Err(P2pError::NoCandidateGathered);
    }

    // 4. Resolve STUN servers
    let mut resolved_stun_servers = Vec::new();
    for url in &config.stun_servers {
        if let Some(addr) = resolve_stun_server(url, true).await {
            resolved_stun_servers.push(addr);
        }
        if config.ipv6_enabled
            && let Some(addr) = resolve_stun_server(url, false).await
        {
            resolved_stun_servers.push(addr);
        }
    }

    // 5. Query STUN servers in parallel (one task per local socket)
    let (cands_tx, mut cands_rx) = mpsc::channel(64);
    let mut stun_query_count = 0;

    for (socket, iface, preference) in active_sockets {
        let socket_is_ipv4 = socket.local_addr().is_ok_and(|address| address.is_ipv4());
        let stun_addrs = resolved_stun_servers
            .iter()
            .copied()
            .filter(|address| address.is_ipv4() == socket_is_ipv4)
            .collect::<Vec<_>>();
        if stun_addrs.is_empty() {
            continue;
        }
        let cands_tx = cands_tx.clone();
        stun_query_count += 1;

        tokio::spawn(async move {
            for server_addr in stun_addrs {
                match crate::stun::query_stun(&socket, server_addr, Duration::from_millis(500))
                    .await
                {
                    Ok(binding) => {
                        if let Ok(Some(candidate)) =
                            server_reflexive_candidate(&binding, Some(iface.index), preference)
                        {
                            let _ = cands_tx.send(candidate).await;
                            break;
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
        let _ = tx_pm
            .send(GatheringEvent::PortMappingComplete { success: false })
            .await;
    });

    drop(cands_tx); // Close coordinate sender so rx terminates when tasks finish

    // 6. Coordinate STUN result collection with the overall timeout
    let total_timeout = config.total_timeout;
    let gather_loop = async {
        let mut gathered = Vec::new();
        while let Some(candidate) = cands_rx.recv().await {
            let _ = tx
                .send(GatheringEvent::CandidateFound(candidate.clone()))
                .await;
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
    let _ = tx
        .send(GatheringEvent::StunComplete {
            success: stun_success,
        })
        .await;

    result.duration_ms = start_time.elapsed().as_millis() as u64;
    let _ = tx.send(GatheringEvent::Complete(result.clone())).await;

    Ok(result)
}

/// Gathers same-family host and server-reflexive candidates on an already-bound UDP
/// socket. The caller retains ownership of that exact socket for connectivity
/// checks, preserving the NAT mapping created by STUN.
///
/// # Errors
///
/// Returns an error when the socket is not usable, no matching local
/// interface can be advertised, or candidate construction fails.
pub async fn gather_candidates_on_socket(
    config: GathererConfig,
    socket: &UdpSocket,
    tx: mpsc::Sender<GatheringEvent>,
) -> Result<GatheringResult, P2pError> {
    if config.generation == 0 {
        return Err(P2pError::Internal {
            reason: "candidate generation must be non-zero".into(),
        });
    }
    let started = std::time::Instant::now();
    let local = socket
        .local_addr()
        .map_err(|error| P2pError::SocketBindFailed {
            reason: error.to_string(),
        })?;
    if local.port() == 0 {
        return Err(P2pError::SocketBindFailed {
            reason: "P2P v2 requires a bound UDP socket".into(),
        });
    }

    let socket_is_ipv4 = local.is_ipv4();

    let mut interfaces = enumerate_interfaces()?
        .into_iter()
        .filter(|interface| {
            interface.address.is_ipv4() == socket_is_ipv4
                && (socket_is_ipv4 || is_public_ipv6(interface.address))
                && matches!(filter_interface(interface), InterfaceFilter::Accept)
        })
        .collect::<Vec<_>>();
    interfaces.sort_by_key(|interface| (interface.cost, interface.index, interface.address));
    if interfaces.is_empty() {
        return Err(P2pError::NoLocalInterface);
    }

    let mut result = GatheringResult {
        generation: config.generation,
        candidates: Vec::new(),
        stun_succeeded: false,
        port_mapping_succeeded: false,
        duration_ms: 0,
    };
    let mut candidate_set = CandidateSet::new(512).map_err(|error| P2pError::Internal {
        reason: format!("failed to initialize the candidate set: {error}"),
    })?;
    for interface in &interfaces {
        if let Some(candidate) = host_candidate(interface, local.port())? {
            candidate_set
                .insert(candidate)
                .map_err(|error| P2pError::Internal {
                    reason: format!("failed to collect a host candidate: {error}"),
                })?;
        }
    }
    if candidate_set.is_empty() {
        return Err(P2pError::NoCandidateGathered);
    }

    let stun_servers = async {
        for url in &config.stun_servers {
            let Some(server) = resolve_stun_server(url, socket_is_ipv4).await else {
                continue;
            };
            if let Ok(binding) =
                crate::stun::query_stun(socket, server, Duration::from_millis(750)).await
            {
                let preferred = &interfaces[0];
                if let Some(candidate) = server_reflexive_candidate(
                    &binding,
                    Some(preferred.index),
                    interface_preference(preferred.cost),
                )? {
                    return Ok::<Option<P2pCandidate>, P2pError>(Some(candidate));
                }
            }
        }
        Ok(None)
    };

    let upnp_mapping = async {
        if !config.port_mapping_enabled || !socket_is_ipv4 {
            return None;
        }
        let search_options = igd_next::SearchOptions {
            timeout: Some(Duration::from_millis(1_200)),
            single_search_timeout: Some(Duration::from_millis(350)),
            ..Default::default()
        };
        let gateway = igd_next::aio::tokio::search_gateway(search_options)
            .await
            .ok()?;
        let route_probe = UdpSocket::bind("0.0.0.0:0").await.ok()?;
        route_probe.connect(gateway.addr).await.ok()?;
        let internal_ip = route_probe.local_addr().ok()?.ip();
        let internal = SocketAddr::new(internal_ip, local.port());
        let external = gateway
            .get_any_address(
                igd_next::PortMappingProtocol::UDP,
                internal,
                7_200,
                "Sanser P2P",
            )
            .await
            .ok()?;
        Some((external, internal_ip))
    };

    let (stun_result, upnp_result) = tokio::join!(
        tokio::time::timeout(config.total_timeout, stun_servers),
        tokio::time::timeout(Duration::from_millis(1_500), upnp_mapping)
    );
    if let Ok(Ok(Some(candidate))) = stun_result {
        result.stun_succeeded = true;
        candidate_set
            .insert(candidate)
            .map_err(|error| P2pError::Internal {
                reason: format!("failed to collect a STUN candidate: {error}"),
            })?;
    }
    if let Ok(Some((external, internal_ip))) = upnp_result
        && let Some(mapped_interface) = interfaces.iter().find(|item| item.address == internal_ip)
        && let Some(candidate) = port_mapped_candidate(
            external,
            Some(mapped_interface.index),
            interface_preference(mapped_interface.cost),
        )?
    {
        result.port_mapping_succeeded = true;
        candidate_set
            .insert(candidate)
            .map_err(|error| P2pError::Internal {
                reason: format!("failed to collect a UPnP candidate: {error}"),
            })?;
    }
    let ordered = candidate_set.ordered_by_priority();
    let external = ordered
        .iter()
        .find(|candidate| {
            matches!(
                candidate.candidate_type,
                CandidateType::ServerReflexive | CandidateType::PortMapped
            )
        })
        .map(|candidate| (*candidate).clone());
    result.candidates = ordered
        .into_iter()
        .take(MAX_SIGNAL_CANDIDATES)
        .cloned()
        .collect();
    if let Some(external) = external
        && !result
            .candidates
            .iter()
            .any(|candidate| candidate.endpoint() == external.endpoint())
    {
        if result.candidates.len() == MAX_SIGNAL_CANDIDATES {
            result.candidates.pop();
        }
        result.candidates.push(external);
        result.candidates.sort_unstable_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    for candidate in &result.candidates {
        let _ = tx
            .send(GatheringEvent::CandidateFound(candidate.clone()))
            .await;
    }
    let _ = tx
        .send(GatheringEvent::StunComplete {
            success: result.stun_succeeded,
        })
        .await;
    let _ = tx
        .send(GatheringEvent::PortMappingComplete {
            success: result.port_mapping_succeeded,
        })
        .await;
    result.duration_ms = started.elapsed().as_millis() as u64;
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
        let handle = tokio::spawn(async move { gather_candidates(config, tx).await });

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        let result = handle.await.unwrap();
        assert!(result.is_ok(), "gathering failed: {:?}", result.err());
        let res = result.unwrap();

        assert!(
            res.candidates.len() >= 1,
            "should find at least one host candidate"
        );
        assert!(
            !res.stun_succeeded,
            "STUN should fail with invalid STUN server"
        );

        // Verify events were emitted
        assert!(
            events
                .iter()
                .any(|e| matches!(e, GatheringEvent::CandidateFound(_)))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, GatheringEvent::Complete(_)))
        );
    }

    #[tokio::test]
    async fn single_socket_gathering_preserves_the_bound_media_port() {
        let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        let port = socket.local_addr().unwrap().port();
        let config = GathererConfig {
            stun_servers: vec!["127.0.0.1:9".to_owned()],
            total_timeout: Duration::from_millis(30),
            port_mapping_enabled: false,
            ipv6_enabled: false,
            generation: 1,
        };
        let (tx, mut rx) = mpsc::channel(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = gather_candidates_on_socket(config, &socket, tx)
            .await
            .unwrap();
        drain.await.unwrap();

        assert!(!result.candidates.is_empty());
        assert!(
            result
                .candidates
                .iter()
                .all(|candidate| candidate.port == port)
        );
        assert_eq!(socket.local_addr().unwrap().port(), port);
    }
}
