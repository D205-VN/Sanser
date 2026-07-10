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
    fn set_remote_description(&mut self, description: SessionDescription) -> Result<(), PeerError>;
    fn add_remote_candidate(&mut self, candidate: IceCandidate) -> Result<(), PeerError>;
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
