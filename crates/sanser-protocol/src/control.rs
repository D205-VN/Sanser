use thiserror::Error;

pub const MAX_NACK_SEQUENCES: usize = 256;
/// Fixed width of an [`Acknowledgement`] payload on the wire.
pub const ACKNOWLEDGEMENT_PAYLOAD_LEN: usize = 16;
const NACK_PREFIX_LEN: usize = 12;

/// A cumulative acknowledgement plus a 64-packet selective acknowledgement
/// window for one stream.
///
/// `cumulative_sequence` acknowledges that sequence and every earlier sequence
/// in the same `(session, stream, direction, key generation)` tuple. Bit `i`
/// of `selective_mask`, where bit zero is the least-significant bit,
/// acknowledges `cumulative_sequence + 1 + i`. Sequence numbers do not wrap
/// within a key generation. A receiver emits no acknowledgement until it has
/// a cumulative base sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Acknowledgement {
    pub cumulative_sequence: u64,
    pub selective_mask: u64,
}

impl Acknowledgement {
    /// Encodes this ACK/SACK payload in network byte order.
    #[must_use]
    pub fn encode(self) -> [u8; ACKNOWLEDGEMENT_PAYLOAD_LEN] {
        let mut encoded = [0_u8; ACKNOWLEDGEMENT_PAYLOAD_LEN];
        encoded[..8].copy_from_slice(&self.cumulative_sequence.to_be_bytes());
        encoded[8..].copy_from_slice(&self.selective_mask.to_be_bytes());
        encoded
    }

    /// Decodes an ACK/SACK payload from its fixed-width wire representation.
    ///
    /// # Errors
    ///
    /// Returns [`ControlPayloadError::LengthMismatch`] unless `encoded`
    /// contains exactly [`ACKNOWLEDGEMENT_PAYLOAD_LEN`] bytes.
    pub fn decode(encoded: &[u8]) -> Result<Self, ControlPayloadError> {
        let encoded: &[u8; ACKNOWLEDGEMENT_PAYLOAD_LEN] = encoded
            .try_into()
            .map_err(|_| ControlPayloadError::LengthMismatch)?;
        Ok(Self {
            cumulative_sequence: u64::from_be_bytes(
                encoded[..8]
                    .try_into()
                    .map_err(|_| ControlPayloadError::LengthMismatch)?,
            ),
            selective_mask: u64::from_be_bytes(
                encoded[8..]
                    .try_into()
                    .map_err(|_| ControlPayloadError::LengthMismatch)?,
            ),
        })
    }

