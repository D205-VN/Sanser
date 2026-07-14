// Tauri extracts owned command arguments and state guards through its command
// macro. References here are not valid IPC command arguments even though the
// implementation itself only borrows them.
#![allow(
    clippy::needless_pass_by_value,
    clippy::unwrap_used,
    clippy::unnecessary_wraps,
    clippy::manual_let_else,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::single_match,
    clippy::uninlined_format_args
)]

use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};

use tauri::{AppHandle, State};
use url::{Host, Url};

use crate::{
    engine::{EngineManager, probe_sidecar},
    error::DesktopError,
    models::{
        Capability, DiagnosticsExport, EngineKind, EngineStatus, LaunchEngineRequest,
        PROTOCOL_VERSION, Preferences, RuntimeCapabilities, RuntimeStatus, SANSER_VERSION,
    },
    storage,
};

fn platform_label() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn is_peer_route(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => {
            !address.is_unspecified()
                && !address.is_loopback()
                && !address.is_multicast()
                && !address.is_link_local()
                && address.octets() != [255, 255, 255, 255]
        }
        IpAddr::V6(address) => {
            !(address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || address.is_unicast_link_local()
                || address.is_unique_local()
                || (address.segments()[0] == 0x2001 && address.segments()[1] == 0x0db8))
        }
    }
}

fn validated_route_target(server_url: &str) -> Result<(String, u16), DesktopError> {
    let url = Url::parse(server_url.trim())
        .map_err(|_| DesktopError::InvalidRequest("server URL is invalid".into()))?;
    let host = url
        .host()
        .ok_or_else(|| DesktopError::InvalidRequest("server URL has no host".into()))?;
    let loopback = match host {
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(DesktopError::InvalidRequest(
            "server URL must use HTTPS, except for loopback development".into(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DesktopError::InvalidRequest(
            "server URL contains unsupported credentials or metadata".into(),
        ));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| DesktopError::InvalidRequest("server URL has no usable port".into()))?;
    Ok((
        url.host_str()
            .ok_or_else(|| DesktopError::InvalidRequest("server URL has no host".into()))?
            .to_owned(),
        port,
    ))
}

fn discover_local_route_address(server_url: &str) -> Result<Option<String>, DesktopError> {
    let (host, port) = validated_route_target(server_url)?;
    let targets = (host.as_str(), port)
        .to_socket_addrs()
        .map_or_else(|_| Vec::new(), Iterator::collect::<Vec<SocketAddr>>);
    for target in targets {
        let bind_address = if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let Ok(socket) = UdpSocket::bind(bind_address) else {
            continue;
        };
        if socket.connect(target).is_err() {
            continue;
        }
        let Ok(local) = socket.local_addr() else {
            continue;
        };
        if is_peer_route(local.ip()) {
            return Ok(Some(local.ip().to_string()));
        }
    }
    Ok(None)
}

#[tauri::command]
pub fn get_runtime_status(
    app: AppHandle,
    engines: State<'_, EngineManager>,
) -> Result<RuntimeStatus, DesktopError> {
    let host = engines.status(&app, EngineKind::Host)?;
    let client = engines.status(&app, EngineKind::Client)?;
    let local_server = engines.status(&app, EngineKind::LocalServer)?;
    let host_probe = probe_sidecar(&app, EngineKind::Host);
    let client_probe = probe_sidecar(&app, EngineKind::Client);

    let host_capability = if !cfg!(target_os = "windows") {
        Capability::unavailable("Windows host engine is only available on Windows")
    } else if host.installed && host_probe.compatible {
        Capability::available()
    } else if host.installed {
        Capability::unavailable(
            host_probe
                .reason
                .clone()
                .unwrap_or_else(|| "Windows host sidecar capability probe failed".into()),
        )
    } else {
        Capability::unavailable("sanser-host-windows is not bundled")
    };
    let client_capability = if !cfg!(target_os = "macos") {
        Capability::unavailable("macOS client engine is only available on macOS")
    } else if client.installed && client_probe.compatible {
        Capability::available()
    } else if client.installed {
        Capability::unavailable(
            client_probe
                .reason
                .clone()
                .unwrap_or_else(|| "macOS client sidecar capability probe failed".into()),
        )
    } else {
        Capability::unavailable("sanser-client-macos is not bundled")
    };
    let platform_probe = if cfg!(target_os = "windows") {
        &host_probe
    } else {
        &client_probe
    };

    Ok(RuntimeStatus {
        platform: platform_label(),
        version: SANSER_VERSION,
        protocol_version: PROTOCOL_VERSION,
        capabilities: RuntimeCapabilities {
            desktop_shell: Capability::available(),
            secure_storage: Capability::available(),
            host_engine: host_capability,
            client_engine: client_capability,
            local_server: Capability::unavailable(
                "Local database/server mode is disabled; desktop uses the deployed PostgreSQL/Neon API",
            ),
            local_discovery: Capability::planned(
                "Signed mDNS/UDP discovery backend is not installed",
            ),
            web_rtc: Capability::planned("libdatachannel transport is not linked yet"),
            native_direct: if platform_probe.compatible && platform_probe.native_direct {
                Capability::available()
            } else {
                Capability::unavailable(platform_probe.reason.clone().unwrap_or_else(|| {
                    "Authenticated native direct transport is not available".into()
                }))
            },
            native_snv2: if platform_probe.compatible && platform_probe.native_snv2 {
                Capability::available()
            } else {
                Capability::unavailable(
                    "Native sidecar uses the authenticated legacy wire; shared SNV2 migration is not verified",
                )
            },
            gamepad: Capability::planned("Native controller state transport is not linked yet"),
            clipboard: Capability::planned(
                "Permission-gated clipboard transport is not linked yet",
            ),
            p2p_v2: Capability::available(),
        },
        engines: vec![host, client, local_server],
    })
}

#[tauri::command]
pub async fn load_preferences(app: AppHandle) -> Result<Option<Preferences>, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::load_preferences(&app))
        .await
        .map_err(|error| DesktopError::Storage(format!("preferences task failed: {error}")))?
}

