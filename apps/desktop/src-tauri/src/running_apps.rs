//! Applications the host can pick in Settings → Host → Blocked apps.
//! Read only: nothing is started, stopped or sent anywhere.

use serde::{Deserialize, Serialize};

use crate::error::DesktopError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunningApp {
    /// macOS bundle identifier or Windows executable file name.
    pub id: String,
    pub name: String,
}

#[tauri::command]
pub async fn list_running_apps() -> Result<Vec<RunningApp>, DesktopError> {
    tauri::async_runtime::spawn_blocking(list)
        .await
        .map_err(|_| DesktopError::Unavailable("unable to list running apps".into()))?
}

fn tidy(mut apps: Vec<RunningApp>) -> Vec<RunningApp> {
    apps.retain(|app| crate::engine::valid_blocked_app_id(&app.id));
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps.dedup_by(|a, b| a.id.eq_ignore_ascii_case(&b.id));
    apps.truncate(200);
    apps
}

#[cfg(target_os = "macos")]
fn list() -> Result<Vec<RunningApp>, DesktopError> {
    // NSWorkspace through JavaScript for Automation: regular (Dock) apps only.
    // It reads this Mac's process list and needs no Automation permission.
    const SCRIPT: &str = "ObjC.import('AppKit');var a=$.NSWorkspace.sharedWorkspace.runningApplications;var o=[];\
for(var i=0;i<a.count;i++){var x=a.objectAtIndex(i);if(x.activationPolicy==0&&!x.bundleIdentifier.isNil()){\
o.push({id:x.bundleIdentifier.js,name:x.localizedName.isNil()?x.bundleIdentifier.js:x.localizedName.js});}}JSON.stringify(o)";
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", SCRIPT])
        .output()
        .map_err(|_| DesktopError::Unavailable("unable to list running apps".into()))?;
    if !output.status.success() {
        return Err(DesktopError::Unavailable(
            "unable to list running apps".into(),
        ));
    }
    let apps: Vec<RunningApp> = serde_json::from_slice(&output.stdout)
        .map_err(|_| DesktopError::Unavailable("unable to read the running app list".into()))?;
    Ok(tidy(
        apps.into_iter()
            .filter(|app| app.id != "com.sanser.desktop")
            .collect(),
    ))
}

#[cfg(windows)]
fn list() -> Result<Vec<RunningApp>, DesktopError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|_| DesktopError::Unavailable("unable to list running apps".into()))?;
    if !output.status.success() {
        return Err(DesktopError::Unavailable(
            "unable to list running apps".into(),
        ));
    }
    Ok(tidy(parse_tasklist(&String::from_utf8_lossy(
        &output.stdout,
    ))))
}

#[cfg(not(any(target_os = "macos", windows)))]
#[allow(clippy::unnecessary_wraps)]
fn list() -> Result<Vec<RunningApp>, DesktopError> {
    Ok(tidy(Vec::new()))
}

/// System processes the remote person never opens; hidden from the picker.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
const WINDOWS_SYSTEM: &[&str] = &[
    "system idle process",
    "system",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "services.exe",
    "lsass.exe",
    "svchost.exe",
    "fontdrvhost.exe",
    "winlogon.exe",
    "dwm.exe",
    "conhost.exe",
    "sihost.exe",
    "taskhostw.exe",
    "ctfmon.exe",
    "runtimebroker.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "textinputhost.exe",
    "dllhost.exe",
    "wmiprvse.exe",
    "spoolsv.exe",
    "memory compression",
    "secure system",
    "lsaiso.exe",
    "audiodg.exe",
    "tasklist.exe",
    "sanser.exe",
    "sanser-host-windows.exe",
];

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn parse_tasklist(text: &str) -> Vec<RunningApp> {
    text.lines()
        .filter_map(|line| {
            let name = line.trim().strip_prefix('"')?.split('"').next()?.trim();
            let lower = name.to_ascii_lowercase();
            let executable = std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
            if !executable || WINDOWS_SYSTEM.contains(&lower.as_str()) {
                return None;
            }
            Some(RunningApp {
                id: name.to_owned(),
                name: name[..name.len() - 4].to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasklist_lines_become_unique_user_apps() {
        let text = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
\"svchost.exe\",\"1000\",\"Services\",\"0\",\"9,000 K\"\r\n\
\"chrome.exe\",\"4200\",\"Console\",\"1\",\"120,000 K\"\r\n\
\"chrome.exe\",\"4300\",\"Console\",\"1\",\"80,000 K\"\r\n\
\"Discord.exe\",\"5000\",\"Console\",\"1\",\"90,000 K\"\r\n\
garbage line\r\n";
        let apps = tidy(parse_tasklist(text));
        assert_eq!(
            apps,
            [
                RunningApp {
                    id: "chrome.exe".into(),
                    name: "chrome".into()
                },
                RunningApp {
                    id: "Discord.exe".into(),
                    name: "Discord".into()
                },
            ]
        );
    }
}
