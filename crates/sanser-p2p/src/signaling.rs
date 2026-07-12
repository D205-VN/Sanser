//! Signaling message types exchanged through the server WebSocket.
//!
//! These types are serialized to JSON and relayed between two peers via the
//! Sanser signaling server. The server validates but does not interpret the
//! payload beyond access control and size limits.

use crate::candidate::P2pCandidate;
use serde::{Deserialize, Serialize};

/// All P2P signaling message types routed through the server.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum P2pSignal {
    /// Initial handshake announcing P2P capability.
    #[serde(rename = "p2p.hello")]
    Hello(P2pHello),

    /// A batch of gathered candidates.
    #[serde(rename = "p2p.candidates")]
    Candidates(P2pCandidateBatch),

    /// All candidates have been gathered for this generation.
    #[serde(rename = "p2p.gatheringComplete")]
    GatheringComplete(P2pGatheringComplete),

    /// This peer is ready to begin hole punching.
    #[serde(rename = "p2p.punchReady")]
    PunchReady(P2pPunchReady),

    /// Nominate a candidate pair for media transport.
    #[serde(rename = "p2p.nominate")]
    Nominate(P2pNominate),

    /// Acknowledge the nominated pair and begin streaming.
    #[serde(rename = "p2p.selected")]
    Selected(P2pSelected),

    /// Notify that the active path has changed.
    #[serde(rename = "p2p.pathChanged")]
    PathChanged(P2pPathChanged),

    /// P2P negotiation has failed.
    #[serde(rename = "p2p.failed")]
    Failed(P2pFailed),

    /// P2P session is closed.
    #[serde(rename = "p2p.closed")]
    Closed(P2pClosed),
}

/// Envelope that wraps a signaling message with routing metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalEnvelope {
    pub session_id: String,
    pub target_device_id: String,
    pub payload: P2pSignal,
}

// ─── Message payloads ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pHello {
    /// Protocol version for the P2P handshake.
    pub p2p_version: u8,
    /// Whether this peer will act as the controlling (nominating) side.
    pub controlling: bool,
    /// Ephemeral public key for key exchange (base64-encoded X25519).
    /// Populated in Phase 6 (security).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_public_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pCandidateBatch {
    /// Monotonically increasing generation counter. A new generation
    /// indicates a network change and obsoletes all prior candidates.
    pub generation: u32,
    /// Gathered candidates (max 16 per message).
    pub candidates: Vec<P2pCandidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pGatheringComplete {
    pub generation: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pPunchReady {
    pub generation: u32,
    /// Suggested synchronized start time (Unix millis). The server may
    /// adjust this to account for clock skew.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pNominate {
    pub generation: u32,
    /// ID of the nominated candidate pair.
    pub pair_id: String,
    /// Measured RTT on the nominated pair in milliseconds.
    pub rtt_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pSelected {
    pub generation: u32,
    pub pair_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pPathChanged {
    pub generation: u32,
    pub old_pair_id: String,
    pub new_pair_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pFailed {
    pub generation: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pClosed {
    pub reason: String,
}

/// Role assigned by the server to each peer in a session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum P2pRole {
    /// Responsible for nominating the final candidate pair.
    Controlling,
    /// Accepts the nomination from the controlling peer.
    Controlled,
}

/// Session credential returned by the server for P2P negotiation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pSessionCredential {
    pub session_id: String,
    pub device_id: String,
    pub peer_device_id: String,
    pub role: P2pRole,
    pub authorization_token: String,
    pub expires_at: i64,
    pub signaling_url: String,
    pub stun_urls: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{CandidateType, MappingProtocol, TransportProtocol};
    use std::net::IpAddr;

    #[test]
    fn hello_roundtrips_through_json() {
        let signal = P2pSignal::Hello(P2pHello {
            p2p_version: 1,
            controlling: true,
            ephemeral_public_key: None,
        });
        let json = serde_json::to_string(&signal);
        assert!(json.is_ok(), "serialize failed: {:?}", json.err());
        let parsed: Result<P2pSignal, _> = serde_json::from_str(
            json.as_ref()
                .unwrap_or_else(|e| panic!("unwrap failed: {e:?}")),
        );
        assert!(parsed.is_ok(), "deserialize failed: {:?}", parsed.err());
    }

    #[test]
    fn candidates_roundtrip_through_json() {
        let candidate = P2pCandidate {
            id: "host-1".into(),
            candidate_type: CandidateType::Host,
            address: IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 10)),
            port: 50000,
            protocol: TransportProtocol::Udp,
            interface_index: Some(1),
            mapping_protocol: MappingProtocol::None,
            priority: 120,
            foundation: "host-eth0".into(),
        };
        let signal = P2pSignal::Candidates(P2pCandidateBatch {
            generation: 1,
            candidates: vec![candidate],
        });
        let json = serde_json::to_string(&signal);
        assert!(json.is_ok(), "serialize failed: {:?}", json.err());
    }

    #[test]
    fn credential_has_required_fields() {
        let cred = P2pSessionCredential {
            session_id: "sess-1".into(),
            device_id: "dev-1".into(),
            peer_device_id: "dev-2".into(),
            role: P2pRole::Controlling,
            authorization_token: "token123".into(),
            expires_at: 1_700_000_000,
            signaling_url: "wss://example.com/ws".into(),
            stun_urls: vec!["stun:stun.l.google.com:19302".into()],
        };
        let json = serde_json::to_string(&cred);
        assert!(json.is_ok());
    }
}
