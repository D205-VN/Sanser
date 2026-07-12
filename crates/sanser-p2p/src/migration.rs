//! Route migration and network change detection.

use serde::Serialize;

/// Reason a route migration was triggered.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationReason {
    /// A network interface was added or removed.
    InterfaceChanged,
    /// Wi-Fi SSID changed.
    WiFiChanged,
    /// The active route stopped responding to keepalives.
    RouteLost,
    /// Device resumed from sleep or hibernate.
    ResumedFromSleep,
    /// A significantly better route was discovered.
    BetterRouteFound,
    /// IPv4 route replaced by IPv6 or vice versa.
    ProtocolSwitch,
}

/// State of a route migration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationState {
    #[default]
    Stable,
    /// Network change detected, re-gathering candidates.
    Regathering,
    /// New candidates exchanged, checking connectivity.
    Rechecking,
    /// New route confirmed, switching over.
    Switching,
    /// Migration completed.
    Completed,
    /// Migration failed, falling back to old route if possible.
    Failed,
}

/// Reconnect schedule per plan section 26.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconnectSchedule {
    /// Keep probing the current route for this long (ms).
    pub probe_current_ms: u64,
    /// Retry old candidate pairs for this long after initial probe (ms).
    pub retry_old_ms: u64,
    /// Gather new candidates for this long after retry (ms).
    pub regather_ms: u64,
    /// Total timeout before declaring failure (ms).
    pub total_timeout_ms: u64,
}

impl Default for ReconnectSchedule {
    fn default() -> Self {
        Self {
            probe_current_ms: 1_000,
            retry_old_ms: 2_000,
            regather_ms: 5_000,
            total_timeout_ms: 8_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconnectAction {
    ProbeCurrentRoute,
    RetryOldCandidates,
    GatherNewCandidates,
    FailConnection,
}

pub struct MigrationController {
    state: MigrationState,
    schedule: ReconnectSchedule,
}

impl MigrationController {
    #[must_use]
    pub fn new(schedule: ReconnectSchedule) -> Self {
        Self {
            state: MigrationState::Stable,
            schedule,
        }
    }

    #[must_use]
    pub fn state(&self) -> MigrationState {
        self.state
    }

    pub fn transition_to(&mut self, next: MigrationState) {
        self.state = next;
    }

    #[must_use]
    pub fn check_reconnect_action(&self, elapsed_ms: u64) -> ReconnectAction {
        if elapsed_ms < self.schedule.probe_current_ms {
            ReconnectAction::ProbeCurrentRoute
        } else if elapsed_ms < self.schedule.probe_current_ms + self.schedule.retry_old_ms {
            ReconnectAction::RetryOldCandidates
        } else if elapsed_ms < self.schedule.total_timeout_ms {
            ReconnectAction::GatherNewCandidates
        } else {
            ReconnectAction::FailConnection
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_schedule_actions() {
        let schedule = ReconnectSchedule::default();
        let controller = MigrationController::new(schedule);

        assert_eq!(
            controller.check_reconnect_action(500),
            ReconnectAction::ProbeCurrentRoute
        );
        assert_eq!(
            controller.check_reconnect_action(1500),
            ReconnectAction::RetryOldCandidates
        );
        assert_eq!(
            controller.check_reconnect_action(4000),
            ReconnectAction::GatherNewCandidates
        );
        assert_eq!(
            controller.check_reconnect_action(9000),
            ReconnectAction::FailConnection
        );
    }
}
