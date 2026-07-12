//! SNV2 packet framing.
//!
//! Multibyte integers use network byte order. Authentication uses a dedicated
//! 32-byte direction key; callers must never reuse the same key in both
//! directions of a session.

mod control;
mod packet;
mod reassembly;
mod replay;
mod wire;

pub use control::{
    ACKNOWLEDGEMENT_PAYLOAD_LEN, Acknowledgement, ControlPayloadError, KeyframeRequest,
    MAX_NACK_SEQUENCES, Nack,
};
pub use packet::{PacketFlags, PacketPriority, PacketType};
pub use reassembly::{
    CompletedFrame, FrameAssembler, FrameAssemblerConfig, FrameAssemblerError, FramePush,
};
pub use replay::{REPLAY_WINDOW_SIZE, ReplayDecision, ReplayWindow};
pub use wire::{
    AUTH_TAG_LEN, AuthKey, FIXED_HEADER_LEN, GLOBAL_MAX_PAYLOAD_LEN, MAGIC, Packet, PacketHeader,
    ProtocolError,
};
