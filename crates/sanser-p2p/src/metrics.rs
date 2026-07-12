//! Runtime metrics collected during a P2P session.

use serde::Serialize;

/// Network quality snapshot sampled every 250–500 ms.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathMetricsSnapshot {
    /// Round-trip time in milliseconds.
    pub rtt_ms: f64,
    /// Jitter (RTT variance) in milliseconds.
    pub jitter_ms: f64,
    /// Packet loss ratio (0.0–1.0).
    pub packet_loss: f64,
    /// Packet reorder ratio (0.0–1.0).
    pub packet_reorder: f64,
    /// Observed send bitrate in kbps.
    pub send_bitrate_kbps: u32,
    /// Observed receive bitrate in kbps.
    pub recv_bitrate_kbps: u32,
}

/// Cumulative session-level counters.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCounters {
    /// Total packets sent.
    pub packets_sent: u64,
    /// Total packets received.
    pub packets_received: u64,
    /// Total bytes sent.
    pub bytes_sent: u64,
    /// Total bytes received.
    pub bytes_received: u64,
    /// Packets dropped due to late arrival.
    pub packets_dropped_late: u64,
    /// Packets dropped due to authentication failure.
    pub packets_dropped_auth: u64,
    /// Packets dropped as replay.
    pub packets_dropped_replay: u64,
    /// Number of NACK requests sent.
    pub nacks_sent: u64,
    /// Number of keyframe requests sent.
    pub keyframe_requests: u64,
    /// Number of successful reconnects.
    pub reconnect_count: u32,
    /// Number of route migrations.
    pub route_migration_count: u32,
}

/// Candidate gathering timing.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatheringMetrics {
    /// Time to enumerate local interfaces in milliseconds.
    pub interface_enum_ms: u64,
    /// Time for STUN binding in milliseconds (0 = not attempted).
    pub stun_ms: u64,
    /// Whether STUN succeeded.
    pub stun_success: bool,
    /// Time for PCP mapping in milliseconds (0 = not attempted).
    pub pcp_ms: u64,
    /// Whether PCP succeeded.
    pub pcp_success: bool,
    /// Time for NAT-PMP mapping in milliseconds (0 = not attempted).
    pub nat_pmp_ms: u64,
    /// Whether NAT-PMP succeeded.
    pub nat_pmp_success: bool,
    /// Time for UPnP mapping in milliseconds (0 = not attempted).
    pub upnp_ms: u64,
    /// Whether UPnP succeeded.
    pub upnp_success: bool,
    /// Total number of candidates gathered.
    pub candidates_gathered: u32,
    /// Total gathering duration in milliseconds.
    pub total_ms: u64,
}

/// Connectivity check metrics.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityMetrics {
    /// Total candidate pairs formed.
    pub pairs_total: u32,
    /// Pairs that succeeded.
    pub pairs_succeeded: u32,
    /// Pairs that failed.
    pub pairs_failed: u32,
    /// Pairs still in progress.
    pub pairs_in_progress: u32,
    /// Time from first check to nomination in milliseconds.
    pub time_to_nomination_ms: u64,
}

/// Aggregated P2P diagnostics suitable for sanitized export.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pDiagnostics {
    pub gathering: GatheringMetrics,
    pub connectivity: ConnectivityMetrics,
    pub path: PathMetricsSnapshot,
    pub counters: SessionCounters,
    /// Selected route kind description (e.g. "lan_ipv4", "stun_hole_punch").
    pub selected_route: Option<String>,
    /// Whether IPv6 is available on any interface.
    pub ipv6_available: bool,
    /// Firewall state as detected.
    pub firewall_state: FirewallState,
}

/// Detected firewall posture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirewallState {
    #[default]
    Unknown,
    Open,
    Filtered,
    Blocked,
}

pub struct AdaptiveBitrateController {
    current_bitrate_kbps: u32,
    min_bitrate_kbps: u32,
    max_bitrate_kbps: u32,
    rtt_history: Vec<f64>,
    loss_history: Vec<f64>,
}

impl AdaptiveBitrateController {
    pub fn new(min_kbps: u32, max_kbps: u32, initial_kbps: u32) -> Self {
        Self {
            current_bitrate_kbps: initial_kbps,
            min_bitrate_kbps: min_kbps,
            max_bitrate_kbps: max_kbps,
            rtt_history: Vec::new(),
            loss_history: Vec::new(),
        }
    }

    pub fn update(&mut self, rtt_ms: f64, loss: f64) -> u32 {
        self.rtt_history.push(rtt_ms);
        if self.rtt_history.len() > 10 {
            self.rtt_history.remove(0);
        }
        self.loss_history.push(loss);
        if self.loss_history.len() > 10 {
            self.loss_history.remove(0);
        }

        let avg_rtt = self.rtt_history.iter().sum::<f64>() / self.rtt_history.len() as f64;
        let avg_loss = self.loss_history.iter().sum::<f64>() / self.loss_history.len() as f64;

        if avg_loss > 0.05 {
            self.current_bitrate_kbps = (self.current_bitrate_kbps as f64 * 0.75) as u32;
        } else if avg_rtt > 100.0 {
            self.current_bitrate_kbps = (self.current_bitrate_kbps as f64 * 0.85) as u32;
        } else if avg_loss < 0.01 && avg_rtt < 50.0 {
            self.current_bitrate_kbps = (self.current_bitrate_kbps as f64 * 1.08) as u32;
        }

        self.current_bitrate_kbps = self.current_bitrate_kbps
            .clamp(self.min_bitrate_kbps, self.max_bitrate_kbps);
        self.current_bitrate_kbps
    }

    pub fn current_bitrate(&self) -> u32 {
        self.current_bitrate_kbps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_diagnostics_serializes_to_json() {
        let diagnostics = P2pDiagnostics::default();
        let json = serde_json::to_string(&diagnostics);
        assert!(
            json.is_ok(),
            "default diagnostics must serialize: {:?}",
            json.err()
        );
    }

    #[test]
    fn test_adaptive_bitrate_controller() {
        let mut controller = AdaptiveBitrateController::new(1000, 10000, 5000);
        assert_eq!(controller.current_bitrate(), 5000);

        // Good channel -> bitrate increases
        let rate = controller.update(10.0, 0.0);
        assert!(rate > 5000);

        // Bad channel -> bitrate drops
        let mut controller = AdaptiveBitrateController::new(1000, 10000, 5000);
        let rate = controller.update(200.0, 0.1);
        assert!(rate < 5000);
    }
}
