//! NAT keepalive and binding refresh.

use serde::Serialize;

/// Keepalive configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeepaliveConfig {
    /// Interval when streaming is active (milliseconds).
    pub active_interval_ms: u64,
    /// Interval when idle (milliseconds).
    pub idle_interval_ms: u64,
    /// Fast interval used when NAT is detected as aggressive.
    pub aggressive_interval_ms: u64,
    /// Number of missed keepalives before triggering reconnect.
    pub max_missed: u32,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            active_interval_ms: 2_000,
            idle_interval_ms: 5_000,
            aggressive_interval_ms: 1_000,
            max_missed: 3,
        }
    }
}

/// Keepalive session state.
#[derive(Clone, Debug, Default, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeepaliveState {
    pub sent: u64,
    pub received: u64,
    pub missed_consecutive: u32,
    pub last_sent_ms: u64,
    pub last_received_ms: u64,
}

impl KeepaliveState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_sent(&mut self, now_ms: u64) {
        self.sent += 1;
        self.last_sent_ms = now_ms;
        self.missed_consecutive += 1;
    }

    pub fn handle_received(&mut self, now_ms: u64) {
        self.received += 1;
        self.last_received_ms = now_ms;
        self.missed_consecutive = 0;
    }

    #[must_use]
    pub fn is_disconnected(&self, config: &KeepaliveConfig) -> bool {
        self.missed_consecutive >= config.max_missed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keepalive_state_transitions() {
        let mut state = KeepaliveState::new();
        let config = KeepaliveConfig::default();

        assert!(!state.is_disconnected(&config));

        state.handle_sent(1000);
        assert_eq!(state.sent, 1);
        assert_eq!(state.missed_consecutive, 1);

        state.handle_sent(3000);
        state.handle_sent(5000);
        assert_eq!(state.missed_consecutive, 3);
        assert!(state.is_disconnected(&config));

        state.handle_received(6000);
        assert_eq!(state.missed_consecutive, 0);
        assert!(!state.is_disconnected(&config));
    }
}
