//! Exam mode on the student computer. See docs/exam-mode.md.
//!
//! A small platform script keeps only the allowed apps in front, closes AI
//! apps and spots AI websites by window title. It prints what it sees; the
//! exam page drains that report into the heartbeat. While a guard runs
//! Sanser refuses to quit. The supervisor (or the room's time limit) ends it.
#![allow(clippy::needless_pass_by_value)] // Tauri commands take owned state handles.

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};

use crate::error::DesktopError;

const INTERVAL_MS: u32 = 350;
const MAX_QUEUED: usize = 50;

/// Words that mark an AI app by bundle identifier, file name or app name.
const AI_APPS: [&str; 16] = [
    "chatgpt",
    "openai",
    "claude",
    "anthropic",
    "gemini",
    "copilot",
    "perplexity",
    "deepseek",
    "grok",
    "lm studio",
    "lmstudio",
    "ollama",
    "quillbot",
    "msty",
    "cursor",
    "windsurf",
];

/// Window titles of AI websites. A bare "Claude" or "Gemini" counts only as a
/// whole title segment, so "Claude Monet - Wikipedia" is not a violation.
const AI_TITLES: [&str; 10] = [
    "chatgpt",
    "(^|[-–—|:] *)claude( *[-–—|:]|$)",
    "google gemini|(^|[-–—|:] *)gemini( *[-–—|:]|$)",
    "copilot",
    "perplexity",
    "deepseek",
    "(^|[-–—|:] *)grok( *[-–—|:]|$)",
    "quillbot",
    "character\\.ai",
    "meta ai|mistral ai|le chat mistral",
];

/// In front without being a violation: lock screen, password prompts, menus.
#[cfg(target_os = "macos")]
const SYSTEM_APPS: [&str; 9] = [
    "com.apple.loginwindow",
    "com.apple.ScreenSaver.Engine",
    "com.apple.SecurityAgent",
    "com.apple.systemuiserver",
    "com.apple.dock",
    "com.apple.notificationcenterui",
    "com.apple.controlcenter",
    "com.apple.coreservices.uiagent",
    "com.apple.UserNotificationCenter",
];
#[cfg(not(target_os = "macos"))]
const SYSTEM_APPS: [&str; 4] = [
    "lockapp.exe",
    "logonui.exe",
    "consent.exe",
    "credentialuibroker.exe",
];
/// Windows shell parts: switched away from silently (taskbar, Start, search).
const QUIET_APPS: [&str; 5] = [
    "explorer.exe",
    "searchhost.exe",
    "searchapp.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuardSettings {
    /// macOS bundle identifiers or Windows file names. Empty while paused.
    pub allowed_apps: Vec<String>,
    pub block_ai: bool,
    /// The supervisor paused this student: Sanser covers the whole screen.
    #[serde(default)]
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Violation {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardReport {
    pub running: bool,
    pub foreground_app: Option<String>,
    pub violations: Vec<Violation>,
}

#[derive(Default)]
struct Seen {
    foreground: Option<String>,
    violations: Vec<Violation>,
    exited: bool,
}

struct Running {
    child: Child,
    settings: GuardSettings,
    seen: Arc<Mutex<Seen>>,
}

#[derive(Default)]
pub struct ExamGuard {
    running: Mutex<Option<Running>>,
    locked: AtomicBool,
}

impl ExamGuard {
    /// True while a student exam is in progress; quitting is refused.
    pub fn locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    fn stop_child(&self) {
        if let Ok(mut running) = self.running.lock()
            && let Some(mut current) = running.take()
        {
            let _ = current.child.kill();
            let _ = current.child.wait();
        }
    }
}

impl Drop for ExamGuard {
    fn drop(&mut self) {
        self.stop_child();
    }
}

/// The JSON settings both platform scripts read.
fn script_config(settings: &GuardSettings) -> serde_json::Value {
    serde_json::json!({
        "allowed": settings.allowed_apps,
        "blockAi": settings.block_ai,
        "system": SYSTEM_APPS,
        "quiet": QUIET_APPS,
        "aiApps": AI_APPS,
        "aiTitles": AI_TITLES,
        "selfId": "com.sanser.desktop",
        "selfPid": std::process::id(),
        "intervalMs": INTERVAL_MS,
    })
}

/// One line of script output. Unknown or malformed lines are ignored.
fn record(seen: &Mutex<Seen>, line: &str) {
    #[derive(Deserialize)]
    struct Line {
        t: String,
        #[serde(default)]
        d: String,
    }
    let Ok(line) = serde_json::from_str::<Line>(line) else {
        return;
    };
    let detail: String = line
        .d
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let Ok(mut seen) = seen.lock() else {
        return;
    };
    match line.t.as_str() {
        "front" => seen.foreground = Some(detail),
        "left-app" | "ai-app" | "ai-site" => {
            // The same app bounced back many times is one entry per report.
            let duplicate = seen
                .violations
                .iter()
                .any(|item| item.kind == line.t && item.detail == detail);
            if !duplicate && seen.violations.len() < MAX_QUEUED {
                seen.violations.push(Violation {
                    kind: line.t,
                    detail,
                });
            }
        }
        _ => {}
    }
}

#[cfg(target_os = "macos")]
fn spawn_script(settings: &GuardSettings) -> std::io::Result<Child> {
    Command::new("/usr/bin/osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            include_str!("exam_guard/guard_macos.js"),
        ])
        .arg(script_config(settings).to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(windows)]
fn encoded_powershell(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
fn powershell(script: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
        ])
        .arg(encoded_powershell(script))
        .creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(windows)]
fn spawn_script(settings: &GuardSettings) -> std::io::Result<Child> {
    powershell(include_str!("exam_guard/guard_windows.ps1"))
        .env("SANSER_EXAM_GUARD", script_config(settings).to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn spawn_script(_settings: &GuardSettings) -> std::io::Result<Child> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "exam mode needs macOS or Windows",
    ))
}

