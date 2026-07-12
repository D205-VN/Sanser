//! Candidate pair nomination logic.
//!
//! The controlling peer selects the best pair and notifies the controlled peer.

use serde::Serialize;

/// Result of the nomination decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Nomination {
    /// ID of the nominated candidate pair.
    pub pair_id: String,
    /// Generation in which the nomination was made.
    pub generation: u32,
    /// Measured RTT in milliseconds on the nominated pair.
    pub rtt_ms: u64,
}

/// Policy for when to finalize nomination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NominationPolicy {
    /// Nominate the first pair that succeeds (aggressive).
    Aggressive,
    /// Wait for all checks to complete, then pick the best (regular).
    Regular,
    /// Wait for a configured duration after first success before nominating.
    Delayed { wait_ms: u64 },
}

impl Default for NominationPolicy {
    fn default() -> Self {
        // Plan section 12 says: "A route thành công sớm có thể được sử dụng
        // ngay, nhưng hệ thống tiếp tục kiểm tra ngắn hạn để tìm route tốt hơn."
        Self::Delayed { wait_ms: 500 }
    }
}
