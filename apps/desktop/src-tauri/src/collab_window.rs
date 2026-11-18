//! Chat/annotation commands and the local annotation overlay window.
//!
//! The overlay is a transparent, always-on-top, click-through webview. It is
//! content-protected (Windows `WDA_EXCLUDEFROMCAPTURE`, macOS
//! `NSWindowSharingNone`) and, on a macOS host, additionally removed from the
//! `ScreenCaptureKit` filter by the native engine, so strokes are never baked
//! into the captured video. Pen mode makes it accept the pointer until the
//! user presses Done or Esc.
#![allow(clippy::needless_pass_by_value)] // Tauri commands take owned handles.
use crate::{
    collab::{Event, Outgoing, Snapshot},
    engine::EngineManager,
    error::DesktopError,
    models::EngineKind,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State,
    WebviewUrl, WebviewWindowBuilder,
};
use uuid::Uuid;

pub const LABEL: &str = "annotation-overlay";
/// Also matched by the macOS host engine to exclude the window from capture.
const TITLE: &str = "Sanser Annotations";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub kind: EngineKind,
    pub session_id: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Overlay {
    #[serde(skip)]
    id: Uuid,
    scope: Scope,
    pen: bool,
    color: u8,
    #[serde(skip)]
    name: String,
}
#[derive(Default)]
pub struct OverlayState(Mutex<Option<Overlay>>);

fn scope_valid(scope: Scope) -> Result<(), DesktopError> {
    match scope.kind {
        EngineKind::Host => Ok(()),
        EngineKind::Client if scope.session_id.is_some() => Ok(()),
        _ => Err(DesktopError::InvalidRequest(
            "Chat needs an active session".into(),
        )),
    }
}

/// The overlay may only read strokes and send its own pen input.
pub fn allowed_app_command(label: &str, command: &str) -> bool {
    label == LABEL
        && matches!(
            command,
            "annotation_overlay_poll" | "annotation_overlay_stroke" | "annotation_overlay_pen_off"
        )
}

#[tauri::command]
pub fn collab_snapshot(
    engines: State<'_, EngineManager>,
    scope: Scope,
    after: u64,
) -> Result<Snapshot, DesktopError> {
    scope_valid(scope)?;
    engines.collab_snapshot(scope.kind, scope.session_id, after)
}

#[tauri::command]
pub fn collab_post(
    engines: State<'_, EngineManager>,
    scope: Scope,
    message: Outgoing,
) -> Result<u64, DesktopError> {
    scope_valid(scope)?;
    engines.collab_post(scope.kind, scope.session_id, message)
}

#[tauri::command]
pub fn collab_set_annotations(
    engines: State<'_, EngineManager>,
    enabled: bool,
) -> Result<(), DesktopError> {
    engines.collab_set_annotations(enabled)
}

fn close_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.close();
    }
}

#[tauri::command]
pub async fn open_annotation_overlay(
    app: AppHandle,
    scope: Scope,
    locale: String,
) -> Result<(), DesktopError> {
    scope_valid(scope)?;
    // The session must be live before a capture-excluded window appears.
    app.state::<EngineManager>()
        .collab_snapshot(scope.kind, scope.session_id, u64::MAX)?;
    let id = Uuid::new_v4();
    {
        let state = app.state::<OverlayState>();
        let mut current = state
            .0
            .lock()
            .map_err(|_| DesktopError::Process("Annotation overlay unavailable".into()))?;
        if current
            .as_ref()
            .is_some_and(|overlay| overlay.scope == scope)
            && app.get_webview_window(LABEL).is_some()
        {
            return Ok(());
        }
        *current = Some(Overlay {
            id,
            scope,
            pen: false,
            color: 0,
            name: String::new(),
        });
    }
    close_window(&app);
    let window = WebviewWindowBuilder::new(
        &app,
        LABEL,
        WebviewUrl::App(
            if locale == "en" {
                "annotation.html?lang=en"
            } else {
                "annotation.html?lang=vi"
            }
            .into(),
        ),
    )
    .title(TITLE)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .resizable(false)
    .focused(false)
    .content_protected(true)
    .visible(false)
    .inner_size(1.0, 1.0)
    .build()
    .map_err(|error| {
        DesktopError::Process(format!("Unable to open the annotation overlay: {error}"))
    })?;
    window.set_ignore_cursor_events(true).map_err(|error| {
        let _ = window.close();
        DesktopError::Process(format!("Unable to make the overlay click-through: {error}"))
    })?;
    if scope.kind == EngineKind::Host {
        // The native host captures the primary (main) display.
        if let Ok(Some(monitor)) = window.primary_monitor() {
            let _ = window.set_position(*monitor.position());
            let _ = window.set_size(*monitor.size());
        }
        let _ = window.show();
        let _ = app.state::<EngineManager>().collab_exclude_overlay(true);
    }
    tauri::async_runtime::spawn(follow(app.clone(), id, scope));
    Ok(())
}