#[tauri::command]
pub async fn save_preferences(
    app: AppHandle,
    preferences: Preferences,
) -> Result<(), DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::save_preferences(&app, &preferences))
        .await
        .map_err(|error| DesktopError::Storage(format!("preferences task failed: {error}")))?
}

#[tauri::command]
pub async fn secure_get(key: String) -> Result<Option<String>, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::secure_get(&key))
        .await
        .map_err(|_| DesktopError::SecureStorage)?
}

#[tauri::command]
pub async fn secure_set(key: String, value: String) -> Result<(), DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::secure_set(&key, value))
        .await
        .map_err(|_| DesktopError::SecureStorage)?
}

#[tauri::command]
pub async fn secure_delete(key: String) -> Result<(), DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::secure_delete(&key))
        .await
        .map_err(|_| DesktopError::SecureStorage)?
}

/// Returns the local interface address selected by the OS for the configured
/// API route. UDP `connect` performs no network I/O, and loopback/link-local
/// addresses are never advertised to another device.
#[tauri::command]
pub async fn get_local_route_address(server_url: String) -> Result<Option<String>, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || discover_local_route_address(&server_url))
        .await
        .map_err(|error| DesktopError::Process(format!("route discovery task failed: {error}")))?
}

#[tauri::command]
pub fn launch_engine(
    app: AppHandle,
    engines: State<'_, EngineManager>,
    p2p: State<'_, P2pSessionManager>,
    relay: State<'_, crate::relay::RelayManager>,
    request: LaunchEngineRequest,
) -> Result<(), DesktopError> {
    let reserved_socket = if request.relay {
        relay.verify_launch(&request)?;
        None
    } else {
        relay.stop_for_engine(request.kind);
        p2p.take_selected_socket(&request)?
    };
    let result = engines.launch(&app, &request, reserved_socket);
    if result.is_err() && request.relay {
        relay.stop_for_engine(request.kind);
    }
    result
}

