use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum PacketPriority {
    ReliableInput = 1,
    RealtimeInput = 2,
    Audio = 3,
    VideoControl = 4,
    VideoPayload = 5,
    Diagnostics = 6,
}

impl TryFrom<u8> for PacketPriority {
    type Error = UnknownPriority;

    fn try_from(value: u8) -> Result<Self, UnknownPriority> {
        match value {
            1 => Ok(Self::ReliableInput),
            2 => Ok(Self::RealtimeInput),
            3 => Ok(Self::Audio),
            4 => Ok(Self::VideoControl),
            5 => Ok(Self::VideoPayload),
            6 => Ok(Self::Diagnostics),
            _ => Err(UnknownPriority(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum PacketType {
    Handshake = 1,
    Authentication = 2,
    Video = 3,
    Audio = 4,
    MouseMove = 5,
    MouseButton = 6,
    Keyboard = 7,
    Gamepad = 8,
    Clipboard = 9,
    NetworkFeedback = 10,
    EncoderFeedback = 11,
    DecoderFeedback = 12,
    Nack = 13,
    KeyframeRequest = 14,
    Keepalive = 15,
    Disconnect = 16,
    Error = 17,
    Acknowledgement = 18,
}

impl PacketType {
    #[must_use]
    pub const fn priority(self) -> PacketPriority {
        match self {
            Self::MouseButton | Self::Keyboard | Self::Disconnect => PacketPriority::ReliableInput,
            Self::MouseMove | Self::Gamepad => PacketPriority::RealtimeInput,
            Self::Audio => PacketPriority::Audio,
            Self::Handshake
            | Self::Authentication
            | Self::NetworkFeedback
            | Self::Nack
            | Self::KeyframeRequest
            | Self::Keepalive
            | Self::Acknowledgement => PacketPriority::VideoControl,
            Self::Video => PacketPriority::VideoPayload,
            Self::Clipboard | Self::EncoderFeedback | Self::DecoderFeedback | Self::Error => {
                PacketPriority::Diagnostics
            }
        }
    }

    #[must_use]
    pub const fn max_payload_len(self) -> usize {
        match self {
            Self::Video => 1_048_576,
            Self::Clipboard => 262_144,
            Self::Acknowledgement => crate::ACKNOWLEDGEMENT_PAYLOAD_LEN,
            Self::Audio
            | Self::NetworkFeedback
            | Self::EncoderFeedback
            | Self::DecoderFeedback
            | Self::Nack
            | Self::KeyframeRequest
            | Self::Keepalive
            | Self::Disconnect
            | Self::Error => 65_536,
            Self::Handshake | Self::Authentication => 16_384,
            Self::MouseMove | Self::MouseButton | Self::Keyboard | Self::Gamepad => 4_096,
        }
    }

    /// Returns the exact payload width for packet classes whose wire schema is
    /// fixed-size. Other classes are bounded only by [`Self::max_payload_len`].
    #[must_use]
    pub const fn exact_payload_len(self) -> Option<usize> {
        match self {
            Self::Acknowledgement => Some(crate::ACKNOWLEDGEMENT_PAYLOAD_LEN),
            _ => None,
        }
    }
}

impl TryFrom<u8> for PacketType {
    type Error = UnknownPacketType;

    fn try_from(value: u8) -> Result<Self, UnknownPacketType> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::Authentication),
            3 => Ok(Self::Video),
            4 => Ok(Self::Audio),
            5 => Ok(Self::MouseMove),
            6 => Ok(Self::MouseButton),
            7 => Ok(Self::Keyboard),
            8 => Ok(Self::Gamepad),
            9 => Ok(Self::Clipboard),
            10 => Ok(Self::NetworkFeedback),
            11 => Ok(Self::EncoderFeedback),
            12 => Ok(Self::DecoderFeedback),
            13 => Ok(Self::Nack),
            14 => Ok(Self::KeyframeRequest),
            15 => Ok(Self::Keepalive),
            16 => Ok(Self::Disconnect),
            17 => Ok(Self::Error),
            18 => Ok(Self::Acknowledgement),
            _ => Err(UnknownPacketType(value)),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    pub struct PacketFlags: u16 {
        const KEY_FRAME = 1 << 0;
        const END_OF_FRAME = 1 << 1;
        const RETRANSMITTED = 1 << 2;
        const ACK_REQUIRED = 1 << 3;
        const DISCONTINUITY = 1 << 4;
        /// Marks the fragment whose sequence starts a video frame.
        const START_OF_FRAME = 1 << 5;
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("unknown packet type {0}")]
pub struct UnknownPacketType(pub u8);

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("unknown packet priority {0}")]
pub struct UnknownPriority(pub u8);