// Keep a viewer overlay aligned with the native viewer window, and close the
// overlay when its session ends or a newer overlay replaces it.
async fn follow(app: AppHandle, id: Uuid, scope: Scope) {
    let mut last = None;
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let current = app
            .state::<OverlayState>()
            .0
            .lock()
            .ok()
            .is_some_and(|value| value.as_ref().is_some_and(|overlay| overlay.id == id));
        let engines = app.state::<EngineManager>();
        let live = engines
            .collab_snapshot(scope.kind, scope.session_id, u64::MAX)
            .is_ok();
        let Some(window) = app.get_webview_window(LABEL) else {
            break;
        };
        if !current || !live {
            if current && let Ok(mut state) = app.state::<OverlayState>().0.lock() {
                *state = None;
            }
            let _ = window.close();
            break;
        }
        let Some(session) = scope
            .session_id
            .filter(|_| scope.kind == EngineKind::Client)
        else {
            continue;
        };
        let view = engines.collab_view(session);
        if view == last {
            continue;
        }
        last = view;
        match view {
            Some(view) if view.visible && view.width > 0 && view.height > 0 => {
                if view.unit == "pt" {
                    let _ = window.set_position(LogicalPosition::new(view.x, view.y));
                    let _ = window.set_size(LogicalSize::new(view.width, view.height));
                } else {
                    let _ = window.set_position(PhysicalPosition::new(view.x, view.y));
                    let _ = window.set_size(PhysicalSize::new(view.width, view.height));
                }
                let _ = window.show();
            }
            _ => {
                let _ = window.hide();
            }
        }
    }
    if app.get_webview_window(LABEL).is_none() {
        let mut excluded = false;
        if let Ok(mut state) = app.state::<OverlayState>().0.lock()
            && state.as_ref().is_some_and(|overlay| overlay.id == id)
        {
            excluded = scope.kind == EngineKind::Host;
            *state = None;
        }
        if excluded {
            let _ = app.state::<EngineManager>().collab_exclude_overlay(false);
        }
    }
}

#[tauri::command]
pub fn close_annotation_overlay(app: AppHandle) -> Result<(), DesktopError> {
    let host = app
        .state::<OverlayState>()
        .0
        .lock()
        .map_err(|_| DesktopError::Process("Annotation overlay unavailable".into()))?
        .take()
        .is_some_and(|overlay| overlay.scope.kind == EngineKind::Host);
    close_window(&app);
    if host {
        let _ = app.state::<EngineManager>().collab_exclude_overlay(false);
    }
    Ok(())
}

fn apply_pen(app: &AppHandle, enabled: bool) -> Result<(), DesktopError> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    window
        .set_ignore_cursor_events(!enabled)
        .map_err(|error| DesktopError::Process(format!("Unable to change pen mode: {error}")))?;
    if enabled {
        let _ = window.set_focus();
    }
    Ok(())
}

