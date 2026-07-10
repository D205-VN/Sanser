//! Native WebRTC policy and signaling primitives.
//!
//! Platform bindings implement [`PeerBackend`]; this crate validates all
//! untrusted signaling values before they cross the libdatachannel boundary.

mod candidate;
mod channel;
mod ice;
mod peer;
mod state;

pub use candidate::{
    CandidateBuffer, CandidateBufferConfig, CandidateError, CandidatePush, CandidateType,
    IceCandidate,
};
pub use channel::{ChannelKind, DataChannelPolicy};
pub use ice::{ConnectionPolicy, IceCredential, IceServer, IceServerError, IceTransportPolicy};
pub use peer::{PeerBackend, PeerError, SessionDescription, SessionDescriptionType};
pub use state::{ConnectionState, SignalingState, StateTransitionError};
