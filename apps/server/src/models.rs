use serde::{Deserialize, Serialize};

use crate::config::NetworkMode;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthTokens {
    pub token_type: &'static str,
    pub access_token: String,
    pub access_expires_at: i64,
    pub refresh_token: String,
    pub refresh_expires_at: i64,
    pub account: Account,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub os_version: String,
    pub gpu: String,
    pub sanser_version: String,
    pub protocol_version: i64,
    pub online: bool,
    pub streaming: bool,
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<i64>,
    pub codecs: Vec<String>,
    pub native_transport: bool,
    pub webrtc: bool,
    pub audio: bool,
    pub gamepad: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSession {
    pub id: String,
    pub requester_device_id: String,
    pub host_device_id: String,
    pub state: String,
    pub network_mode: NetworkMode,
    pub quality_profile: String,
    pub requested_codec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_transport: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_ready_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disconnect_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEnvelope {
    pub id: String,
    #[serde(skip_serializing)]
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: i64,
}
