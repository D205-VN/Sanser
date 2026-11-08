use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{engine::EngineManager, error::DesktopError, models::EngineKind};

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UacStatus {
    available: bool,
    enabled: bool,
    ready: bool,
}

// Older helpers do not report `current`: they must be repaired before they can
// be used to authorize this release's protected native host.
#[cfg(any(windows, test))]
fn current_installation(status: &serde_json::Value) -> bool {
    status.get("enabled").and_then(serde_json::Value::as_bool) == Some(true)
        && status.get("current").and_then(serde_json::Value::as_bool) == Some(true)
}

#[cfg(windows)]
fn helper_output(
    app: &AppHandle,
    mode: &str,
    timeout: std::time::Duration,
) -> Result<std::process::Output, DesktopError> {
    let path = crate::engine::named_candidate_paths(app, "sanser-uac-service")
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| DesktopError::Unavailable("Windows UAC helper is not bundled".into()))?;
    native_output(&path, mode, timeout)
}

#[cfg(windows)]
fn native_output(
    path: &std::path::Path,
    mode: &str,
    timeout: std::time::Duration,
) -> Result<std::process::Output, DesktopError> {
    use std::{process::Stdio, time::Instant};
    let mut command = crate::engine::native_command(path);
    command
        .arg(mode)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            if output.status.success() {
                return Ok(output);
            }
            return Err(DesktopError::Process(
                "Windows UAC setup failed or administrator approval was cancelled".into(),
            ));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DesktopError::Process("Windows UAC helper timed out".into()));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(windows)]
pub(crate) fn protected_host(app: &AppHandle) -> Option<std::path::PathBuf> {
    use std::time::Duration;
    let output = helper_output(app, "--status", Duration::from_secs(2)).ok()?;
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    if current_installation(&status) {
        let output = helper_output(app, "--host-path", Duration::from_secs(2)).ok()?;
        let path = std::path::PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
        return path.is_absolute().then_some(path).filter(|p| p.is_file());
    }
    None
}

// Resolve once for a launch. A second best-effort status lookup used to fall
// back to the user-writable bundled host after the first lookup said UAC was
// enabled. That host is correctly rejected by the SYSTEM broker.
#[cfg(any(windows, test))]
fn resolve_launch_host(
    status: &serde_json::Value,
    path: impl FnOnce() -> Result<Option<std::path::PathBuf>, DesktopError>,
) -> Result<Option<std::path::PathBuf>, DesktopError> {
    match status.get("enabled").and_then(serde_json::Value::as_bool) {
        Some(false) => Ok(None),
        Some(true) if current_installation(status) => {
            let path = path()?.ok_or_else(|| DesktopError::Unavailable(
                "The protected Windows host is unavailable. Repair Windows administrator-window support before connecting.".into(),
            ))?;
            if !path.is_absolute() || !path.is_file() {
                return Err(DesktopError::Unavailable(
                    "The protected Windows host is missing. Reinstall the current Sanser update or repair UAC support.".into(),
                ));
            }
            Ok(Some(path))
        }
        Some(true) => Err(DesktopError::Unavailable(
            "Windows administrator-window support is from an older installation. Reinstall the current Sanser update, or use Host > Windows administrator prompts > Update UAC support before connecting.".into(),
        )),
        None => Err(DesktopError::Process(
            "Unable to verify Windows administrator-window support. Repair UAC support before connecting.".into(),
        )),
    }
}

#[cfg(windows)]
pub(crate) fn launch_host(
    app: &AppHandle,
    input_enabled: bool,
) -> Result<Option<std::path::PathBuf>, DesktopError> {
    use std::time::Duration;
    if !crate::engine::named_candidate_paths(app, "sanser-uac-service")
        .iter()
        .any(|path| path.is_file())
    {
        return Ok(None);
    }
    let output = helper_output(app, "--status", Duration::from_secs(5))?;
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        DesktopError::Process("Unable to verify Windows administrator-window support".into())
    })?;
    let path = resolve_launch_host(&status, || {
        let output = helper_output(app, "--host-path", Duration::from_secs(2))?;
        let path = String::from_utf8(output.stdout)
            .map_err(|_| DesktopError::Process("Invalid protected Windows host path".into()))?;
        Ok(Some(std::path::PathBuf::from(path.trim())))
    })?;
    if input_enabled && let Some(path) = &path {
        let output = native_output(path, "--uac-ready-json", Duration::from_secs(6))?;
        let readiness: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| {
                DesktopError::Process(
                    "Unable to verify the Windows administrator input channel".into(),
                )
            })?;
        validate_input_readiness(&readiness)?;
    }
    Ok(path)
}