#[tauri::command]
pub fn set_annotation_pen(
    app: AppHandle,
    enabled: bool,
    color: u8,
    name: String,
) -> Result<(), DesktopError> {
    if color >= crate::collab::COLORS {
        return Err(DesktopError::InvalidRequest("Unknown pen colour".into()));
    }
    {
        let state = app.state::<OverlayState>();
        let mut current = state
            .0
            .lock()
            .map_err(|_| DesktopError::Process("Annotation overlay unavailable".into()))?;
        let overlay = current.as_mut().ok_or_else(|| {
            DesktopError::Unavailable("Open annotations before using the pen".into())
        })?;
        if enabled {
            let annotations = app
                .state::<EngineManager>()
                .collab_snapshot(overlay.scope.kind, overlay.scope.session_id, u64::MAX)?
                .annotations;
            if !annotations {
                return Err(DesktopError::Unavailable(
                    "The host turned annotations off".into(),
                ));
            }
        }
        overlay.pen = enabled;
        overlay.color = color;
        overlay.name = crate::collab::clean_name(&name);
    }
    apply_pen(&app, enabled)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPoll {
    kind: EngineKind,
    pen: bool,
    color: u8,
    annotations: bool,
    video_width: u32,
    video_height: u32,
    cursor: u64,
    events: Vec<Event>,
}

fn overlay(app: &AppHandle) -> Result<Overlay, DesktopError> {
    app.state::<OverlayState>()
        .0
        .lock()
        .map_err(|_| DesktopError::Process("Annotation overlay unavailable".into()))?
        .clone()
        .ok_or_else(|| DesktopError::Unavailable("Annotation overlay closed".into()))
}

#[tauri::command]
pub fn annotation_overlay_poll(app: AppHandle, after: u64) -> Result<OverlayPoll, DesktopError> {
    let overlay = overlay(&app)?;
    let engines = app.state::<EngineManager>();
    let snapshot = engines.collab_snapshot(overlay.scope.kind, overlay.scope.session_id, after)?;
    let view = overlay
        .scope
        .session_id
        .filter(|_| overlay.scope.kind == EngineKind::Client)
        .and_then(|session| engines.collab_view(session));
    if overlay.pen && !snapshot.annotations {
        // Host policy changed while drawing: leave pen mode.
        if let Ok(mut state) = app.state::<OverlayState>().0.lock()
            && let Some(current) = state.as_mut()
        {
            current.pen = false;
        }
        let _ = apply_pen(&app, false);
    }
    Ok(OverlayPoll {
        kind: overlay.scope.kind,
        pen: overlay.pen && snapshot.annotations,
        color: overlay.color,
        annotations: snapshot.annotations,
        video_width: view.map_or(0, |view| view.video_width),
        video_height: view.map_or(0, |view| view.video_height),
        cursor: snapshot.cursor,
        events: snapshot
            .events
            .into_iter()
            .filter(|event| event.kind != "chat")
            .collect(),
    })
}

#[tauri::command]
pub fn annotation_overlay_stroke(
    app: AppHandle,
    stroke_id: String,
    points: Vec<u16>,
    end: bool,
    clear: bool,
) -> Result<u64, DesktopError> {
    let overlay = overlay(&app)?;
    if !overlay.pen {
        return Err(DesktopError::Unavailable("Pen mode is off".into()));
    }
    let message = if clear {
        Outgoing::Clear { name: overlay.name }
    } else {
        Outgoing::Stroke {
            name: overlay.name,
            stroke_id,
            color: overlay.color,
            points,
            end,
        }
    };
    app.state::<EngineManager>()
        .collab_post(overlay.scope.kind, overlay.scope.session_id, message)
}

#[tauri::command]
pub fn annotation_overlay_pen_off(app: AppHandle) -> Result<(), DesktopError> {
    if let Ok(mut state) = app.state::<OverlayState>().0.lock()
        && let Some(current) = state.as_mut()
    {
        current.pen = false;
    }
    apply_pen(&app, false)
}

#[tauri::command]
pub fn annotation_overlay_status(app: AppHandle) -> Result<Option<Scope>, DesktopError> {
    Ok(app
        .state::<OverlayState>()
        .0
        .lock()
        .map_err(|_| DesktopError::Process("Annotation overlay unavailable".into()))?
        .as_ref()
        .map(|overlay| overlay.scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_window_can_only_poll_and_draw() {
        assert!(allowed_app_command(LABEL, "annotation_overlay_poll"));
        assert!(allowed_app_command(LABEL, "annotation_overlay_stroke"));
        assert!(allowed_app_command(LABEL, "annotation_overlay_pen_off"));
        for command in [
            "collab_post",
            "collab_set_annotations",
            "launch_engine",
            "secure_get",
            "session_user_action",
            "set_annotation_pen",
        ] {
            assert!(!allowed_app_command(LABEL, command), "{command}");
        }
        assert!(!allowed_app_command(
            "webrtc-voice",
            "annotation_overlay_poll"
        ));
    }
    #[test]
    fn scopes_require_a_viewer_session() {
        let session = Some(Uuid::new_v4());
        assert!(
            scope_valid(Scope {
                kind: EngineKind::Host,
                session_id: None
            })
            .is_ok()
        );
        assert!(
            scope_valid(Scope {
                kind: EngineKind::Client,
                session_id: session
            })
            .is_ok()
        );
        assert!(
            scope_valid(Scope {
                kind: EngineKind::Client,
                session_id: None
            })
            .is_err()
        );
        assert!(
            scope_valid(Scope {
                kind: EngineKind::LocalServer,
                session_id: session
            })
            .is_err()
        );
    }
}