/// Starts the guard, or restarts it when the settings changed.
#[tauri::command]
pub fn exam_guard_apply(
    app: tauri::AppHandle,
    state: tauri::State<'_, ExamGuard>,
    settings: GuardSettings,
) -> Result<(), DesktopError> {
    if settings.allowed_apps.len() > 32
        || settings
            .allowed_apps
            .iter()
            .any(|id| !crate::engine::valid_blocked_app_id(id))
    {
        return Err(DesktopError::InvalidRequest(
            "allowed apps are invalid".into(),
        ));
    }
    state.locked.store(true, Ordering::SeqCst);
    {
        let mut running = state
            .running
            .lock()
            .map_err(|_| DesktopError::Unavailable("exam guard is unavailable".into()))?;
        if let Some(current) = running.as_mut() {
            let alive = matches!(current.child.try_wait(), Ok(None));
            if alive && current.settings == settings {
                return Ok(());
            }
        }
    }
    state.stop_child();
    cover_screen(&app, settings.paused);
    let mut child = spawn_script(&settings)
        .map_err(|_| DesktopError::Unavailable("the exam guard could not start".into()))?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    if let Some(stdout) = child.stdout.take() {
        let seen = seen.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                record(&seen, &line);
            }
            if let Ok(mut seen) = seen.lock() {
                seen.exited = true;
            }
        });
    }
    let mut running = state
        .running
        .lock()
        .map_err(|_| DesktopError::Unavailable("exam guard is unavailable".into()))?;
    *running = Some(Running {
        child,
        settings,
        seen,
    });
    Ok(())
}

/// What the guard saw since the last call. `running: false` means the script
/// stopped; the exam page reports that and applies the settings again.
#[tauri::command]
pub fn exam_guard_report(state: tauri::State<'_, ExamGuard>) -> GuardReport {
    let Ok(mut running) = state.running.lock() else {
        return GuardReport {
            running: false,
            foreground_app: None,
            violations: Vec::new(),
        };
    };
    let Some(current) = running.as_mut() else {
        return GuardReport {
            running: false,
            foreground_app: None,
            violations: Vec::new(),
        };
    };
    let alive = matches!(current.child.try_wait(), Ok(None));
    let Ok(mut seen) = current.seen.lock() else {
        return GuardReport {
            running: alive,
            foreground_app: None,
            violations: Vec::new(),
        };
    };
    let mut violations = std::mem::take(&mut seen.violations);
    if !alive || seen.exited {
        violations.push(Violation {
            kind: "guard-failed".into(),
            detail: "the exam guard stopped".into(),
        });
    }
    GuardReport {
        running: alive && !seen.exited,
        foreground_app: seen.foreground.clone(),
        violations,
    }
}

/// Ends exam mode on this computer: the guard stops and Sanser may quit.
#[tauri::command]
pub fn exam_guard_stop(app: tauri::AppHandle, state: tauri::State<'_, ExamGuard>) {
    state.stop_child();
    state.locked.store(false, Ordering::SeqCst);
    cover_screen(&app, false);
}

/// Paused: the Sanser window fills the screen above everything else.
fn cover_screen(app: &tauri::AppHandle, cover: bool) {
    use tauri::Manager;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.set_always_on_top(cover);
            let _ = window.set_fullscreen(cover);
            if cover {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
    });
}

/// One JPEG of the main screen as base64, sized for the supervisor grid.
#[tauri::command]
pub async fn exam_capture_screen(large: bool) -> Result<String, DesktopError> {
    tauri::async_runtime::spawn_blocking(move || capture(if large { 1280 } else { 960 }))
        .await
        .map_err(|_| DesktopError::Unavailable("screenshot failed".into()))?
}

