// Tauri extracts owned command arguments and state guards through its command
// macro. References here are not valid IPC command arguments even though the
// implementation itself only borrows them.
#![allow(clippy::needless_pass_by_value)]

use tauri::{AppHandle, State};

use crate::{
    engine::EngineManager,
    error::DesktopError,
    models::{
        Capability, DiagnosticsExport, EngineKind, LaunchEngineRequest, PROTOCOL_VERSION,
        Preferences, RuntimeCapabilities, RuntimeStatus, SANSER_VERSION,
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
            local_server: Capability::unavailable(
                "Local database/server mode is disabled; desktop uses the deployed PostgreSQL/Neon API",
            ),
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
pub async fn export_diagnostics(
    app: AppHandle,
    contents: String,
) -> Result<DiagnosticsExport, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || storage::export_diagnostics(&app, &contents))
        .await
        .map_err(|error| DesktopError::Storage(format!("diagnostics task failed: {error}")))?
}
