use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Auto,
    Direct,
    Relay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Snv2,
    WebRtcDirect,
    WebRtcRelayUdp,
    WebRtcRelayTcp,
    WebRtcRelayTls,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityProfile {
    #[default]
    Auto,
    Competitive,
    Balanced,
    Quality,
    Custom,
}