#[tauri::command]
pub fn stop_engine(
    engines: State<'_, EngineManager>,
    relay: State<'_, crate::relay::RelayManager>,
    kind: EngineKind,
) -> Result<(), DesktopError> {
    let result = engines.stop(kind);
    relay.stop_for_engine(kind);
    result
}

#[tauri::command]
pub fn get_engine_status(
    app: AppHandle,
    engines: State<'_, EngineManager>,
    relay: State<'_, crate::relay::RelayManager>,
    kind: EngineKind,
) -> Result<EngineStatus, DesktopError> {
    let mut status = engines.status(&app, kind)?;
    if let Some(failure) = relay.take_failure(kind) {
        let _ = engines.stop(kind);
        status.running = false;
        status.process_id = None;
        status.last_error = Some(failure);
    }
    Ok(status)
}

#[tauri::command]
pub async fn export_diagnostics(
    app: AppHandle,
    contents: String,
) -> Result<DiagnosticsExport, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::export_diagnostics(&app, &contents))
        .await
        .map_err(|error| DesktopError::Storage(format!("diagnostics task failed: {error}")))?
}

use sanser_p2p::{
    CandidatePair, GathererConfig, P2pCandidate, PairState, check_connectivity,
    gather_candidates_on_socket, make_pair_id, pair_priority,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, Socket, Type};
use std::time::Duration;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use zeroize::Zeroize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum P2pTransportPhase {
    Gathering,
    Gathered,
    Selected,
}

struct P2pTransport {
    session_id: uuid::Uuid,
    attempt_id: uuid::Uuid,
    sockets: Vec<P2pTransportSocket>,
    selected_socket: Option<usize>,
    phase: P2pTransportPhase,
}

struct P2pTransportSocket {
    socket: Arc<tokio::net::UdpSocket>,
    candidates: Vec<P2pCandidate>,
}

#[derive(Default)]
pub struct P2pSessionManager {
    transport: Mutex<Option<P2pTransport>>,
}

impl P2pSessionManager {
    fn lock_transport(&self) -> Result<std::sync::MutexGuard<'_, Option<P2pTransport>>, String> {
        self.transport
            .lock()
            .map_err(|_| "P2P transport state is unavailable".to_owned())
    }

    fn clear_if_current(&self, session_id: uuid::Uuid, attempt_id: uuid::Uuid) {
        if let Ok(mut transport) = self.transport.lock()
            && transport.as_ref().is_some_and(|current| {
                current.session_id == session_id && current.attempt_id == attempt_id
            })
        {
            *transport = None;
        }
    }

    fn take_selected_socket(
        &self,
        request: &LaunchEngineRequest,
    ) -> Result<Option<Arc<tokio::net::UdpSocket>>, DesktopError> {
        let Some(session_id) = request.session_id.as_deref() else {
            return Ok(None);
        };
        let session_id = uuid::Uuid::parse_str(session_id)
            .map_err(|_| DesktopError::InvalidRequest("engine sessionId must be a UUID".into()))?;
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| DesktopError::Process("P2P transport state is unavailable".into()))?;
        let Some(current) = transport.as_ref() else {
            if request.udp_connect.is_some() || request.udp_bind_port.is_some() {
                return Err(DesktopError::Process(
                    "the selected P2P socket is no longer reserved".into(),
                ));
            }
            return Ok(None);
        };
        if current.session_id != session_id {
            return Err(DesktopError::Process(
                "selected P2P socket belongs to another session".into(),
            ));
        }
        if current.phase != P2pTransportPhase::Selected {
            return Err(DesktopError::Process(
                "P2P connectivity has not selected a route".into(),
            ));
        }
        let selected_socket = current
            .selected_socket
            .and_then(|index| current.sockets.get(index))
            .ok_or_else(|| DesktopError::Process("selected P2P socket is unavailable".into()))?;
        let reserved_port = selected_socket
            .socket
            .local_addr()
            .map_err(|error| DesktopError::Process(error.to_string()))?
            .port();
        let requested_port = match request.kind {
            EngineKind::Host => request.udp_bind_port,
            EngineKind::Client => request.port,
            EngineKind::LocalServer => None,
        };
        if requested_port != Some(reserved_port) {
            return Err(DesktopError::InvalidRequest(
                "native engine port does not match the selected P2P socket".into(),
            ));
        }
        Ok(transport.take().and_then(|mut selected| {
            let index = selected.selected_socket?;
            (index < selected.sockets.len()).then(|| selected.sockets.swap_remove(index).socket)
        }))
    }
}