#[cfg(any(windows, test))]
fn validate_input_readiness(readiness: &serde_json::Value) -> Result<(), DesktopError> {
    if readiness.get("ready").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(());
    }
    let number = |name: &str| {
        readiness
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    Err(DesktopError::Unavailable(format!(
        "Windows administrator input is unavailable (stage={}, status={}, check={}, win32={}). Repair UAC support on the Windows host before reconnecting.",
        number("stage"),
        number("status"),
        number("check"),
        number("win32"),
    )))
}

fn read_windows_uac_status(app: &AppHandle) -> UacStatus {
    #[cfg(windows)]
    {
        let available = crate::engine::named_candidate_paths(app, "sanser-uac-service")
            .into_iter()
            .any(|path| path.is_file());
        let enabled = helper_output(app, "--status", std::time::Duration::from_secs(2))
            .ok()
            .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
            .is_some_and(|status| {
                status.get("enabled").and_then(serde_json::Value::as_bool) == Some(true)
            });
        let path = protected_host(app);
        return UacStatus {
            available,
            enabled,
            ready: path.is_some_and(|path| {
                crate::engine::probe_path(&path, EngineKind::Host).compatible
                    && native_output(&path, "--uac-ready-json", std::time::Duration::from_secs(6))
                        .ok()
                        .and_then(|output| {
                            serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
                        })
                        .is_some_and(|value| validate_input_readiness(&value).is_ok())
            }),
        };
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        UacStatus::default()
    }
}

#[tauri::command]
pub(crate) async fn windows_uac_status(app: AppHandle) -> Result<UacStatus, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || read_windows_uac_status(&app))
        .await
        .map_err(|error| DesktopError::Process(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::current_installation;
    use serde_json::json;

    #[test]
    fn stale_or_unknown_protected_helper_never_counts_as_current() {
        assert!(!current_installation(
            &json!({"enabled":true,"version":"2.2.19"})
        ));
        assert!(!current_installation(
            &json!({"enabled":true,"current":false})
        ));
        assert!(!current_installation(
            &json!({"enabled":false,"current":true})
        ));
        assert!(current_installation(
            &json!({"enabled":true,"current":true})
        ));
    }

    #[test]
    fn enabled_broker_never_falls_back_to_an_unprotected_host() {
        use super::resolve_launch_host;
        use crate::error::DesktopError;
        let current = json!({"enabled":true,"current":true});
        assert!(
            resolve_launch_host(&current, || Err(DesktopError::Process(
                "transient timeout".into()
            )))
            .is_err()
        );
        assert!(resolve_launch_host(&current, || Ok(None)).is_err());
        assert!(
            resolve_launch_host(&current, || Ok(Some("missing-protected-host.exe".into())))
                .is_err()
        );
        assert!(
            resolve_launch_host(&json!({"enabled":true,"current":false}), || panic!(
                "stale host queried"
            ))
            .is_err()
        );
        assert!(resolve_launch_host(&json!({}), || panic!("unknown status queried")).is_err());
        assert!(
            resolve_launch_host(&json!({"enabled":false}), || panic!(
                "disabled broker queried"
            ))
            .unwrap()
            .is_none()
        );
        let exe = std::env::current_exe().unwrap();
        let mut lookups = 0;
        let selected = resolve_launch_host(&current, || {
            lookups += 1;
            Ok(Some(exe.clone()))
        })
        .unwrap();
        assert_eq!(selected, Some(exe));
        assert_eq!(lookups, 1);
    }

    #[test]
    fn service_running_does_not_imply_the_privileged_input_channel_is_ready() {
        use super::validate_input_readiness;
        assert!(validate_input_readiness(&json!({"enabled":true,"current":true})).is_err());
        let failure = validate_input_readiness(&json!({"ready":false,"stage":3,"status":9,"check":5,"win32":5,"path":"private","token":"secret"})).unwrap_err().to_string();
        assert!(failure.contains("stage=3, status=9, check=5, win32=5"));
        assert!(!failure.contains("private") && !failure.contains("secret"));
        assert!(validate_input_readiness(&json!({"ready":true})).is_ok());
    }
}

#[tauri::command]
pub(crate) async fn configure_windows_uac(
    app: AppHandle,
    engines: State<'_, EngineManager>,
    enabled: bool,
) -> Result<UacStatus, DesktopError> {
    if engines.status(&app, EngineKind::Host)?.running {
        return Err(DesktopError::InvalidRequest(
            "Stop the current screen-sharing session before changing UAC support".into(),
        ));
    }
    #[cfg(windows)]
    return tauri::async_runtime::spawn_blocking(move || {
        helper_output(
            &app,
            if enabled { "--enable" } else { "--disable" },
            std::time::Duration::from_secs(130),
        )?;
        Ok(read_windows_uac_status(&app))
    })
    .await
    .map_err(|error| DesktopError::Process(error.to_string()))?;
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err(DesktopError::Unavailable(
            "UAC support is only available on Windows".into(),
        ))
    }
}
