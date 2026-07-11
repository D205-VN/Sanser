use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SignalingState {
    #[default]
    Stable,
    HaveLocalOffer,
    HaveRemoteOffer,
    Closed,
}

impl SignalingState {
    /// Moves the SDP signaling state through a legal offer/answer transition.
    ///
    /// # Errors
    ///
    /// Returns [`StateTransitionError`] with the attempted `from` and `to`
    /// states when the transition is not allowed by the signaling state model.
    pub fn transition(self, next: Self) -> Result<Self, StateTransitionError<Self>> {
        let allowed = matches!(
            (self, next),
            (
                Self::Stable,
                Self::HaveLocalOffer | Self::HaveRemoteOffer | Self::Closed
            ) | (
                Self::HaveLocalOffer | Self::HaveRemoteOffer,
                Self::Stable | Self::Closed
            )
        ) || self == next;
        allowed.then_some(next).ok_or(StateTransitionError {
            from: self,
            to: next,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConnectionState {
    #[default]
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

impl ConnectionState {
    /// Moves the peer connection through a legal lifecycle transition.
    ///
    /// # Errors
    ///
    /// Returns [`StateTransitionError`] with the attempted `from` and `to`
    /// states when the connection lifecycle does not permit the transition.
    pub fn transition(self, next: Self) -> Result<Self, StateTransitionError<Self>> {
        let allowed = match self {
            Self::New => matches!(next, Self::Connecting | Self::Closed),
            Self::Connecting => matches!(next, Self::Connected | Self::Failed | Self::Closed),
            Self::Connected => matches!(next, Self::Disconnected | Self::Failed | Self::Closed),
            Self::Disconnected => matches!(next, Self::Connecting | Self::Failed | Self::Closed),
            Self::Failed => matches!(next, Self::Closed),
            Self::Closed => false,
        } || self == next;
        allowed.then_some(next).ok_or(StateTransitionError {
            from: self,
            to: next,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("invalid state transition from {from:?} to {to:?}")]
pub struct StateTransitionError<T> {
    pub from: T,
    pub to: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_state_cannot_reopen() {
        assert!(
            ConnectionState::New
                .transition(ConnectionState::Connecting)
                .is_ok()
        );
        assert!(
            ConnectionState::Closed
                .transition(ConnectionState::Connected)
                .is_err()
        );
    }
}
