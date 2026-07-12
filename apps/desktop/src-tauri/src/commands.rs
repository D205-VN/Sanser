// Tauri extracts owned command arguments and state guards through its command
// macro. References here are not valid IPC command arguments even though the
// implementation itself only borrows them.
#![allow(clippy::needless_pass_by_value)]

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
        // The current native listeners use IPv4 sockaddr structures. Do not
        // advertise an IPv6 route until both endpoint parsers/listeners are
        // migrated and covered by interoperability tests.
        IpAddr::V6(_) => false,
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
    request: LaunchEngineRequest,
) -> Result<(), DesktopError> {
    engines.launch(&app, &request)
}

#[tauri::command]
pub fn stop_engine(
    engines: State<'_, EngineManager>,
    kind: EngineKind,
) -> Result<(), DesktopError> {
    engines.stop(kind)
}

#[tauri::command]
pub fn get_engine_status(
    app: AppHandle,
    engines: State<'_, EngineManager>,
    kind: EngineKind,
) -> Result<EngineStatus, DesktopError> {
    engines.status(&app, kind)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn route_discovery_accepts_https_and_rejects_insecure_remote_urls() {
        assert!(validated_route_target("https://sanser.example/api").is_ok());
        assert!(validated_route_target("http://127.0.0.1:5174").is_ok());
        assert!(validated_route_target("http://sanser.example").is_err());
        assert!(validated_route_target("file:///tmp/server").is_err());
    }
}