#[cfg(target_os = "macos")]
fn capture(width: u32) -> Result<String, DesktopError> {
    use base64::Engine;
    let failed = || DesktopError::Unavailable("screenshot failed".into());
    let directory = std::env::temp_dir().join(format!("sanser-exam-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).map_err(|_| failed())?;
    let raw = directory.join("screen.jpg");
    let small = directory.join("small.jpg");
    let result = (|| {
        // -x: no sound. -m: the main display only.
        let status = Command::new("/usr/sbin/screencapture")
            .args(["-x", "-m", "-t", "jpg"])
            .arg(&raw)
            .status()
            .map_err(|_| failed())?;
        if !status.success() {
            return Err(failed());
        }
        for (size, quality) in [(width, "45"), (720, "35")] {
            let status = Command::new("/usr/bin/sips")
                .args([
                    "-Z",
                    &size.to_string(),
                    "-s",
                    "format",
                    "jpeg",
                    "-s",
                    "formatOptions",
                    quality,
                ])
                .arg(&raw)
                .arg("--out")
                .arg(&small)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|_| failed())?;
            if !status.success() {
                return Err(failed());
            }
            let bytes = std::fs::read(&small).map_err(|_| failed())?;
            if bytes.len() <= 170_000 {
                return Ok(base64::engine::general_purpose::STANDARD.encode(bytes));
            }
        }
        Err(failed())
    })();
    let _ = std::fs::remove_dir_all(&directory);
    result
}

#[cfg(windows)]
fn capture(width: u32) -> Result<String, DesktopError> {
    let failed = || DesktopError::Unavailable("screenshot failed".into());
    for (size, quality) in [(width, 45), (720, 35)] {
        let output = powershell(include_str!("exam_guard/screenshot_windows.ps1"))
            .env("SANSER_SHOT_WIDTH", size.to_string())
            .env("SANSER_SHOT_QUALITY", quality.to_string())
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| failed())?;
        let encoded = String::from_utf8(output.stdout).map_err(|_| failed())?;
        let encoded = encoded.trim();
        if !output.status.success() || encoded.is_empty() {
            return Err(failed());
        }
        if encoded.len() <= 225_000 {
            return Ok(encoded.to_owned());
        }
    }
    Err(failed())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn capture(_width: u32) -> Result<String, DesktopError> {
    Err(DesktopError::Unavailable(
        "exam mode needs macOS or Windows".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_lines_become_a_deduplicated_report() {
        let seen = Mutex::new(Seen::default());
        record(&seen, r#"{"t":"ready"}"#);
        record(&seen, r#"{"t":"front","d":"Microsoft Word"}"#);
        record(&seen, r#"{"t":"left-app","d":"Safari"}"#);
        record(&seen, r#"{"t":"left-app","d":"Safari"}"#);
        record(&seen, r#"{"t":"ai-app","d":"ChatGPT\u0007"}"#);
        record(&seen, "not json");
        record(&seen, r#"{"t":"made-up","d":"x"}"#);
        let seen = seen.lock().map_err(|_| "poisoned");
        let Ok(seen) = seen else { return };
        assert_eq!(seen.foreground.as_deref(), Some("Microsoft Word"));
        assert_eq!(
            seen.violations,
            [
                Violation {
                    kind: "left-app".into(),
                    detail: "Safari".into()
                },
                Violation {
                    kind: "ai-app".into(),
                    detail: "ChatGPT".into()
                },
            ]
        );
    }

    #[test]
    fn ai_titles_catch_ai_sites_but_not_ordinary_pages() -> Result<(), regex::Error> {
        let patterns = AI_TITLES
            .iter()
            .map(|source| {
                regex::RegexBuilder::new(source)
                    .case_insensitive(true)
                    .build()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let matches = |title: &str| patterns.iter().any(|pattern| pattern.is_match(title));
        assert!(matches("ChatGPT"));
        assert!(matches("Claude"));
        assert!(matches("New chat - Claude - Google Chrome"));
        assert!(matches("\u{200e}Google Gemini"));
        assert!(matches("Gemini - Microsoft Edge"));
        assert!(!matches("Claude Monet - Wikipedia"));
        assert!(!matches("Gemini constellation facts"));
        assert!(!matches("Bài kiểm tra Toán 10A - Google Forms"));
        Ok(())
    }

    #[test]
    fn settings_reach_the_script_with_this_process_and_the_lists() {
        let config = script_config(&GuardSettings {
            paused: false,
            allowed_apps: vec!["WINWORD.EXE".into()],
            block_ai: true,
        });
        assert_eq!(config["allowed"][0], "WINWORD.EXE");
        assert_eq!(config["selfPid"], std::process::id());
        assert_eq!(
            config["aiTitles"].as_array().map(Vec::len),
            Some(AI_TITLES.len())
        );
    }
}
