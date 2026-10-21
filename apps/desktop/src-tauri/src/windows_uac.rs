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
    use std::{process::Stdio, time::Instant};
    let path = crate::engine::named_candidate_paths(app, "sanser-uac-service")
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| DesktopError::Unavailable("Windows UAC helper is not bundled".into()))?;
    let mut command = crate::engine::native_command(&path);
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

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri IPC owns extracted AppHandle arguments.
pub(crate) fn windows_uac_status(app: AppHandle) -> UacStatus {
    #[cfg(windows)]
    {
        let available = crate::engine::named_candidate_paths(&app, "sanser-uac-service")
            .into_iter()
            .any(|path| path.is_file());
        let enabled = helper_output(&app, "--status", std::time::Duration::from_secs(2))
            .ok()
            .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
            .is_some_and(|status| {
                status.get("enabled").and_then(serde_json::Value::as_bool) == Some(true)
            });
        let path = protected_host(&app);
        return UacStatus {
            available,
            enabled,
            ready: path
                .is_some_and(|path| crate::engine::probe_path(&path, EngineKind::Host).compatible),
        };
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        UacStatus::default()
    }
}

#[cfg(windows)]
pub(crate) fn ensure_current(app: &AppHandle) -> Result<(), DesktopError> {
    let output = match helper_output(app, "--status", std::time::Duration::from_secs(2)) {
        Ok(output) => output,
        Err(_) => return Ok(()),
    };
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        DesktopError::Process("Unable to verify Windows administrator-window support".into())
    })?;
    if status.get("enabled").and_then(serde_json::Value::as_bool) == Some(true)
        && !current_installation(&status)
    {
        return Err(DesktopError::Unavailable(
            "Windows administrator-window support is from an older installation. Reinstall the current Sanser update, or use Host > Windows administrator prompts > Update UAC support before connecting.".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::current_installation;
    use serde_json::json;

    #[test]
    fn stale_or_unknown_protected_helper_never_counts_as_current() {
        assert!(!current_installation(
            &json!({"enabled":true,"version":"2.2.18"})
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
        Ok(windows_uac_status(app))
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
