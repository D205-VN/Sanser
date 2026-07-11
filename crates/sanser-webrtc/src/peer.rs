use crate::{DataChannelPolicy, IceCandidate};
use thiserror::Error;

const MAX_SDP_BYTES: usize = 256 * 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionDescriptionType {
    Offer,
    Answer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDescription {
    pub kind: SessionDescriptionType,
    pub sdp: String,
}

impl SessionDescription {
    /// Validates an SDP offer or answer before passing it to a native backend.
    ///
    /// # Errors
    ///
    /// Returns [`PeerError::InvalidDescription`] when the SDP is empty, exceeds
    /// 256 KiB, contains a NUL byte, or does not begin with the SDP `v=0` line.
    pub fn validate(self) -> Result<Self, PeerError> {
        if self.sdp.is_empty()
            || self.sdp.len() > MAX_SDP_BYTES
            || self.sdp.contains('\0')
            || !self.sdp.starts_with("v=0")
        {
            return Err(PeerError::InvalidDescription);
        }
        Ok(self)
    }
}

/// Minimal boundary implemented by the platform libdatachannel adapter.
/// Implementations must invoke callbacks on their own bounded executor.
pub trait PeerBackend: Send {
    /// Applies a previously validated remote offer or answer.
    ///
    /// # Errors
    ///
    /// Returns [`PeerError::InvalidDescription`] if the adapter rejects invalid
    /// SDP, [`PeerError::Closed`] after peer shutdown, or [`PeerError::Backend`]
    /// when the native WebRTC implementation rejects the operation.
    fn set_remote_description(&mut self, description: SessionDescription) -> Result<(), PeerError>;

    /// Adds a validated trickle ICE candidate to the remote peer.
    ///
    /// # Errors
    ///
    /// Returns [`PeerError::Closed`] after peer shutdown or
    /// [`PeerError::Backend`] when the native WebRTC implementation cannot add
    /// the candidate.
    fn add_remote_candidate(&mut self, candidate: IceCandidate) -> Result<(), PeerError>;

    /// Creates a data channel using the supplied reliability policy.
    ///
    /// # Errors
    ///
    /// Returns [`PeerError::InvalidChannelLabel`] for a label the adapter cannot
    /// accept, [`PeerError::Closed`] after peer shutdown, or
    /// [`PeerError::Backend`] when native channel creation fails.
    fn create_data_channel(
        &mut self,
        label: &str,
        policy: DataChannelPolicy,
    ) -> Result<(), PeerError>;
    fn close(&mut self);
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PeerError {
    #[error("session description is invalid")]
    InvalidDescription,
    #[error("data-channel label is invalid")]
    InvalidChannelLabel,
    #[error("native WebRTC backend rejected the operation")]
    Backend,
    #[error("native WebRTC peer is closed")]
    Closed,
}
