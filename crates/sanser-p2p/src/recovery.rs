//! Bounded reconnect policy. A reconnect re-gathers candidates and re-authorizes
//! the accepted session; it is not seamless in-flight socket migration.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryCause {
    PeerTimeout,
    RelayLost,
    ControlUnresponsive,
    SustainedWireDelay,
    ApplicationDelay,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEvidence {
    pub cause: RecoveryCause,
    pub automatic_attempts: u8,
    pub stable_ms: u64,
    pub evidence_age_ms: u64,
    #[serde(default)]
    pub control_silence_ms: f64,
    #[serde(default)]
    pub probe_samples: u32,
    #[serde(default)]
    pub probe_median_ms: f64,
    #[serde(default)]
    pub wire_rtt_ms: f64,
    #[serde(default)]
    pub elevated_samples: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryAction {
    Observe,
    Reconnect,
    Manual,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPlan {
    pub action: RecoveryAction,
    pub delay_ms: u64,
    pub reason: &'static str,
}

#[must_use]
pub fn plan_route_recovery(evidence: &RecoveryEvidence) -> RecoveryPlan {
    let observe = |reason| RecoveryPlan {
        action: RecoveryAction::Observe,
        delay_ms: 0,
        reason,
    };
    if evidence.evidence_age_ms > 3_000 {
        return observe("stale-evidence");
    }
    let justified = match evidence.cause {
        RecoveryCause::PeerTimeout | RecoveryCause::RelayLost => true,
        RecoveryCause::ApplicationDelay => false,
        RecoveryCause::ControlUnresponsive => {
            evidence.stable_ms >= 15_000
                && evidence.control_silence_ms.is_finite()
                && evidence.control_silence_ms >= 15_000.0
        }
        RecoveryCause::SustainedWireDelay => {
            evidence.stable_ms >= 15_000
                && evidence.probe_samples >= 3
                && evidence.elevated_samples >= 5
                && evidence.probe_median_ms.is_finite()
                && (0.0..80.0).contains(&evidence.probe_median_ms)
                && evidence.wire_rtt_ms.is_finite()
                && evidence.wire_rtt_ms
                    > 80.0_f64
                        .max(evidence.probe_median_ms * 2.0)
                        .max(evidence.probe_median_ms + 40.0)
        }
    };
    if !justified {
        return observe("insufficient-route-evidence");
    }
    if evidence.automatic_attempts >= 2 {
        return RecoveryPlan {
            action: RecoveryAction::Manual,
            delay_ms: 0,
            reason: "automatic-recovery-limit",
        };
    }
    RecoveryPlan {
        action: RecoveryAction::Reconnect,
        delay_ms: if evidence.automatic_attempts == 0 {
            1_500
        } else {
            5_000
        },
        reason: match evidence.cause {
            RecoveryCause::PeerTimeout => "peer-timeout",
            RecoveryCause::RelayLost => "relay-lost",
            RecoveryCause::ControlUnresponsive => "control-unresponsive",
            _ => "wire-delay-increased",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence(cause: RecoveryCause) -> RecoveryEvidence {
        RecoveryEvidence {
            cause,
            automatic_attempts: 0,
            stable_ms: 20_000,
            evidence_age_ms: 0,
            control_silence_ms: 15_000.0,
            probe_samples: 5,
            probe_median_ms: 14.0,
            wire_rtt_ms: 200.0,
            elevated_samples: 5,
        }
    }
    #[test]
    fn slow_nomination_and_application_delay_do_not_tear_down_a_working_route() {
        assert_eq!(
            plan_route_recovery(&evidence(RecoveryCause::ApplicationDelay)).action,
            RecoveryAction::Observe
        );
        let mut e = evidence(RecoveryCause::SustainedWireDelay);
        e.probe_median_ms = 183.0;
        assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Observe);
        e.probe_median_ms = 14.0;
        e.elevated_samples = 4;
        assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Observe);
        e.elevated_samples = 5;
        e.stable_ms = 5_000;
        assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Observe);
        e.stable_ms = 20_000;
        assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Reconnect);
        e.wire_rtt_ms = f64::NAN;
        assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Observe);
    }
    #[test]
    fn recovery_has_a_finite_budget_and_never_acts_on_stale_evidence() {
        for cause in [
            RecoveryCause::PeerTimeout,
            RecoveryCause::RelayLost,
            RecoveryCause::ControlUnresponsive,
        ] {
            let mut e = evidence(cause);
            assert_eq!(plan_route_recovery(&e).delay_ms, 1_500);
            e.automatic_attempts = 1;
            assert_eq!(plan_route_recovery(&e).delay_ms, 5_000);
            e.automatic_attempts = 2;
            assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Manual);
            e.automatic_attempts = 0;
            e.evidence_age_ms = 3_001;
            assert_eq!(plan_route_recovery(&e).action, RecoveryAction::Observe);
        }
    }
}
