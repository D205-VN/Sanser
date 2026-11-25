//! Start Sanser when the user signs in to this computer.
//!
//! Per-user only and opt-in: a `LaunchAgent` in `~/Library/LaunchAgents` on
//! macOS, the `HKCU\…\Run` value on Windows. Neither needs administrator
//! rights. The entry starts Sanser with `--autostart`, which keeps the window
//! in the tray on Windows; going online still follows the host preference.

use serde::Serialize;

use crate::error::DesktopError;

pub const AUTOSTART_ARG: &str = "--autostart";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginItemStatus {
    pub available: bool,
    pub enabled: bool,
}

/// True when this process was started by the login item.
pub fn launched_at_login() -> bool {
    std::env::args().any(|arg| arg == AUTOSTART_ARG)
}

fn executable() -> Result<std::path::PathBuf, DesktopError> {
    std::env::current_exe()
        .map_err(|_| DesktopError::Process("Unable to locate the Sanser app".into()))
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::{Path, PathBuf};

    use super::{AUTOSTART_ARG, DesktopError};

    const LABEL: &str = "com.sanser.desktop.login";

    fn agent_path() -> Result<PathBuf, DesktopError> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| DesktopError::Process("Unable to find the home folder".into()))?;
        Ok(PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    pub fn plist(executable: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\
\t<key>Label</key><string>{LABEL}</string>\n\
\t<key>ProgramArguments</key><array><string>{}</string><string>{AUTOSTART_ARG}</string></array>\n\
\t<key>RunAtLoad</key><true/>\n\
\t<key>LimitLoadToSessionType</key><string>Aqua</string>\n\
\t<key>ProcessType</key><string>Interactive</string>\n\
</dict>\n</plist>\n",
            escape(&executable.to_string_lossy())
        )
    }

    pub fn enabled() -> bool {
        agent_path().is_ok_and(|path| path.is_file())
    }

    pub fn set(enabled: bool, executable: &Path) -> Result<(), DesktopError> {
        let path = agent_path()?;
        if enabled {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder).map_err(|_| {
                    DesktopError::Process("Unable to create the LaunchAgents folder".into())
                })?;
            }
            std::fs::write(&path, plist(executable)).map_err(|_| {
                DesktopError::Process("Unable to save the start-at-login setting".into())
            })
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(DesktopError::Process(
                    "Unable to remove the start-at-login setting".into(),
                )),
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use super::{AUTOSTART_ARG, DesktopError};

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "Sanser";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn reg(args: &[&str]) -> bool {
        Command::new("reg")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|status| status.success())
    }

    pub fn enabled() -> bool {
        reg(&["query", RUN_KEY, "/v", VALUE])
    }

    pub fn set(enabled: bool, executable: &Path) -> Result<(), DesktopError> {
        let ok = if enabled {
            let command = format!("\"{}\" {AUTOSTART_ARG}", executable.to_string_lossy());
            reg(&[
                "add",
                RUN_KEY,
                "/v",
                VALUE,
                "/t",
                "REG_SZ",
                "/d",
                command.as_str(),
                "/f",
            ])
        } else {
            !self::enabled() || reg(&["delete", RUN_KEY, "/v", VALUE, "/f"])
        };
        if ok {
            Ok(())
        } else {
            Err(DesktopError::Process(
                "Unable to change the start-at-login setting".into(),
            ))
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use std::path::Path;

    use super::DesktopError;

    pub fn enabled() -> bool {
        false
    }

    pub fn set(_: bool, _: &Path) -> Result<(), DesktopError> {
        Err(DesktopError::Process(
            "Start at login is not available on this system".into(),
        ))
    }
}

fn status() -> LoginItemStatus {
    LoginItemStatus {
        available: cfg!(any(target_os = "macos", windows)),
        enabled: platform::enabled(),
    }
}

#[tauri::command]
pub fn login_item_status() -> LoginItemStatus {
    status()
}

#[tauri::command]
pub fn set_login_item(enabled: bool) -> Result<LoginItemStatus, DesktopError> {
    platform::set(enabled, &executable()?)?;
    Ok(status())
}

/// Keeps an enabled entry pointing at this copy of the app after it moves or updates.
pub fn refresh() {
    if platform::enabled()
        && let Ok(executable) = executable()
        && let Err(error) = platform::set(true, &executable)
    {
        eprintln!("Unable to refresh the start-at-login entry: {error}");
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn launch_agent_starts_this_app_with_the_autostart_flag_and_escapes_paths() {
        let plist = super::platform::plist(std::path::Path::new(
            "/Applications/Sanser & Co.app/Contents/MacOS/Sanser",
        ));
        assert!(plist.contains("<string>/Applications/Sanser &amp; Co.app/Contents/MacOS/Sanser</string><string>--autostart</string>"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
        assert!(plist.contains("com.sanser.desktop.login"));
        // launchd must accept the file: plutil ships with every macOS.
        let path = std::env::temp_dir().join(format!("sanser-login-{}.plist", std::process::id()));
        std::fs::write(&path, &plist).ok();
        let lint = std::process::Command::new("plutil")
            .arg("-lint")
            .arg(&path)
            .status();
        std::fs::remove_file(&path).ok();
        assert!(
            lint.is_ok_and(|status| status.success()),
            "valid property list"
        );
    }
}