#[tauri::command]
pub fn p2p_stop(manager: State<'_, P2pSessionManager>, attempt_id: String) -> Result<(), String> {
    let attempt_id = uuid::Uuid::parse_str(attempt_id.trim())
        .map_err(|_| "P2P attemptId must be a UUID".to_owned())?;
    let mut current = manager.lock_transport()?;
    if current
        .as_ref()
        .is_some_and(|transport| transport.attempt_id == attempt_id)
    {
        *current = None;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pGatherResult {
    pub candidates: Vec<P2pCandidate>,
    pub local_port: u16,
    pub stun_succeeded: bool,
    pub port_mapping_succeeded: bool,
    pub ipv6_available: bool,
    pub public_endpoint: Option<String>,
    pub duration_ms: u64,
}

#[tauri::command]
pub async fn p2p_gather(
    manager: State<'_, P2pSessionManager>,
    session_id: String,
    attempt_id: String,
    stun_servers: Vec<String>,
    preferred_local_port: Option<u16>,
) -> Result<P2pGatherResult, String> {
    let session_id = uuid::Uuid::parse_str(session_id.trim())
        .map_err(|_| "P2P sessionId must be a UUID".to_owned())?;
    let attempt_id = uuid::Uuid::parse_str(attempt_id.trim())
        .map_err(|_| "P2P attemptId must be a UUID".to_owned())?;
    if stun_servers.len() > 3 {
        return Err("At most three STUN servers may be used per attempt".into());
    }
    let stun_servers = stun_servers
        .iter()
        .map(|server| validate_stun_server(server))
        .collect::<Result<Vec<_>, _>>()?;
    if preferred_local_port.is_some_and(|port| port < 1_024) {
        return Err("Preferred host UDP port must be between 1024 and 65535".into());
    }
    let bind_port = preferred_local_port.unwrap_or(0);
    let ipv4_socket = Arc::new(
        tokio::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, bind_port))
            .await
            .map_err(|error| {
                if preferred_local_port.is_some() {
                    format!("Unable to reserve fixed host UDP port {bind_port}: {error}")
                } else {
                    format!("Unable to reserve the P2P UDP socket: {error}")
                }
            })?,
    );
    let local_port = ipv4_socket
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let ipv6_socket = bind_ipv6_socket(local_port).ok().map(Arc::new);
    {
        let mut current = manager.lock_transport()?;
        let mut sockets = vec![P2pTransportSocket {
            socket: Arc::clone(&ipv4_socket),
            candidates: Vec::new(),
        }];
        if let Some(socket) = &ipv6_socket {
            sockets.push(P2pTransportSocket {
                socket: Arc::clone(socket),
                candidates: Vec::new(),
            });
        }
        *current = Some(P2pTransport {
            session_id,
            attempt_id,
            sockets,
            selected_socket: None,
            phase: P2pTransportPhase::Gathering,
        });
    }
    let ipv4_config = GathererConfig {
        stun_servers,
        total_timeout: Duration::from_secs(3),
        port_mapping_enabled: true,
        ipv6_enabled: false,
        generation: 1,
    };

    // Drain incremental events so a machine with many interfaces cannot block
    // gathering on a full diagnostics channel.
    let (tx, mut rx) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let ipv4_tx = tx.clone();
    let ipv6_tx = tx.clone();
    drop(tx);
    let ipv4_gather = gather_candidates_on_socket(ipv4_config.clone(), &ipv4_socket, ipv4_tx);
    let ipv6_gather = async move {
        let socket = ipv6_socket.as_ref()?;
        let mut config = ipv4_config;
        config.port_mapping_enabled = false;
        config.ipv6_enabled = true;
        gather_candidates_on_socket(config, socket, ipv6_tx)
            .await
            .ok()
    };
    let (gather_result, ipv6_result) = tokio::join!(ipv4_gather, ipv6_gather);
    let _ = drain.await;
    let mut gather_result = match gather_result {
        Ok(result) => result,
        Err(error) => {
            manager.clear_if_current(session_id, attempt_id);
            return Err(error.to_string());
        }
    };
    if let Some(manual_port) = preferred_local_port
        && let Some(public_address) = gather_result
            .candidates
            .iter()
            .find(|candidate| {
                matches!(
                    candidate.candidate_type,
                    sanser_p2p::CandidateType::ServerReflexive
                        | sanser_p2p::CandidateType::PortMapped
                )
            })
            .map(|candidate| candidate.address)
        && let Some(candidate) = manual_forward_candidate(public_address, manual_port)
        && !gather_result
            .candidates
            .iter()
            .any(|existing| existing.endpoint() == candidate.endpoint())
    {
        // A symmetric NAT may give STUN a random source port even when the
        // router has an explicit public:fixed -> host:fixed rule. Advertise
        // that fixed endpoint as a lower-priority fallback.
        if gather_result.candidates.len() >= 16 {
            gather_result.candidates.pop();
        }
        gather_result.candidates.push(candidate);
    }
    let mut candidates = gather_result.candidates.clone();
    if let Some(result) = &ipv6_result {
        candidates.extend(result.candidates.iter().cloned());
    }
    candidates.sort_unstable_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    let required = [
        candidates
            .iter()
            .find(|candidate| candidate.candidate_type == sanser_p2p::CandidateType::Ipv6Global)
            .cloned(),
        candidates
            .iter()
            .find(|candidate| candidate.candidate_type == sanser_p2p::CandidateType::Manual)
            .cloned(),
        candidates
            .iter()
            .find(|candidate| {
                matches!(
                    candidate.candidate_type,
                    sanser_p2p::CandidateType::PortMapped
                        | sanser_p2p::CandidateType::ServerReflexive
                )
            })
            .cloned(),
    ];
    // Reserve three slots for one IPv6, manual-forward and discovered public
    // endpoint so virtual IPv4 interfaces cannot crowd out useful fallbacks.
    candidates.truncate(13);
    for candidate in required.into_iter().flatten() {
        if !candidates.iter().any(|item| item.id == candidate.id) {
            candidates.push(candidate);
        }
    }
    let signaled_ids = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<HashSet<_>>();
    {
        let mut current = manager.lock_transport()?;
        let Some(transport) = current.as_mut().filter(|transport| {
            transport.session_id == session_id && transport.attempt_id == attempt_id
        }) else {
            return Err("P2P gathering was superseded by another attempt".into());
        };
        transport.sockets[0].candidates = gather_result
            .candidates
            .iter()
            .filter(|candidate| signaled_ids.contains(&candidate.id))
            .cloned()
            .collect();
        if let Some(ipv6_result) = &ipv6_result
            && let Some(socket) = transport.sockets.get_mut(1)
        {
            socket.candidates = ipv6_result
                .candidates
                .iter()
                .filter(|candidate| signaled_ids.contains(&candidate.id))
                .cloned()
                .collect();
        }
        transport.phase = P2pTransportPhase::Gathered;
        // Keep the bounded signaling set for the command result below.
        gather_result.candidates = candidates;
    }

    let public_endpoint = gather_result
        .candidates
        .iter()
        .find(|candidate| {
            matches!(
                candidate.candidate_type,
                sanser_p2p::CandidateType::PortMapped | sanser_p2p::CandidateType::ServerReflexive
            )
        })
        .map(|candidate| candidate.endpoint().to_string());
    let ipv6_available = ipv6_result
        .as_ref()
        .is_some_and(|result| !result.candidates.is_empty());
    let duration_ms = ipv6_result
        .as_ref()
        .map_or(gather_result.duration_ms, |result| {
            result.duration_ms.max(gather_result.duration_ms)
        });
    Ok(P2pGatherResult {
        candidates: gather_result.candidates,
        local_port,
        stun_succeeded: gather_result.stun_succeeded,
        port_mapping_succeeded: gather_result.port_mapping_succeeded,
        ipv6_available,
        public_endpoint,
        duration_ms,
    })
}

