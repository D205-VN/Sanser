use tauri::{AppHandle, State};

use crate::{
    engine::{find_sidecar, EngineManager},
    error::DesktopError,
    models::{
        Capability, DiagnosticsExport, EngineKind, LaunchEngineRequest, Preferences,
        RuntimeCapabilities, RuntimeStatus, PROTOCOL_VERSION, SANSER_VERSION,
    },
    storage,
};

fn platform_label() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[tauri::command]
pub fn get_runtime_status(
    app: AppHandle,
    engines: State<'_, EngineManager>,
) -> Result<RuntimeStatus, DesktopError> {
    let host = engines.status(&app, EngineKind::Host)?;
    let client = engines.status(&app, EngineKind::Client)?;
    let local_server = engines.status(&app, EngineKind::LocalServer)?;
    let direct_engine = host.installed || client.installed;

    let host_capability = if !cfg!(target_os = "windows") {
        Capability::unavailable("Windows host engine is only available on Windows")
    } else if host.installed {
        Capability::available()
    } else {
        Capability::unavailable("sanser-host-windows is not bundled")
    };
    let client_capability = if !cfg!(target_os = "macos") {
        Capability::unavailable("macOS client engine is only available on macOS")
    } else if client.installed {
        Capability::available()
    } else {
        Capability::unavailable("sanser-client-macos is not bundled")
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
            local_server: if local_server.installed {
                Capability::available()
            } else {
                Capability::unavailable("sanser-server is not bundled")
            },
            local_discovery: Capability::planned(
                "Signed mDNS/UDP discovery backend is not installed",
            ),
            web_rtc: Capability::planned("libdatachannel transport is not linked yet"),
            native_snv2: if direct_engine {
                Capability::unavailable(
                    "Native sidecar is installed, but its SNV2 capability handshake is not verified",
                )
            } else {
                Capability::unavailable("No platform native SNV2 sidecar is bundled")
            },
            gamepad: Capability::planned("Native controller state transport is not linked yet"),
            clipboard: Capability::planned("Permission-gated clipboard transport is not linked yet"),
        },
        engines: vec![host, client, local_server],
    })
}

#[tauri::command]
pub fn load_preferences(app: AppHandle) -> Result<Option<Preferences>, DesktopError> {
    storage::load_preferences(&app)
}

#[tauri::command]
pub fn save_preferences(
    app: AppHandle,
    preferences: Preferences,
) -> Result<(), DesktopError> {
    storage::save_preferences(&app, &preferences)
}

#[tauri::command]
pub fn secure_get(key: String) -> Result<Option<String>, DesktopError> {
    storage::secure_get(&key)
}

#[tauri::command]
pub fn secure_set(key: String, value: String) -> Result<(), DesktopError> {
    storage::secure_set(&key, value)
}

#[tauri::command]
pub fn secure_delete(key: String) -> Result<(), DesktopError> {
    storage::secure_delete(&key)
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
pub fn export_diagnostics(
    app: AppHandle,
    contents: String,
) -> Result<DiagnosticsExport, DesktopError> {
    storage::export_diagnostics(&app, &contents)
}

#[allow(dead_code)]
fn whitelist_is_complete(app: &AppHandle) -> bool {
    [EngineKind::Host, EngineKind::Client, EngineKind::LocalServer]
        .into_iter()
        .all(|kind| find_sidecar(app, kind).is_some())
}
