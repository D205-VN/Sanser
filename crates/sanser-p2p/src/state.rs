use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum P2pStage {
    #[default]
    Idle,
    SessionRequested,
    SessionAccepted,
    Gathering,
    ExchangingCandidates,
    PunchPreparing,
    Checking,
    Connected,
    Authenticating,
    Streaming,
    Reconnecting,
    Failed,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum P2pFailure {
    NoLocalCandidate,
    StunUnavailable,
    PortMappingUnavailable,
    PeerOffline,
    CandidateExchangeTimeout,
    PunchTimeout,
    AuthenticationFailed,
    FirewallBlocked,
    NetworkChanged,
    SessionExpired,
    NoDirectRoute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum P2pEvent {
    RequestSession,
    AcceptSession,
    StartGathering { generation: u32 },
    GatheringCompleted,
    CandidatesExchanged,
    PunchReady,
    PathConnected,
    BeginAuthentication,
    AuthenticationSucceeded,
    PathLost,
    RetryConnectivityChecks,
    Fail(P2pFailure),
    Close,
}

/// Strict event-driven lifecycle for one P2P session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct P2pStateMachine {
    stage: P2pStage,
    generation: u32,
    last_failure: Option<P2pFailure>,
    failed_from: Option<P2pStage>,
}

impl P2pStateMachine {
    #[must_use]
    pub const fn stage(&self) -> P2pStage {
        self.stage
    }

    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    #[must_use]
    pub const fn last_failure(&self) -> Option<P2pFailure> {
        self.last_failure
    }

    /// Applies one lifecycle event without permitting state skips.
    ///
    /// # Errors
    ///
    /// Returns [`StateMachineError::InvalidTransition`] for an event that is
    /// illegal in the current stage, or [`StateMachineError::StaleGeneration`]
    /// when gathering does not advance the candidate generation.
    pub fn apply(&mut self, event: P2pEvent) -> Result<P2pStage, StateMachineError> {
        if let P2pEvent::StartGathering { generation } = event {
            if generation == 0 {
                return Err(StateMachineError::InvalidGeneration(generation));
            }
            if generation <= self.generation {
                return Err(StateMachineError::StaleGeneration {
                    current: self.generation,
                    attempted: generation,
                });
            }
        }

        let next = match (self.stage, event) {
            (P2pStage::Idle, P2pEvent::RequestSession) => P2pStage::SessionRequested,
            (P2pStage::SessionRequested, P2pEvent::AcceptSession) => P2pStage::SessionAccepted,
            (
                P2pStage::SessionAccepted | P2pStage::Reconnecting,
                P2pEvent::StartGathering { generation },
            ) => {
                self.generation = generation;
                self.last_failure = None;
                self.failed_from = None;
                P2pStage::Gathering
            }
            (P2pStage::Failed, P2pEvent::StartGathering { generation })
                if self.failed_from.is_some_and(can_restart_after_failure) =>
            {
                self.generation = generation;
                self.last_failure = None;
                self.failed_from = None;
                P2pStage::Gathering
            }
            (P2pStage::Gathering, P2pEvent::GatheringCompleted) => P2pStage::ExchangingCandidates,
            (P2pStage::ExchangingCandidates, P2pEvent::CandidatesExchanged) => {
                P2pStage::PunchPreparing
            }
            (P2pStage::PunchPreparing, P2pEvent::PunchReady)
            | (P2pStage::Reconnecting, P2pEvent::RetryConnectivityChecks) => P2pStage::Checking,
            (P2pStage::Checking, P2pEvent::PathConnected) => P2pStage::Connected,
            (P2pStage::Connected, P2pEvent::BeginAuthentication) => P2pStage::Authenticating,
            (P2pStage::Authenticating, P2pEvent::AuthenticationSucceeded) => P2pStage::Streaming,
            (
                P2pStage::Connected | P2pStage::Authenticating | P2pStage::Streaming,
                P2pEvent::PathLost,
            ) => P2pStage::Reconnecting,
            (stage, P2pEvent::Fail(failure)) if !matches!(stage, P2pStage::Closed) => {
                if stage != P2pStage::Failed {
                    self.failed_from = Some(stage);
                }
                self.last_failure = Some(failure);
                P2pStage::Failed
            }
            (stage, P2pEvent::Close) if !matches!(stage, P2pStage::Closed) => P2pStage::Closed,
            (P2pStage::Closed, P2pEvent::Close) => P2pStage::Closed,
            (from, attempted) => {
                return Err(StateMachineError::InvalidTransition {
                    from,
                    event: attempted,
                });
            }
        };
        self.stage = next;
        Ok(next)
    }
}

const fn can_restart_after_failure(stage: P2pStage) -> bool {
    matches!(
        stage,
        P2pStage::SessionAccepted
            | P2pStage::Gathering
            | P2pStage::ExchangingCandidates
            | P2pStage::PunchPreparing
            | P2pStage::Checking
            | P2pStage::Connected
            | P2pStage::Authenticating
            | P2pStage::Streaming
            | P2pStage::Reconnecting
    )
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum StateMachineError {
    #[error("invalid P2P transition from {from:?} for event {event:?}")]
    InvalidTransition { from: P2pStage, event: P2pEvent },
    #[error("candidate generation must be non-zero")]
    InvalidGeneration(u32),
    #[error("candidate generation {attempted} must be newer than {current}")]
    StaleGeneration { current: u32, attempted: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(machine: &mut P2pStateMachine, event: P2pEvent) {
        machine
            .apply(event)
            .unwrap_or_else(|error| panic!("state transition failed: {error}"));
    }

    #[test]
    fn happy_path_cannot_skip_security_or_connectivity_stages() {
        let mut machine = P2pStateMachine::default();
        apply(&mut machine, P2pEvent::RequestSession);
        apply(&mut machine, P2pEvent::AcceptSession);
        apply(&mut machine, P2pEvent::StartGathering { generation: 1 });
        apply(&mut machine, P2pEvent::GatheringCompleted);
        apply(&mut machine, P2pEvent::CandidatesExchanged);
        apply(&mut machine, P2pEvent::PunchReady);
        apply(&mut machine, P2pEvent::PathConnected);
        assert_eq!(
            machine.apply(P2pEvent::AuthenticationSucceeded),
            Err(StateMachineError::InvalidTransition {
                from: P2pStage::Connected,
                event: P2pEvent::AuthenticationSucceeded,
            })
        );
        apply(&mut machine, P2pEvent::BeginAuthentication);
        apply(&mut machine, P2pEvent::AuthenticationSucceeded);
        assert_eq!(machine.stage(), P2pStage::Streaming);
        assert_eq!(machine.generation(), 1);
    }

    #[test]
    fn reconnect_requires_a_new_generation_or_explicit_pair_recheck() {
        let mut machine = P2pStateMachine {
            stage: P2pStage::Streaming,
            generation: 3,
            last_failure: None,
            failed_from: None,
        };
        apply(&mut machine, P2pEvent::PathLost);
        assert_eq!(
            machine.apply(P2pEvent::StartGathering { generation: 3 }),
            Err(StateMachineError::StaleGeneration {
                current: 3,
                attempted: 3,
            })
        );
        apply(&mut machine, P2pEvent::StartGathering { generation: 4 });
        assert_eq!(machine.stage(), P2pStage::Gathering);
        assert_eq!(machine.generation(), 4);
    }

    #[test]
    fn failure_reason_is_retained_and_closed_is_terminal() {
        let mut machine = P2pStateMachine::default();
        apply(&mut machine, P2pEvent::Fail(P2pFailure::FirewallBlocked));
        assert_eq!(machine.last_failure(), Some(P2pFailure::FirewallBlocked));
        apply(&mut machine, P2pEvent::Close);
        assert_eq!(machine.stage(), P2pStage::Closed);
        assert_eq!(machine.apply(P2pEvent::Close), Ok(P2pStage::Closed));
        assert!(machine.apply(P2pEvent::RequestSession).is_err());
    }

    #[test]
    fn failure_before_acceptance_cannot_bypass_session_authorization() {
        let mut machine = P2pStateMachine::default();
        apply(&mut machine, P2pEvent::RequestSession);
        apply(&mut machine, P2pEvent::Fail(P2pFailure::PeerOffline));
        assert_eq!(
            machine.apply(P2pEvent::StartGathering { generation: 1 }),
            Err(StateMachineError::InvalidTransition {
                from: P2pStage::Failed,
                event: P2pEvent::StartGathering { generation: 1 },
            })
        );
    }

    #[test]
    fn candidate_generation_is_nonzero_without_an_artificial_restart_cap() {
        let mut machine = P2pStateMachine {
            stage: P2pStage::SessionAccepted,
            ..P2pStateMachine::default()
        };
        assert_eq!(
            machine.apply(P2pEvent::StartGathering { generation: 0 }),
            Err(StateMachineError::InvalidGeneration(0))
        );
        apply(
            &mut machine,
            P2pEvent::StartGathering {
                generation: u32::MAX,
            },
        );
        assert_eq!(machine.generation(), u32::MAX);
    }
}
