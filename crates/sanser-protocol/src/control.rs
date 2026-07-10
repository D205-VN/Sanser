use thiserror::Error;

pub const MAX_NACK_SEQUENCES: usize = 256;
const NACK_PREFIX_LEN: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Nack {
    pub frame_id: u64,
    pub missing_sequences: Vec<u64>,
}

impl Nack {
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
}