fn bind_ipv6_socket(port: u16) -> std::io::Result<tokio::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::new(std::net::Ipv6Addr::UNSPECIFIED.into(), port).into())?;
    tokio::net::UdpSocket::from_std(socket.into())
}

fn manual_forward_candidate(address: IpAddr, port: u16) -> Option<P2pCandidate> {
    let candidate_type = sanser_p2p::CandidateType::Manual;
    let mapping_protocol = sanser_p2p::MappingProtocol::Manual;
    let clean_address = address.to_string().replace(['.', ':'], "-");
    let candidate = P2pCandidate {
        id: format!("manual-{clean_address}-{port}"),
        candidate_type,
        address,
        port,
        protocol: sanser_p2p::TransportProtocol::Udp,
        interface_index: None,
        mapping_protocol,
        priority: sanser_p2p::candidate_priority(candidate_type, mapping_protocol, u16::MAX)
            .ok()?,
        foundation: format!("manual-{clean_address}"),
    };
    candidate.validate().is_ok().then_some(candidate)
}

fn validate_stun_server(value: &str) -> Result<String, String> {
    let value = value.trim();
    let endpoint = value.strip_prefix("stun:").unwrap_or(value);
    if value.len() > 255
        || endpoint.is_empty()
        || endpoint
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'?' | b'#' | b'@'))
    {
        return Err("STUN server must be a host and optional port".into());
    }
    Ok(format!("stun:{endpoint}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct P2pPunchRequest {
    pub session_id: String,
    pub attempt_id: String,
    pub remote_candidates: Vec<P2pCandidate>,
    pub controlling: bool,
    pub local_device_id: String,
    pub peer_device_id: String,
    pub session_credential: String,
}

impl Drop for P2pPunchRequest {
    fn drop(&mut self) {
        self.session_credential.zeroize();
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pPunchResult {
    pub local_port: u16,
    pub remote_address: String,
    pub remote_port: u16,
}

#[tauri::command]
pub async fn p2p_punch(
    manager: State<'_, P2pSessionManager>,
    mut request: P2pPunchRequest,
) -> Result<P2pPunchResult, String> {
    let session_id = uuid::Uuid::parse_str(request.session_id.trim())
        .map_err(|_| "P2P sessionId must be a UUID".to_owned())?;
    let attempt_id = uuid::Uuid::parse_str(request.attempt_id.trim())
        .map_err(|_| "P2P attemptId must be a UUID".to_owned())?;
    let local_device_id = uuid::Uuid::parse_str(request.local_device_id.trim())
        .map_err(|_| "localDeviceId must be a UUID".to_owned())?;
    let peer_device_id = uuid::Uuid::parse_str(request.peer_device_id.trim())
        .map_err(|_| "peerDeviceId must be a UUID".to_owned())?;
    if local_device_id == peer_device_id {
        return Err("P2P peers must be different devices".into());
    }
    if !(32..=256).contains(&request.session_credential.len())
        || !request.session_credential.is_ascii()
    {
        return Err("P2P session credential is invalid".into());
    }
    if request.remote_candidates.is_empty() || request.remote_candidates.len() > 64 {
        return Err("Remote candidate count must be between 1 and 64".into());
    }
    for candidate in &request.remote_candidates {
        candidate
            .validate()
            .map_err(|_| "Remote candidate metadata is invalid".to_owned())?;
    }

    let mut sockets = {
        let current = manager.lock_transport()?;
        let transport = current
            .as_ref()
            .filter(|transport| {
                transport.session_id == session_id && transport.attempt_id == attempt_id
            })
            .ok_or_else(|| "No gathered P2P socket exists for this session".to_owned())?;
        if transport.phase != P2pTransportPhase::Gathered {
            return Err("P2P socket is not ready for connectivity checks".into());
        }
        transport
            .sockets
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, Arc::clone(&entry.socket), entry.candidates.clone()))
            .collect::<Vec<_>>()
    };
    // Both peers derive the same family order. A global IPv6 path avoids NAT,
    // so give it a short first attempt before the longer IPv4 traversal.
    sockets.sort_by_key(|(_, socket, _)| {
        socket
            .local_addr()
            .map_or(2, |address| i32::from(!address.is_ipv6()))
    });

    let probe_key = derive_probe_key(session_id, &request.session_credential);
    request.session_credential.zeroize();
    let mut selected_route = None;
    let mut last_error = None;
    for (index, socket, local_candidates) in sockets {
        let mut pairs = build_candidate_pairs(
            &local_candidates,
            &request.remote_candidates,
            request.controlling,
        );
        if pairs.is_empty() {
            continue;
        }
        let is_ipv6 = socket.local_addr().is_ok_and(|address| address.is_ipv6());
        match check_connectivity(
            &socket,
            &mut pairs,
            session_id,
            device_hash(local_device_id),
            device_hash(peer_device_id),
            &probe_key,
            request.controlling,
            Duration::from_secs(if is_ipv6 { 3 } else { 6 }),
        )
        .await
        {
            Ok(pair) => {
                selected_route = Some((index, socket, pair));
                break;
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    let Some((selected_index, socket, selected)) = selected_route else {
        manager.clear_if_current(session_id, attempt_id);
        return Err(last_error.unwrap_or_else(|| {
            "No compatible IPv4 or IPv6 candidate pair could be constructed".into()
        }));
    };
    {
        let mut current = manager.lock_transport()?;
        let Some(transport) = current.as_mut().filter(|transport| {
            transport.session_id == session_id && transport.attempt_id == attempt_id
        }) else {
            return Err("P2P connectivity check was superseded".into());
        };
        transport.selected_socket = Some(selected_index);
        transport.phase = P2pTransportPhase::Selected;
    }
    let local_port = socket
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    Ok(P2pPunchResult {
        local_port,
        remote_address: selected.remote.ip().to_string(),
        remote_port: selected.remote.port(),
    })
}

fn build_candidate_pairs(
    local_candidates: &[P2pCandidate],
    remote_candidates: &[P2pCandidate],
    controlling: bool,
) -> Vec<CandidatePair> {
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for local in local_candidates {
        for remote in remote_candidates
            .iter()
            .filter(|candidate| candidate.address.is_ipv4() == local.address.is_ipv4())
        {
            let pair_id = make_pair_id(&local.id, &remote.id);
            if !seen.insert(pair_id.clone()) {
                continue;
            }
            let priority = if controlling {
                pair_priority(local.priority, remote.priority)
            } else {
                pair_priority(remote.priority, local.priority)
            };
            pairs.push(CandidatePair {
                pair_id,
                local: local.endpoint(),
                remote: remote.endpoint(),
                local_candidate_id: local.id.clone(),
                remote_candidate_id: remote.id.clone(),
                priority,
                state: PairState::Waiting,
            });
        }
    }
    pairs
}

fn derive_probe_key(session_id: uuid::Uuid, credential: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"sanser-p2p-connectivity-probe-v1\0");
    hasher.update(session_id.as_bytes());
    hasher.update(credential.as_bytes());
    hasher.finalize().into()
}

fn device_hash(device_id: uuid::Uuid) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"sanser-p2p-device-id-v1\0");
    hasher.update(device_id.as_bytes());
    let digest = hasher.finalize();
    u64::from_be_bytes(digest[..8].try_into().unwrap_or([0; 8]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn test_candidate(id: &str, address: &str, port: u16, preference: u16) -> P2pCandidate {
        let candidate_type = sanser_p2p::CandidateType::Host;
        let mapping = sanser_p2p::MappingProtocol::None;
        P2pCandidate {
            id: id.into(),
            candidate_type,
            address: address.parse().unwrap(),
            port,
            protocol: sanser_p2p::TransportProtocol::Udp,
            interface_index: Some(1),
            mapping_protocol: mapping,
            priority: sanser_p2p::candidate_priority(candidate_type, mapping, preference).unwrap(),
            foundation: format!("foundation-{id}"),
        }
    }

    #[test]
    fn peer_routes_exclude_addresses_that_cannot_identify_a_lan_peer() {
        assert!(!is_peer_route(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
        assert!(!is_peer_route(IpAddr::V4(std::net::Ipv4Addr::new(
            169, 254, 1, 2
        ))));
        assert!(is_peer_route(IpAddr::V4(std::net::Ipv4Addr::new(
            192, 168, 1, 10
        ))));
        assert!(!is_peer_route(IpAddr::V6(
            "fd12::10"
                .parse()
                .unwrap_or(std::net::Ipv6Addr::UNSPECIFIED)
        )));
        assert!(is_peer_route(IpAddr::V6(
            "2606:4700:4700::1111"
                .parse()
                .unwrap_or(std::net::Ipv6Addr::UNSPECIFIED)
        )));
    }

    #[test]
    fn route_discovery_accepts_https_and_rejects_insecure_remote_urls() {
        assert!(validated_route_target("https://sanser.example/api").is_ok());
        assert!(validated_route_target("http://127.0.0.1:5174").is_ok());
        assert!(validated_route_target("http://sanser.example").is_err());
        assert!(validated_route_target("file:///tmp/server").is_err());
    }

    #[test]
    fn candidate_pairs_match_on_controlling_and_controlled_peers() {
        let left = test_candidate("left", "192.168.1.10", 40_000, 60_000);
        let right = test_candidate("right", "192.168.1.20", 41_000, 50_000);
        let controlling = build_candidate_pairs(
            std::slice::from_ref(&left),
            std::slice::from_ref(&right),
            true,
        );
        let controlled = build_candidate_pairs(&[right], &[left], false);

        assert_eq!(controlling.len(), 1);
        assert_eq!(controlled.len(), 1);
        assert_eq!(controlling[0].pair_id, controlled[0].pair_id);
        assert_eq!(controlling[0].priority, controlled[0].priority);
    }

    #[test]
    fn candidate_pairs_support_ipv6_without_crossing_address_families() {
        let v6_left = test_candidate("v6-left", "2001:4860:4860::8888", 40_000, 60_000);
        let v6_right = test_candidate("v6-right", "2606:4700:4700::1111", 41_000, 50_000);
        let v4 = test_candidate("v4", "192.168.1.20", 41_000, 50_000);
        let pairs = build_candidate_pairs(&[v6_left], &[v4, v6_right], true);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].local.is_ipv6());
        assert!(pairs[0].remote.is_ipv6());
    }

    #[test]
    fn probe_keys_are_bound_to_session_and_credential() {
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        assert_eq!(
            derive_probe_key(first, "credential"),
            derive_probe_key(first, "credential")
        );
        assert_ne!(
            derive_probe_key(first, "credential"),
            derive_probe_key(second, "credential")
        );
        assert_ne!(
            derive_probe_key(first, "credential"),
            derive_probe_key(first, "other")
        );
    }

    #[test]
    fn stun_endpoint_validation_rejects_url_metadata() {
        assert_eq!(
            validate_stun_server("stun:stun.example.com:3478"),
            Ok("stun:stun.example.com:3478".into())
        );
        assert!(validate_stun_server("stun:user@127.0.0.1:3478").is_err());
        assert!(validate_stun_server("stun:example.test/path").is_err());
    }

    #[test]
    fn manual_forward_candidate_keeps_the_configured_udp_port() {
        let candidate =
            manual_forward_candidate(IpAddr::V4(std::net::Ipv4Addr::new(8, 8, 8, 8)), 50_000)
                .expect("public manual endpoint should validate");
        assert_eq!(candidate.port, 50_000);
        assert_eq!(candidate.candidate_type, sanser_p2p::CandidateType::Manual);
        assert_eq!(
            candidate.mapping_protocol,
            sanser_p2p::MappingProtocol::Manual
        );
    }
}
