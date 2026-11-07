//! Local-only voice window. Room credentials are handed off once, never in URLs,
//! events, persisted preferences or logs. No automatic microphone consent.
use crate::{engine::EngineManager, error::DesktopError, models::EngineKind};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;
use zeroize::Zeroize;
const LABEL: &str = "webrtc-voice";
const STARTUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    kind: EngineKind,
    sharing_id: Uuid,
    session_id: Option<Uuid>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grant {
    url: String,
    token: String,
    #[serde(flatten)]
    scope: Scope,
}
impl Drop for Grant {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
struct Call {
    id: Uuid,
    scope: Scope,
    grant: Option<Grant>,
    ready: bool,
}
impl Call {
    fn startup_expired(&self, elapsed: std::time::Duration) -> bool {
        !self.ready && elapsed >= STARTUP_DEADLINE
    }
}
#[derive(Default)]
pub struct VoiceWindowState(Mutex<Option<Call>>);

fn validate(grant: &Grant) -> Result<(), DesktopError> {
    let invalid = || {
        DesktopError::InvalidRequest("Voice needs a secure gateway and a scoped credential".into())
    };
    let url = url::Url::parse(&grant.url).map_err(|_| invalid())?;
    if !(url.scheme() == "wss" || (url.scheme() == "ws" && url.host_str() == Some("127.0.0.1")))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || grant.token.len() > 8192
        || grant.token.split('.').count() != 3
        || !grant
            .token
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'.' | b'_' | b'-'))
        || grant.scope.kind == EngineKind::LocalServer
        || (grant.scope.kind == EngineKind::Client && grant.scope.session_id.is_none())
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn closed(app: &AppHandle) {
    let scope = app
        .state::<VoiceWindowState>()
        .0
        .lock()
        .ok()
        .and_then(|mut value| value.take())
        .map(|call| call.scope);
    if let Some(scope) = scope {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _ = app.state::<EngineManager>().external_voice_gate(
                scope.kind,
                scope.sharing_id,
                scope.session_id,
                false,
            );
        });
    }
}

pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() == LABEL && matches!(event, tauri::WindowEvent::Destroyed) {
        closed(window.app_handle());
    }
}

#[tauri::command]
pub async fn open_voice_window(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, VoiceWindowState>,
    grant: Grant,
) -> Result<(), DesktopError> {
    if window.label() != "main" {
        return Err(DesktopError::InvalidRequest(
            "Voice must be joined from Sanser".into(),
        ));
    }
    validate(&grant)?;
    if let Some(existing) = app.get_webview_window(LABEL) {
        existing
            .show()
            .map_err(|_| DesktopError::Process("Unable to show voice controls".into()))?;
        return Ok(());
    }
    let scope = grant.scope;
    let id = Uuid::new_v4();
    {
        let mut value = state
            .0
            .lock()
            .map_err(|_| DesktopError::Process("Voice state unavailable".into()))?;
        if value.is_some() {
            return Err(DesktopError::Unavailable("Voice is already opening".into()));
        }
        *value = Some(Call {
            id,
            scope,
            grant: Some(grant),
            ready: false,
        });
    }
    let gate_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        gate_app.state::<EngineManager>().external_voice_gate(
            scope.kind,
            scope.sharing_id,
            scope.session_id,
            true,
        )
    })
    .await;
    if !matches!(&result, Ok(Ok(()))) {
        closed(&app);
        return result.map_err(|_| DesktopError::Process("Voice engine unavailable".into()))?;
    }
    let built = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("voice.html".into()))
        .title("Sanser Voice")
        .inner_size(390.0, 190.0)
        .min_inner_size(390.0, 190.0)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .decorations(false)
        .focused(false)
        .focusable(false)
        .build();
    if let Err(error) = built {
        closed(&app);
        return Err(DesktopError::Process(format!(
            "Unable to open voice controls: {error}"
        )));
    }
    // Ending a native sharing/viewing session also ends its voice window. A
    // previous call's monitor cannot close a newer call with the same label.
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let startup_expired = app
                .state::<VoiceWindowState>()
                .0
                .lock()
                .ok()
                .and_then(|value| {
                    value
                        .as_ref()
                        .filter(|call| call.id == id)
                        .map(|call| call.startup_expired(started.elapsed()))
                });
            let Some(startup_expired) = startup_expired else {
                break;
            };
            if startup_expired
                || app
                    .state::<EngineManager>()
                    .group_voice_status(scope.kind, scope.sharing_id, scope.session_id)
                    .is_err()
            {
                if let Some(window) = app.get_webview_window(LABEL) {
                    let _ = window.close();
                }
                break;
            }
        }
    });
    Ok(())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn voice_window_ready(
    window: WebviewWindow,
    state: State<'_, VoiceWindowState>,
) -> Result<(), DesktopError> {
    if window.label() != LABEL {
        return Err(DesktopError::InvalidRequest(
            "Voice window is required".into(),
        ));
    }
    let mut value = state
        .0
        .lock()
        .map_err(|_| DesktopError::Process("Voice state unavailable".into()))?;
    let call = value
        .as_mut()
        .filter(|call| call.grant.is_none())
        .ok_or_else(|| DesktopError::Unavailable("Voice has not started".into()))?;
    call.ready = true;
    Ok(())
}

