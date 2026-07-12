//! Connectivity check types for candidate pair verification.
//!
//! Phase 1 defines the pair model and authenticated probe format.
//! Actual UDP probe sending/receiving is added in Phase 4.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use hmac::{Hmac, Mac};
use sha2::Sha256;
type HmacSha256 = Hmac<Sha256>;

/// Magic bytes at the start of every connectivity probe packet.
pub const PROBE_MAGIC: [u8; 4] = [0x53, 0x4E, 0x50, 0x32]; // "SNP2"

/// Current probe protocol version.
pub const PROBE_VERSION: u8 = 1;

/// Minimum probe packet size (magic + version + fields + HMAC).
pub const PROBE_MIN_SIZE: usize = 64;

/// A candidate pair formed from one local and one remote candidate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePair {
    /// Unique identifier for this pair.
    pub pair_id: String,
    /// Local candidate endpoint.
    pub local: SocketAddr,
    /// Remote candidate endpoint.
    pub remote: SocketAddr,
    /// Local candidate ID.
    pub local_candidate_id: String,
    /// Remote candidate ID.
    pub remote_candidate_id: String,
    /// Combined priority (higher = preferred).
    pub priority: u64,
    /// Current state of this pair.
    pub state: PairState,
}

/// Connectivity check state for a candidate pair.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PairState {
    /// Not yet checked.
    #[default]
    Waiting,
    /// Probe sent, waiting for response.
    InProgress,
    /// Both directions confirmed.
    Succeeded,
    /// Check failed (timeout or rejection).
    Failed,
    /// Nominated as the selected pair.
    Nominated,
}

/// Schedule entry for the hole-punch timing sequence from plan section 11.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PunchScheduleEntry {
    /// Delay from T0 in milliseconds.
    pub delay_ms: u64,
}

/// Default punch schedule from plan section 11.
pub const PUNCH_SCHEDULE: [PunchScheduleEntry; 8] = [
    PunchScheduleEntry { delay_ms: 0 },
    PunchScheduleEntry { delay_ms: 50 },
    PunchScheduleEntry { delay_ms: 100 },
    PunchScheduleEntry { delay_ms: 200 },
    PunchScheduleEntry { delay_ms: 400 },
    PunchScheduleEntry { delay_ms: 800 },
    PunchScheduleEntry { delay_ms: 1200 },
    PunchScheduleEntry { delay_ms: 1800 },
];

/// Authenticated probe packet layout.
///
/// ```text
/// Offset  Size  Field
/// 0       4     Magic ("SNP2")
/// 4       1     Protocol version
/// 5       16    Session ID (UUID bytes)
/// 21      8     Sender device ID hash (truncated SHA-256)
/// 29      16    Candidate pair ID hash
/// 45      4     Transaction ID
/// 49      8     Timestamp (Unix millis)
/// 57      4     Nonce
/// 61      3     Reserved
/// 64      32    HMAC-SHA256 authentication tag
/// ```
pub const PROBE_TOTAL_SIZE: usize = 96;

/// Builds the pair ID by combining local and remote candidate IDs.
#[must_use]
pub fn make_pair_id(local_id: &str, remote_id: &str) -> String {
    format!("{local_id}:{remote_id}")
}

/// Computes a deterministic 128-bit hash of the pair ID.
#[must_use]
pub fn compute_pair_hash(pair_id: &str) -> u128 {
    use sha2::Digest;
    let mut hasher = Sha256::new();
    hasher.update(pair_id.as_bytes());
    let result = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&result[0..16]);
    u128::from_be_bytes(bytes)
}

/// Computes the combined priority for a candidate pair.
///
/// Uses the ICE formula: `2^32 * min(G,D) + 2 * max(G,D) + (G > D ? 1 : 0)`
/// where G = controlling priority and D = controlled priority.
#[must_use]
pub fn pair_priority(controlling_priority: u32, controlled_priority: u32) -> u64 {
    let g = u64::from(controlling_priority);
    let d = u64::from(controlled_priority);
    let min = g.min(d);
    let max = g.max(d);
    let tie = if g > d { 1u64 } else { 0u64 };
    (1u64 << 32) * min + 2 * max + tie
}

/// Unpacked probe fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeFields {
    pub session_id: uuid::Uuid,
    pub sender_device_hash: u64,
    pub pair_hash: u128,
    pub transaction_id: u32,
    pub timestamp: u64,
    pub nonce: u32,
}

/// Builds the 96-byte authenticated connectivity probe packet.
///
/// # Errors
///
/// Returns a [`P2pError::Internal`] if HMAC initialization fails.
pub fn build_probe_packet(
    fields: &ProbeFields,
    key: &[u8],
) -> Result<[u8; PROBE_TOTAL_SIZE], crate::error::P2pError> {
    let mut packet = [0u8; PROBE_TOTAL_SIZE];
    packet[0..4].copy_from_slice(&PROBE_MAGIC);
    packet[4] = PROBE_VERSION;
    packet[5..21].copy_from_slice(fields.session_id.as_bytes());
    packet[21..29].copy_from_slice(&fields.sender_device_hash.to_be_bytes());
    packet[29..45].copy_from_slice(&fields.pair_hash.to_be_bytes());
    packet[45..49].copy_from_slice(&fields.transaction_id.to_be_bytes());
    packet[49..57].copy_from_slice(&fields.timestamp.to_be_bytes());
    packet[57..61].copy_from_slice(&fields.nonce.to_be_bytes());

    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| crate::error::P2pError::Internal {
            reason: format!("HMAC setup failed: {error}"),
        })?;
    mac.update(&packet[..64]);
    let tag = mac.finalize().into_bytes();
    packet[64..96].copy_from_slice(&tag);
    Ok(packet)
}

