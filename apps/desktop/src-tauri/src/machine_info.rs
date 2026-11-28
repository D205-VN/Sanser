//! What this computer is, for the stream advisor: CPU, memory and the main
//! screen with its refresh rate. Read with the system's own tools, cached
//! for ten minutes, and empty where the platform does not say.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const CACHE_FOR: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_cores: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_gb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_hz: Option<u32>,
}

static CACHE: Mutex<Option<(Instant, MachineInfo)>> = Mutex::new(None);

#[tauri::command]
pub async fn machine_info() -> MachineInfo {
    if let Ok(cache) = CACHE.lock()
        && let Some((at, info)) = cache.as_ref()
        && at.elapsed() < CACHE_FOR
    {
        return info.clone();
    }
    let info = tauri::async_runtime::spawn_blocking(read)
        .await
        .unwrap_or_default();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), info.clone()));
    }
    info
}

#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.chars().take(80).collect())
}

/// `sysctl -n machdep.cpu.brand_string hw.ncpu hw.memsize`
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_sysctl(output: &str, info: &mut MachineInfo) {
    let mut lines = output.lines();
    info.cpu_model = lines.next().and_then(text);
    info.cpu_cores = lines.next().and_then(|line| line.trim().parse().ok());
    info.memory_gb = lines
        .next()
        .and_then(|line| line.trim().parse::<u64>().ok())
        .map(|bytes| u32::try_from(bytes.div_ceil(1 << 30)).unwrap_or(u32::MAX));
}

/// `system_profiler SPDisplaysDataType -json`: the main screen's pixels and rate.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
// The rate is finite and at least 20 Hz before it is rounded.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn parse_system_profiler(json: &Value, info: &mut MachineInfo) {
    let gpus = json["SPDisplaysDataType"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let screens = gpus.iter().flat_map(|gpu| {
        gpu["spdisplays_ndrvs"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(move |screen| (gpu["sppci_model"].as_str().map(str::to_owned), screen))
    });
    let mut chosen = None;
    for (gpu, screen) in screens {
        let main = screen["spdisplays_main"].as_str() == Some("spdisplays_yes");
        if chosen.is_none() || main {
            chosen = Some((gpu, screen));
        }
        if main {
            break;
        }
    }
    let Some((gpu, screen)) = chosen else {
        return;
    };
    info.gpu = gpu.as_deref().and_then(text);
    let pair = |value: &str| {
        let mut parts = value
            .split(|c: char| !c.is_ascii_digit() && c != '.')
            .filter(|part| !part.is_empty());
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    };
    let resolution = screen["_spdisplays_resolution"]
        .as_str()
        .unwrap_or_default();
    if let Some((width, height)) = screen["_spdisplays_pixels"]
        .as_str()
        .and_then(pair)
        .or_else(|| pair(resolution))
    {
        info.display_width = Some(width);
        info.display_height = Some(height);
    }
    info.refresh_hz = resolution
        .split_once('@')
        .and_then(|(_, rate)| {
            rate.trim()
                .trim_end_matches("Hz")
                .trim()
                .parse::<f64>()
                .ok()
        })
        .filter(|rate| rate.is_finite() && *rate >= 20.0)
        .map(|rate| rate.round() as u32);
}

/// The JSON the Windows script prints.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_windows(json: &Value) -> MachineInfo {
    let number = |key: &str| {
        json[key]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    MachineInfo {
        cpu_model: json["cpuModel"].as_str().and_then(text),
        cpu_cores: number("cpuCores"),
        memory_gb: number("memoryGb"),
        gpu: json["gpu"].as_str().and_then(text),
        display_width: number("displayWidth"),
        display_height: number("displayHeight"),
        refresh_hz: number("refreshHz").filter(|rate| *rate >= 20),
    }
}

#[cfg(target_os = "macos")]
fn read() -> MachineInfo {
    use std::process::Command;
    let mut info = MachineInfo::default();
    if let Ok(output) = Command::new("/usr/sbin/sysctl")
        .args(["-n", "machdep.cpu.brand_string", "hw.ncpu", "hw.memsize"])
        .output()
    {
        parse_sysctl(&String::from_utf8_lossy(&output.stdout), &mut info);
    }
    if let Ok(output) = Command::new("/usr/sbin/system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .output()
        && let Ok(json) = serde_json::from_slice::<Value>(&output.stdout)
    {
        parse_system_profiler(&json, &mut info);
    }
    info
}

#[cfg(windows)]
fn read() -> MachineInfo {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$p = Get-CimInstance Win32_Processor | Select-Object -First 1
$c = Get-CimInstance Win32_ComputerSystem
$v = Get-CimInstance Win32_VideoController | Where-Object { $_.CurrentHorizontalResolution } | Select-Object -First 1
[pscustomobject]@{
  cpuModel = ([string]$p.Name).Trim()
  cpuCores = [int]$c.NumberOfLogicalProcessors
  memoryGb = [int][math]::Ceiling($c.TotalPhysicalMemory / 1GB)
  gpu = ([string]$v.Name).Trim()
  displayWidth = [int]$v.CurrentHorizontalResolution
  displayHeight = [int]$v.CurrentVerticalResolution
  refreshHz = [int]$v.CurrentRefreshRate
} | ConvertTo-Json -Compress
"#;
    crate::exam_guard::powershell(SCRIPT)
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok())
        .map(|json| parse_windows(&json))
        .unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn read() -> MachineInfo {
    MachineInfo::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_mac_sysctl_and_the_main_screen() {
        let mut info = MachineInfo::default();
        parse_sysctl("Apple M2\n8\n8589934592\n", &mut info);
        parse_system_profiler(
            &json!({"SPDisplaysDataType": [{
                "sppci_model": "Apple M2",
                "spdisplays_ndrvs": [
                    {"_spdisplays_pixels": "1920 x 1080", "_spdisplays_resolution": "1920 x 1080 @ 75.00Hz"},
                    {"_spdisplays_pixels": "2940 x 1912", "_spdisplays_resolution": "1470 x 956 @ 120.00Hz",
                     "spdisplays_main": "spdisplays_yes"}
                ]
            }]}),
            &mut info,
        );
        assert_eq!(
            info,
            MachineInfo {
                cpu_model: Some("Apple M2".into()),
                cpu_cores: Some(8),
                memory_gb: Some(8),
                gpu: Some("Apple M2".into()),
                display_width: Some(2940),
                display_height: Some(1912),
                refresh_hz: Some(120),
            }
        );
    }

    #[test]
    fn reads_the_windows_script_and_drops_empty_values() {
        let info = parse_windows(&json!({
            "cpuModel": " AMD Ryzen 5 5600X 6-Core Processor ", "cpuCores": 12, "memoryGb": 32,
            "gpu": "NVIDIA GeForce RTX 3060", "displayWidth": 2560, "displayHeight": 1440, "refreshHz": 1
        }));
        assert_eq!(
            info.cpu_model.as_deref(),
            Some("AMD Ryzen 5 5600X 6-Core Processor")
        );
        assert_eq!(
            (info.display_width, info.display_height),
            (Some(2560), Some(1440))
        );
        assert_eq!(info.refresh_hz, None);
        assert_eq!(parse_windows(&json!({})), MachineInfo::default());
    }
}