#[tauri::command]
// Tauri injects owned window/state arguments; references are not IPC arguments.
#[allow(clippy::needless_pass_by_value)]
pub fn take_voice_window_grant(
    window: WebviewWindow,
    state: State<'_, VoiceWindowState>,
) -> Result<Grant, DesktopError> {
    if window.label() != LABEL {
        return Err(DesktopError::InvalidRequest(
            "Voice window is required".into(),
        ));
    }
    state
        .0
        .lock()
        .map_err(|_| DesktopError::Process("Voice state unavailable".into()))?
        .as_mut()
        .and_then(|call| call.grant.take())
        .ok_or_else(|| DesktopError::Unavailable("Voice grant was already consumed".into()))
}

#[tauri::command]
// Tauri injects owned window/state arguments.
#[allow(clippy::needless_pass_by_value)]
pub fn stop_voice_sharing(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, VoiceWindowState>,
) -> Result<(), DesktopError> {
    if window.label() != LABEL {
        return Err(DesktopError::InvalidRequest(
            "Voice window is required".into(),
        ));
    }
    let group = state
        .0
        .lock()
        .ok()
        .and_then(|value| {
            value
                .as_ref()
                .filter(|call| call.scope.kind == EngineKind::Host)
                .map(|call| call.scope.sharing_id)
        })
        .ok_or_else(|| {
            DesktopError::InvalidRequest("Only the sharing host can stop sharing".into())
        })?;
    app.emit_to("main", "voice-stop-sharing", group.to_string())
        .map_err(|_| DesktopError::Process("Unable to stop sharing".into()))
}

pub fn allowed_app_command(label: &str, command: &str) -> bool {
    label == "main"
        || (label == LABEL
            && matches!(
                command,
                "take_voice_window_grant" | "voice_window_ready" | "stop_voice_sharing"
            ))
}

pub fn native_replacement_allowed(app: &AppHandle, kind: EngineKind) -> bool {
    kind == EngineKind::LocalServer
        || app
            .state::<VoiceWindowState>()
            .0
            .lock()
            .is_ok_and(|value| value.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_window_has_no_account_storage_engine_or_uac_commands() {
        assert!(allowed_app_command(LABEL, "take_voice_window_grant"));
        assert!(allowed_app_command(LABEL, "voice_window_ready"));
        assert!(allowed_app_command(LABEL, "stop_voice_sharing"));
        for command in [
            "secure_get",
            "secure_set",
            "launch_engine",
            "configure_windows_uac",
            "open_voice_window",
        ] {
            assert!(!allowed_app_command(LABEL, command));
            assert!(allowed_app_command("main", command));
        }
        assert!(!allowed_app_command("unknown", "take_voice_window_grant"));
    }
    #[test]
    fn startup_watchdog_requires_a_connected_window_not_just_consumed_credentials() {
        let mut call = Call {
            id: Uuid::new_v4(),
            scope: Scope {
                kind: EngineKind::Host,
                sharing_id: Uuid::new_v4(),
                session_id: None,
            },
            grant: None,
            ready: false,
        };
        assert!(
            !call.startup_expired(
                STARTUP_DEADLINE.saturating_sub(std::time::Duration::from_millis(1))
            )
        );
        assert!(call.startup_expired(STARTUP_DEADLINE));
        call.ready = true;
        assert!(!call.startup_expired(STARTUP_DEADLINE * 2));
    }
    #[test]
    fn ipc_handoff_round_trips_without_debug_or_url_credentials() -> Result<(), serde_json::Error> {
        let body = serde_json::json!({"url":"wss://voice.example/","token":"abc.def.ghi","kind":"host","sharingId":Uuid::new_v4(),"sessionId":null});
        let grant: Grant = serde_json::from_value(body.clone())?;
        assert_eq!(serde_json::to_value(&grant)?, body);
        Ok(())
    }
    #[test]
    fn grant_rejects_remote_plaintext_url_tokens_and_nonsharing_roles() {
        let mut grant = Grant {
            url: "wss://voice.example/".into(),
            token: "abc.def.ghi".into(),
            scope: Scope {
                kind: EngineKind::Host,
                sharing_id: Uuid::new_v4(),
                session_id: None,
            },
        };
        assert!(validate(&grant).is_ok());
        for url in [
            "ws://voice.example/",
            "wss://voice.example/?token=secret",
            "wss://user:secret@voice.example/",
            "wss://voice.example/path",
        ] {
            grant.url = url.into();
            assert!(validate(&grant).is_err());
        }
        grant.url = "ws://127.0.0.1:17883/".into();
        assert!(validate(&grant).is_ok());
        grant.scope.kind = EngineKind::Client;
        assert!(validate(&grant).is_err());
    }
}
