use crate::{PacketFlags, PacketPriority, PacketType};
use hmac::{Hmac, Mac};
use sanser_core::{PROTOCOL_VERSION, SessionId, StreamId};
use sha2::Sha256;
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

type HmacSha256 = Hmac<Sha256>;

pub const MAGIC: [u8; 4] = *b"SNV2";
pub const FIXED_HEADER_LEN: usize = 80;
pub const AUTH_TAG_LEN: usize = 16;
pub const GLOBAL_MAX_PAYLOAD_LEN: usize = 1_048_576;
const AUTHENTICATED_HEADER_LEN: usize = FIXED_HEADER_LEN - AUTH_TAG_LEN;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct AuthKey([u8; 32]);

impl AuthKey {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for AuthKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuthKey([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PacketHeader {
    pub packet_type: PacketType,
    pub session_id: SessionId,
    pub stream_id: StreamId,
    pub sequence: u64,
    pub frame_id: u64,
    pub timestamp_us: u64,
    pub flags: PacketFlags,
    pub key_id: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Packet {
    pub header: PacketHeader,
    pub payload: Vec<u8>,
}

impl Packet {
    /// Serializes and authenticates this packet using its direction-specific
    /// session key.
    ///
    /// # Errors
    ///
    /// Returns an error when the session identifier is empty, the payload
    /// exceeds its packet-type limit, a fixed wire field cannot represent its
    /// value, or HMAC initialization fails.
    pub fn encode(&self, key: &AuthKey) -> Result<Vec<u8>, ProtocolError> {
        if self.header.session_id.as_uuid().is_nil() {
            return Err(ProtocolError::EmptySessionId);
        }
        validate_payload(self.header.packet_type, self.payload.len())?;
        let payload_len = u32::try_from(self.payload.len())
            .map_err(|_| ProtocolError::PayloadTooLarge(self.payload.len()))?;
        let mut encoded = vec![0_u8; FIXED_HEADER_LEN + self.payload.len()];
        encoded[0..4].copy_from_slice(&MAGIC);
        encoded[4] = PROTOCOL_VERSION;
        encoded[5] = u8::try_from(FIXED_HEADER_LEN)
            .map_err(|_| ProtocolError::FixedHeaderLengthOutOfRange(FIXED_HEADER_LEN))?;
        encoded[6] = self.header.packet_type as u8;
        encoded[7] = self.header.packet_type.priority() as u8;
        encoded[8..10].copy_from_slice(&self.header.flags.bits().to_be_bytes());
        encoded[10..12].copy_from_slice(&0_u16.to_be_bytes());
        encoded[12..28].copy_from_slice(self.header.session_id.as_bytes());
        encoded[28..32].copy_from_slice(&self.header.stream_id.get().to_be_bytes());
        encoded[32..40].copy_from_slice(&self.header.sequence.to_be_bytes());
        encoded[40..48].copy_from_slice(&self.header.frame_id.to_be_bytes());
        encoded[48..56].copy_from_slice(&self.header.timestamp_us.to_be_bytes());
        encoded[56..60].copy_from_slice(&payload_len.to_be_bytes());
        encoded[60..64].copy_from_slice(&self.header.key_id.to_be_bytes());
        encoded[FIXED_HEADER_LEN..].copy_from_slice(&self.payload);

        let tag = authentication_tag(
            key,
            &encoded[..AUTHENTICATED_HEADER_LEN],
            &encoded[FIXED_HEADER_LEN..],
        )?;
        encoded[AUTHENTICATED_HEADER_LEN..FIXED_HEADER_LEN].copy_from_slice(&tag);
        Ok(encoded)
    }

    /// Parses, validates and authenticates one complete SNV2 packet.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed or non-canonical headers, unsupported
    /// fields, invalid lengths, an empty or unexpected session identifier,
    /// an oversized payload, or failed authentication.
    pub fn decode(
        encoded: &[u8],
        key: &AuthKey,
        expected_session: SessionId,
    ) -> Result<Self, ProtocolError> {
        if encoded.len() < FIXED_HEADER_LEN {
            return Err(ProtocolError::Truncated);
        }
        if encoded[0..4] != MAGIC {
            return Err(ProtocolError::InvalidMagic);
        }
        if encoded[4] != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(encoded[4]));
        }
        if usize::from(encoded[5]) != FIXED_HEADER_LEN {
            return Err(ProtocolError::InvalidHeaderLength(encoded[5]));
        }
        if encoded[10..12] != [0, 0] {
            return Err(ProtocolError::ReservedBitsSet);
        }

        let packet_type = PacketType::try_from(encoded[6])
            .map_err(|_| ProtocolError::UnknownPacketType(encoded[6]))?;
        let priority = PacketPriority::try_from(encoded[7])
            .map_err(|_| ProtocolError::InvalidPriority(encoded[7]))?;
        if priority != packet_type.priority() {
            return Err(ProtocolError::PriorityMismatch);
        }
        let flags_raw = read_u16(encoded, 8)?;
        let flags =
            PacketFlags::from_bits(flags_raw).ok_or(ProtocolError::UnknownFlags(flags_raw))?;
        let session_id =
            SessionId::try_from(&encoded[12..28]).map_err(|_| ProtocolError::InvalidSessionId)?;
        if session_id.as_uuid().is_nil() {
            return Err(ProtocolError::EmptySessionId);
        }
        if session_id != expected_session {
            return Err(ProtocolError::SessionMismatch);
        }
        let payload_len = usize::try_from(read_u32(encoded, 56)?)
            .map_err(|_| ProtocolError::PayloadTooLarge(usize::MAX))?;
        validate_payload(packet_type, payload_len)?;
        let expected_len = FIXED_HEADER_LEN
            .checked_add(payload_len)
            .ok_or(ProtocolError::PayloadTooLarge(payload_len))?;
        if encoded.len() != expected_len {
            return Err(ProtocolError::LengthMismatch {
                declared: expected_len,
                actual: encoded.len(),
            });
        }

        let mut verifier = HmacSha256::new_from_slice(key.as_bytes())
            .map_err(|_| ProtocolError::AuthenticationSetup)?;
        verifier.update(&encoded[..AUTHENTICATED_HEADER_LEN]);
        verifier.update(&encoded[FIXED_HEADER_LEN..]);
        verifier
            .verify_truncated_left(&encoded[AUTHENTICATED_HEADER_LEN..FIXED_HEADER_LEN])
            .map_err(|_| ProtocolError::AuthenticationFailed)?;

        Ok(Self {
            header: PacketHeader {
                packet_type,
                session_id,
                stream_id: StreamId::new(read_u32(encoded, 28)?),
                sequence: read_u64(encoded, 32)?,
                frame_id: read_u64(encoded, 40)?,
                timestamp_us: read_u64(encoded, 48)?,
                flags,
                key_id: read_u32(encoded, 60)?,
            },
            payload: encoded[FIXED_HEADER_LEN..].to_vec(),
        })
    }
}

fn authentication_tag(
    key: &AuthKey,
    authenticated_header: &[u8],
    payload: &[u8],
) -> Result<[u8; AUTH_TAG_LEN], ProtocolError> {
    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .map_err(|_| ProtocolError::AuthenticationSetup)?;
    mac.update(authenticated_header);
    mac.update(payload);
    let bytes = mac.finalize().into_bytes();
    let mut tag = [0_u8; AUTH_TAG_LEN];
    tag.copy_from_slice(&bytes[..AUTH_TAG_LEN]);
    Ok(tag)
}

fn validate_payload(packet_type: PacketType, length: usize) -> Result<(), ProtocolError> {
    if let Some(expected) = packet_type.exact_payload_len() {
        if length != expected {
            return Err(ProtocolError::InvalidPayloadLength {
                packet_type,
                expected,
                actual: length,
            });
        }
    }
    let limit = packet_type.max_payload_len().min(GLOBAL_MAX_PAYLOAD_LEN);
    if length > limit {
        return Err(ProtocolError::PayloadTooLarge(length));
    }
    Ok(())
}

fn read_u16(input: &[u8], offset: usize) -> Result<u16, ProtocolError> {
    let bytes: [u8; 2] = input
        .get(offset..offset + 2)
        .ok_or(ProtocolError::Truncated)?
        .try_into()
        .map_err(|_| ProtocolError::Truncated)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(input: &[u8], offset: usize) -> Result<u32, ProtocolError> {
    let bytes: [u8; 4] = input
        .get(offset..offset + 4)
        .ok_or(ProtocolError::Truncated)?
        .try_into()
        .map_err(|_| ProtocolError::Truncated)?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64(input: &[u8], offset: usize) -> Result<u64, ProtocolError> {
    let bytes: [u8; 8] = input
        .get(offset..offset + 8)
        .ok_or(ProtocolError::Truncated)?
        .try_into()
        .map_err(|_| ProtocolError::Truncated)?;
    Ok(u64::from_be_bytes(bytes))
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProtocolError {
    #[error("packet is shorter than the fixed SNV2 header")]
    Truncated,
    #[error("invalid SNV2 magic")]
    InvalidMagic,
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u8),
    #[error("invalid fixed header length {0}")]
    InvalidHeaderLength(u8),
    #[error("fixed header length {0} cannot be represented on the wire")]
    FixedHeaderLengthOutOfRange(usize),
    #[error("reserved header bits are set")]
    ReservedBitsSet,
    #[error("unknown packet type {0}")]
    UnknownPacketType(u8),
    #[error("invalid packet priority {0}")]
    InvalidPriority(u8),
    #[error("packet type and priority do not match")]
    PriorityMismatch,
    #[error("unknown packet flags 0x{0:04x}")]
    UnknownFlags(u16),
    #[error("invalid session identifier")]
    InvalidSessionId,
    #[error("session identifier must not be empty")]
    EmptySessionId,
    #[error("packet belongs to another session")]
    SessionMismatch,
    #[error("payload length {0} exceeds the packet limit")]
    PayloadTooLarge(usize),
    #[error("{packet_type:?} payload must be exactly {expected} bytes, got {actual}")]
    InvalidPayloadLength {
        packet_type: PacketType,
        expected: usize,
        actual: usize,
    },
    #[error("packet length mismatch: declared {declared}, actual {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("authentication setup failed")]
    AuthenticationSetup,
    #[error("packet authentication failed")]
    AuthenticationFailed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Packet, AuthKey, SessionId) {
        let session = SessionId::new();
        (
            Packet {
                header: PacketHeader {
                    packet_type: PacketType::Video,
                    session_id: session,
                    stream_id: StreamId::VIDEO_PRIMARY,
                    sequence: 42,
                    frame_id: 7,
                    timestamp_us: 123_456,
                    flags: PacketFlags::KEY_FRAME
                        | PacketFlags::START_OF_FRAME
                        | PacketFlags::END_OF_FRAME,
                    key_id: 3,
                },
                payload: vec![1, 2, 3, 4],
            },
            AuthKey::new([9; 32]),
            session,
        )
    }

    #[test]
    fn packet_round_trip_preserves_header_and_payload() {
        let (packet, key, session) = fixture();
        let encoded = packet
            .encode(&key)
            .unwrap_or_else(|error| panic!("encode failed: {error}"));
        assert_eq!(encoded.len(), FIXED_HEADER_LEN + 4);
        assert_eq!(encoded[0..4], MAGIC);
        assert_eq!(encoded[56..60], 4_u32.to_be_bytes());
        let decoded = Packet::decode(&encoded, &key, session)
            .unwrap_or_else(|error| panic!("decode failed: {error}"));
        assert_eq!(decoded, packet);
    }

    #[test]
    fn tampering_is_rejected() {
        let (packet, key, session) = fixture();
        let mut encoded = packet
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        encoded[FIXED_HEADER_LEN] ^= 0xff;
        assert_eq!(
            Packet::decode(&encoded, &key, session),
            Err(ProtocolError::AuthenticationFailed)
        );
    }

    #[test]
    fn rejects_wrong_version_session_and_oversized_input() {
        let (packet, key, session) = fixture();
        let mut version = packet
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        version[4] = 1;
        assert_eq!(
            Packet::decode(&version, &key, session),
            Err(ProtocolError::UnsupportedVersion(1))
        );

        let encoded = packet
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            Packet::decode(&encoded, &key, SessionId::new()),
            Err(ProtocolError::SessionMismatch)
        );

        let oversized = Packet {
            header: PacketHeader {
                packet_type: PacketType::MouseMove,
                ..packet.header
            },
            payload: vec![0; 4_097],
        };
        assert_eq!(
            oversized.encode(&key),
            Err(ProtocolError::PayloadTooLarge(4_097))
        );
    }

    #[test]
    fn rejects_noncanonical_priority_before_authentication() {
        let (packet, key, session) = fixture();
        let mut encoded = packet
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        encoded[7] = PacketPriority::ReliableInput as u8;
        assert_eq!(
            Packet::decode(&encoded, &key, session),
            Err(ProtocolError::PriorityMismatch)
        );
    }

    #[test]
    fn acknowledgement_uses_control_priority_and_exact_payload_width() {
        let (packet, key, session) = fixture();
        assert_eq!(
            PacketType::Acknowledgement.priority(),
            PacketPriority::VideoControl
        );
        let acknowledgement = Packet {
            header: PacketHeader {
                packet_type: PacketType::Acknowledgement,
                flags: PacketFlags::empty(),
                ..packet.header.clone()
            },
            payload: crate::Acknowledgement {
                cumulative_sequence: 40,
                selective_mask: 0b101,
            }
            .encode()
            .to_vec(),
        };
        let encoded = acknowledgement
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(Packet::decode(&encoded, &key, session), Ok(acknowledgement));

        let mut malformed = encoded;
        malformed[59] = u8::try_from(crate::ACKNOWLEDGEMENT_PAYLOAD_LEN - 1)
            .unwrap_or_else(|_| panic!("acknowledgement payload width exceeds u8"));
        malformed.pop();
        assert_eq!(
            Packet::decode(&malformed, &key, session),
            Err(ProtocolError::InvalidPayloadLength {
                packet_type: PacketType::Acknowledgement,
                expected: crate::ACKNOWLEDGEMENT_PAYLOAD_LEN,
                actual: crate::ACKNOWLEDGEMENT_PAYLOAD_LEN - 1,
            })
        );

        assert_eq!(
            Packet {
                header: PacketHeader {
                    packet_type: PacketType::Acknowledgement,
                    ..packet.header.clone()
                },
                payload: vec![0; crate::ACKNOWLEDGEMENT_PAYLOAD_LEN - 1],
            }
            .encode(&key),
            Err(ProtocolError::InvalidPayloadLength {
                packet_type: PacketType::Acknowledgement,
                expected: crate::ACKNOWLEDGEMENT_PAYLOAD_LEN,
                actual: crate::ACKNOWLEDGEMENT_PAYLOAD_LEN - 1,
            })
        );
    }

    #[test]
    fn rejects_unknown_flags_but_accepts_start_of_frame() {
        let (packet, key, session) = fixture();
        let encoded = packet
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(Packet::decode(&encoded, &key, session).is_ok());

        let mut unknown = encoded;
        unknown[9] |= 1 << 6;
        assert_eq!(
            Packet::decode(&unknown, &key, session),
            Err(ProtocolError::UnknownFlags(0x0063))
        );
    }

    #[test]
    fn matches_cross_language_native_hmac_vector() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Vector {
            protocol_version: u8,
            byte_order: String,
            header_prefix_hex: String,
            payload_hex: String,
            direction_key_hex: String,
            auth_tag_hex: String,
        }

        let vector: Vector =
            serde_json::from_str(include_str!("../../../native/protocol/test_vectors.json"))
                .unwrap_or_else(|error| panic!("vector JSON failed: {error}"));
        assert_eq!(vector.protocol_version, PROTOCOL_VERSION);
        assert_eq!(vector.byte_order, "big-endian");
        let header_prefix = hex::decode(vector.header_prefix_hex)
            .unwrap_or_else(|error| panic!("header hex failed: {error}"));
        let payload = hex::decode(vector.payload_hex)
            .unwrap_or_else(|error| panic!("payload hex failed: {error}"));
        let key: [u8; 32] = hex::decode(vector.direction_key_hex)
            .unwrap_or_else(|error| panic!("key hex failed: {error}"))
            .try_into()
            .unwrap_or_else(|_| panic!("key vector length is not 32"));
        let expected_tag = hex::decode(vector.auth_tag_hex)
            .unwrap_or_else(|error| panic!("tag hex failed: {error}"));
        assert_eq!(header_prefix.len(), AUTHENTICATED_HEADER_LEN);
        let tag = authentication_tag(&AuthKey::new(key), &header_prefix, &payload)
            .unwrap_or_else(|error| panic!("HMAC failed: {error}"));
        assert_eq!(tag.as_slice(), expected_tag);

        let mut encoded = header_prefix;
        encoded.extend_from_slice(&tag);
        encoded.extend_from_slice(&payload);
        let session = SessionId::try_from(&encoded[12..28])
            .unwrap_or_else(|error| panic!("session parse failed: {error}"));
        let packet = Packet::decode(&encoded, &AuthKey::new(key), session)
            .unwrap_or_else(|error| panic!("vector decode failed: {error}"));
        assert_eq!(packet.header.packet_type, PacketType::MouseButton);
        assert_eq!(packet.header.sequence, 99);
        assert_eq!(packet.payload, [1, 2, 3]);
    }
}
