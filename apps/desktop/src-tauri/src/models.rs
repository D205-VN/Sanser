use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const SANSER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROTOCOL_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EngineKind {
    Host,
    Client,
    LocalServer,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    Auto,
    Direct,
    Relay,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    Auto,
    H264,
    Hevc,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchEngineRequest {
    pub kind: EngineKind,
    pub session_id: Option<String>,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub codec: VideoCodec,
    pub fps: u16,
    pub bitrate_kbps: u32,
    pub width: u16,
    pub height: u16,
    pub network_mode: NetworkMode,
    #[serde(default)]
    pub audio_enabled: bool,
    #[serde(default)]
    pub input_enabled: bool,
    #[serde(default)]
    pub relative_mouse: bool,
    #[serde(default)]
    pub session_token: Option<String>,
}

impl Drop for LaunchEngineRequest {
    fn drop(&mut self) {
        if let Some(token) = self.session_token.as_mut() {
            token.zeroize();
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityState {
    Available,
    Unavailable,
    Planned,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub state: CapabilityState,
    pub reason: Option<String>,
}

impl Capability {
    pub fn available() -> Self {
        Self {
            state: CapabilityState::Available,
            reason: None,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Unavailable,
            reason: Some(reason.into()),
        }
    }

    pub fn planned(reason: impl Into<String>) -> Self {
        Self {
            state: CapabilityState::Planned,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilities {
    pub desktop_shell: Capability,
    pub secure_storage: Capability,
    pub host_engine: Capability,
    pub client_engine: Capability,
    pub local_server: Capability,
    pub local_discovery: Capability,
    pub web_rtc: Capability,
    pub native_direct: Capability,
    pub native_snv2: Capability,
    pub gamepad: Capability,
    pub clipboard: Capability,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub kind: EngineKind,
    pub installed: bool,
    pub running: bool,
    pub process_id: Option<u32>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub platform: String,
    pub version: &'static str,
    pub protocol_version: u8,
    pub capabilities: RuntimeCapabilities,
    pub engines: Vec<EngineStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    Auto,
    Competitive,
    Balanced,
    Quality,
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Auto,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "2160p")]
    P2160,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamPreferences {
    pub profile: QualityProfile,
    pub codec: VideoCodec,
    pub resolution: Resolution,
    pub fps: u16,
    pub bitrate_mbps: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct HostPreferences {
    pub auto_online: bool,
    pub auto_accept_own_devices: bool,
    pub audio_enabled: bool,
    pub input_enabled: bool,
    pub clipboard_enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseMode {
    Auto,
    Absolute,
    Relative,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputPreferences {
    pub mouse_mode: MouseMode,
    pub polling_rate: u16,
    pub release_shortcut: String,
    pub gamepad_enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    Vi,
    En,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub schema_version: u8,
    pub server_url: String,
    pub network_mode: NetworkMode,
    pub stream: StreamPreferences,
    pub host: HostPreferences,
    pub input: InputPreferences,
    pub locale: Locale,
    pub start_minimized: bool,
    pub diagnostics_enabled: bool,
    pub pinned_device_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsExport {
    pub path: String,
}