    /// Reports whether this cumulative/selective window acknowledges
    /// `sequence`. Callers are responsible for supplying a sequence from the
    /// same stream and key generation as this payload.
    #[must_use]
    pub const fn acknowledges(self, sequence: u64) -> bool {
        if sequence <= self.cumulative_sequence {
            return true;
        }
        let offset = sequence - self.cumulative_sequence - 1;
        offset < 64 && self.selective_mask & (1_u64 << offset) != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Nack {
    pub frame_id: u64,
    pub missing_sequences: Vec<u64>,
}

impl Nack {
    /// Encodes a canonical, bounded NACK payload in network byte order.
    ///
    /// # Errors
    ///
    /// Returns [`ControlPayloadError::NackCount`] when the sequence list is
    /// empty or exceeds [`MAX_NACK_SEQUENCES`], and
    /// [`ControlPayloadError::NonCanonicalSequences`] when the sequence
    /// numbers are not strictly increasing.
    pub fn encode(&self) -> Result<Vec<u8>, ControlPayloadError> {
        if self.missing_sequences.is_empty() || self.missing_sequences.len() > MAX_NACK_SEQUENCES {
            return Err(ControlPayloadError::NackCount(self.missing_sequences.len()));
        }
        if self
            .missing_sequences
            .windows(2)
            .any(|window| window[0] >= window[1])
        {
            return Err(ControlPayloadError::NonCanonicalSequences);
        }
        let count = u16::try_from(self.missing_sequences.len())
            .map_err(|_| ControlPayloadError::NackCount(self.missing_sequences.len()))?;
        let mut encoded = Vec::with_capacity(NACK_PREFIX_LEN + self.missing_sequences.len() * 8);
        encoded.extend_from_slice(&self.frame_id.to_be_bytes());
        encoded.extend_from_slice(&count.to_be_bytes());
        encoded.extend_from_slice(&0_u16.to_be_bytes());
        for sequence in &self.missing_sequences {
            encoded.extend_from_slice(&sequence.to_be_bytes());
        }
        Ok(encoded)
    }

    /// Decodes and validates a canonical NACK payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is truncated, has a mismatched
    /// length, uses reserved bits, declares an invalid sequence count, or
    /// contains sequence numbers that are not strictly increasing.
    pub fn decode(encoded: &[u8]) -> Result<Self, ControlPayloadError> {
        if encoded.len() < NACK_PREFIX_LEN {
            return Err(ControlPayloadError::Truncated);
        }
        if encoded[10..12] != [0, 0] {
            return Err(ControlPayloadError::ReservedBitsSet);
        }
        let count = usize::from(u16::from_be_bytes([encoded[8], encoded[9]]));
        if count == 0 || count > MAX_NACK_SEQUENCES {
            return Err(ControlPayloadError::NackCount(count));
        }
        let expected = NACK_PREFIX_LEN
            .checked_add(count * 8)
            .ok_or(ControlPayloadError::LengthMismatch)?;
        if encoded.len() != expected {
            return Err(ControlPayloadError::LengthMismatch);
        }
        let mut missing_sequences = Vec::with_capacity(count);
        for bytes in encoded[NACK_PREFIX_LEN..].chunks_exact(8) {
            let value = u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| ControlPayloadError::Truncated)?,
            );
            if missing_sequences
                .last()
                .is_some_and(|previous| *previous >= value)
            {
                return Err(ControlPayloadError::NonCanonicalSequences);
            }
            missing_sequences.push(value);
        }
        Ok(Self {
            frame_id: u64::from_be_bytes(
                encoded[0..8]
                    .try_into()
                    .map_err(|_| ControlPayloadError::Truncated)?,
            ),
            missing_sequences,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyframeRequest {
    pub last_decoded_frame_id: u64,
}

impl KeyframeRequest {
    #[must_use]
    pub const fn encode(self) -> [u8; 8] {
        self.last_decoded_frame_id.to_be_bytes()
    }

    /// Decodes a keyframe request from its fixed-width wire representation.
    ///
    /// # Errors
    ///
    /// Returns [`ControlPayloadError::LengthMismatch`] unless `encoded`
    /// contains exactly eight bytes.
    pub fn decode(encoded: &[u8]) -> Result<Self, ControlPayloadError> {
        Ok(Self {
            last_decoded_frame_id: u64::from_be_bytes(
                encoded
                    .try_into()
                    .map_err(|_| ControlPayloadError::LengthMismatch)?,
            ),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ControlPayloadError {
    #[error("control payload is truncated")]
    Truncated,
    #[error("control payload length is invalid")]
    LengthMismatch,
    #[error("control payload reserved bits are set")]
    ReservedBitsSet,
    #[error("NACK sequence count {0} is outside 1..=256")]
    NackCount(usize),
    #[error("NACK sequences must be strictly increasing")]
    NonCanonicalSequences,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nack_round_trip_is_bounded_and_big_endian() {
        let nack = Nack {
            frame_id: 9,
            missing_sequences: vec![10, 12, 15],
        };
        let encoded = nack.encode().unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(&encoded[0..8], &9_u64.to_be_bytes());
        assert_eq!(Nack::decode(&encoded), Ok(nack));
    }

    #[test]
    fn acknowledgement_matches_golden_bytes_and_bit_semantics() {
        let acknowledgement = Acknowledgement {
            cumulative_sequence: 0x0102_0304_0506_0708,
            selective_mask: 0x8000_0000_0000_0005,
        };
        let expected = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x05,
        ];

        assert_eq!(acknowledgement.encode(), expected);
        assert_eq!(Acknowledgement::decode(&expected), Ok(acknowledgement));
        assert!(acknowledgement.acknowledges(acknowledgement.cumulative_sequence - 1));
        assert!(acknowledgement.acknowledges(acknowledgement.cumulative_sequence));
        assert!(acknowledgement.acknowledges(acknowledgement.cumulative_sequence + 1));
        assert!(!acknowledgement.acknowledges(acknowledgement.cumulative_sequence + 2));
        assert!(acknowledgement.acknowledges(acknowledgement.cumulative_sequence + 3));
        assert!(acknowledgement.acknowledges(acknowledgement.cumulative_sequence + 64));
        assert!(!acknowledgement.acknowledges(acknowledgement.cumulative_sequence + 65));
        assert_eq!(
            Acknowledgement::decode(&expected[..ACKNOWLEDGEMENT_PAYLOAD_LEN - 1]),
            Err(ControlPayloadError::LengthMismatch)
        );
    }
}