/// Parses and authenticates the 96-byte connectivity probe packet.
///
/// # Errors
///
/// Returns a [`P2pError::InvalidProbe`] if magic, version or length is incorrect,
/// or [`P2pError::AuthenticationFailed`] if the signature check fails.
pub fn parse_probe_packet(
    packet: &[u8],
    key: &[u8],
) -> Result<ProbeFields, crate::error::P2pError> {
    if packet.len() != PROBE_TOTAL_SIZE {
        return Err(crate::error::P2pError::InvalidProbe {
            reason: format!(
                "invalid size: expected {PROBE_TOTAL_SIZE}, got {}",
                packet.len()
            ),
        });
    }
    if packet[0..4] != PROBE_MAGIC {
        return Err(crate::error::P2pError::InvalidProbe {
            reason: "invalid magic".into(),
        });
    }
    if packet[4] != PROBE_VERSION {
        return Err(crate::error::P2pError::InvalidProbe {
            reason: format!("unsupported version: {}", packet[4]),
        });
    }

    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|error| crate::error::P2pError::Internal {
            reason: format!("HMAC setup failed: {error}"),
        })?;
    mac.update(&packet[..64]);
    mac.verify_slice(&packet[64..96])
        .map_err(|_| crate::error::P2pError::AuthenticationFailed {
            reason: "HMAC signature verification failed".into(),
        })?;

    let session_id = uuid::Uuid::from_slice(&packet[5..21]).map_err(|error| {
        crate::error::P2pError::InvalidProbe {
            reason: format!("invalid UUID: {error}"),
        }
    })?;

    let mut sdh_bytes = [0u8; 8];
    sdh_bytes.copy_from_slice(&packet[21..29]);
    let sender_device_hash = u64::from_be_bytes(sdh_bytes);

    let mut ph_bytes = [0u8; 16];
    ph_bytes.copy_from_slice(&packet[29..45]);
    let pair_hash = u128::from_be_bytes(ph_bytes);

    let mut ti_bytes = [0u8; 4];
    ti_bytes.copy_from_slice(&packet[45..49]);
    let transaction_id = u32::from_be_bytes(ti_bytes);

    let mut ts_bytes = [0u8; 8];
    ts_bytes.copy_from_slice(&packet[49..57]);
    let timestamp = u64::from_be_bytes(ts_bytes);

    let mut no_bytes = [0u8; 4];
    no_bytes.copy_from_slice(&packet[57..61]);
    let nonce = u32::from_be_bytes(no_bytes);

    Ok(ProbeFields {
        session_id,
        sender_device_hash,
        pair_hash,
        transaction_id,
        timestamp,
        nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_priority_symmetric_with_tiebreaker() {
        let p1 = pair_priority(100, 50);
        let p2 = pair_priority(50, 100);
        // Same min/max, but tiebreaker differs
        assert_ne!(p1, p2);
    }

    #[test]
    fn higher_controlling_wins_tiebreaker() {
        let p = pair_priority(100, 50);
        assert_eq!(p % 2, 1, "controlling > controlled should set tiebreaker");
    }

    #[test]
    fn pair_id_combines_both_candidates() {
        let id = make_pair_id("host-1", "srflx-2");
        assert_eq!(id, "host-1:srflx-2");
    }

    #[test]
    fn punch_schedule_is_monotonically_increasing() {
        for window in PUNCH_SCHEDULE.windows(2) {
            assert!(window[0].delay_ms < window[1].delay_ms);
        }
    }

    #[test]
    fn punch_schedule_does_not_exceed_two_seconds() {
        assert!(PUNCH_SCHEDULE.last().map_or(false, |e| e.delay_ms <= 2000));
    }

    #[test]
    fn test_probe_packet_roundtrip() {
        let fields = ProbeFields {
            session_id: uuid::Uuid::new_v4(),
            sender_device_hash: 0x1234567890abcdef,
            pair_hash: 0x9876543210fedcbaabcdef0123456789,
            transaction_id: 42,
            timestamp: 1712970000000,
            nonce: 1001,
        };
        let key = b"mysecretkey";
        let packet = build_probe_packet(&fields, key).unwrap();
        let parsed = parse_probe_packet(&packet, key).unwrap();
        assert_eq!(fields, parsed);
    }

    #[test]
    fn test_probe_packet_invalid_hmac_rejected() {
        let fields = ProbeFields {
            session_id: uuid::Uuid::new_v4(),
            sender_device_hash: 1,
            pair_hash: 2,
            transaction_id: 3,
            timestamp: 4,
            nonce: 5,
        };
        let key = b"correctkey";
        let packet = build_probe_packet(&fields, key).unwrap();
        assert!(parse_probe_packet(&packet, b"wrongkey").is_err());
    }
}
