//! Authenticated UDP relay envelope. Media remains end-to-end encrypted inside it.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

pub const MAX_PACKET_BYTES: usize = 65_507;
pub const OVERHEAD: usize = 64;
const HEADER: usize = 32;

// Deliberately not Debug: the key is delivered only over authenticated TLS.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Offer {
    pub allocation_id: [u8; 16],
    pub key: [u8; 32],
    pub address: String,
}

impl Drop for Offer {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.key.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Register = 1,
    Confirm = 2,
    Data = 3,
    Keepalive = 4,
    Challenge = 129,
    Confirmed = 130,
    Forward = 131,
    Alive = 132,
}

#[derive(Debug, Error)]
pub enum DatagramError {
    #[error("invalid relay datagram")]
    Invalid,
    #[error("relay authentication failed")]
    Authentication,
    #[error("replayed or expired relay datagram")]
    Replay,
}

/// A bounded window accepts reordering without accepting a sequence twice.
#[derive(Default)]
pub struct ReplayWindow {
    highest: Option<u64>,
    bits: [u64; 16],
}

impl ReplayWindow {
    #[must_use]
    pub fn accept(&mut self, sequence: u64) -> bool {
        if let Some(highest) = self.highest {
            if sequence <= highest && highest - sequence >= 1024 {
                return false;
            }
            if sequence > highest {
                if sequence - highest >= 1024 {
                    self.bits.fill(0);
                } else {
                    for next in highest + 1..=sequence {
                        let bit = (next % 1024) as usize;
                        self.bits[bit / 64] &= !(1_u64 << (bit % 64));
                    }
                }
            }
        }
        let bit = (sequence % 1024) as usize;
        let mask = 1_u64 << (bit % 64);
        if self.bits[bit / 64] & mask != 0 {
            return false;
        }
        self.bits[bit / 64] |= mask;
        self.highest = Some(self.highest.map_or(sequence, |old| old.max(sequence)));
        true
    }
}

pub struct PacketAuth {
    allocation_id: [u8; 16],
    key: Zeroizing<[u8; 32]>,
    sequence: u64,
    received: ReplayWindow,
}

impl PacketAuth {
    #[must_use]
    pub fn new(allocation_id: [u8; 16], key: [u8; 32]) -> Self {
        Self {
            allocation_id,
            key: Zeroizing::new(key),
            sequence: 0,
            received: ReplayWindow::default(),
        }
    }

    /// Read routing identity without trusting it; authentication is still required.
    #[must_use]
    pub fn allocation_id(packet: &[u8]) -> Option<[u8; 16]> {
        if !(OVERHEAD..=MAX_PACKET_BYTES).contains(&packet.len()) || packet[..4] != *b"SRU1" {
            return None;
        }
        packet[8..24].try_into().ok()
    }

    /// # Errors
    /// Rejects oversized payloads or an exhausted sequence counter.
    pub fn encode(&mut self, kind: Kind, payload: &[u8]) -> Result<Vec<u8>, DatagramError> {
        if payload.len() > MAX_PACKET_BYTES - OVERHEAD {
            return Err(DatagramError::Invalid);
        }
        self.sequence = self.sequence.checked_add(1).ok_or(DatagramError::Invalid)?;
        let mut frame = vec![0; HEADER];
        frame[..4].copy_from_slice(b"SRU1");
        frame[4] = kind as u8;
        frame[8..24].copy_from_slice(&self.allocation_id);
        frame[24..32].copy_from_slice(&self.sequence.to_be_bytes());
        frame.extend_from_slice(payload);
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref())
            .map_err(|_| DatagramError::Invalid)?;
        mac.update(&frame);
        frame.extend_from_slice(&mac.finalize().into_bytes());
        Ok(frame)
    }

    /// # Errors
    /// Rejects invalid identity, direction, authentication or replayed sequence.
    pub fn decode<'a>(
        &mut self,
        frame: &'a [u8],
        from_server: bool,
    ) -> Result<(Kind, &'a [u8]), DatagramError> {
        if Self::allocation_id(frame) != Some(self.allocation_id) || frame[5..8] != [0; 3] {
            return Err(DatagramError::Invalid);
        }
        let kind = match frame[4] {
            1 => Kind::Register,
            2 => Kind::Confirm,
            3 => Kind::Data,
            4 => Kind::Keepalive,
            129 => Kind::Challenge,
            130 => Kind::Confirmed,
            131 => Kind::Forward,
            132 => Kind::Alive,
            _ => return Err(DatagramError::Invalid),
        };
        if (frame[4] >= 128) != from_server {
            return Err(DatagramError::Invalid);
        }
        let end = frame.len() - 32;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_ref())
            .map_err(|_| DatagramError::Invalid)?;
        mac.update(&frame[..end]);
        mac.verify_slice(&frame[end..])
            .map_err(|_| DatagramError::Authentication)?;
        let sequence = u64::from_be_bytes(
            frame[24..32]
                .try_into()
                .map_err(|_| DatagramError::Invalid)?,
        );
        if !self.received.accept(sequence) {
            return Err(DatagramError::Replay);
        }
        Ok((kind, &frame[HEADER..end]))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn datagrams_can_arrive_out_of_order_but_not_be_replayed_or_reflected() {
        let mut sender = PacketAuth::new([1; 16], [2; 32]);
        let mut receiver = PacketAuth::new([1; 16], [2; 32]);
        let first = sender.encode(Kind::Data, b"one").unwrap();
        let second = sender.encode(Kind::Data, b"two").unwrap();
        let third = sender.encode(Kind::Data, b"three").unwrap();
        assert!(receiver.decode(&first, true).is_err());
        assert_eq!(receiver.decode(&third, false).unwrap().1, b"three");
        assert_eq!(receiver.decode(&first, false).unwrap().1, b"one");
        let mut forged = second.clone();
        forged[32] ^= 1;
        assert!(receiver.decode(&forged, false).is_err());
        assert_eq!(receiver.decode(&second, false).unwrap().1, b"two");
        assert!(receiver.decode(&second, false).is_err());
    }
    #[test]
    fn window_is_bounded_across_wrap_and_large_jumps() {
        let mut window = ReplayWindow::default();
        assert!(window.accept(1000));
        assert!(window.accept(2023));
        assert!(!window.accept(1000));
        assert!(window.accept(1001));
        assert!(window.accept(9999));
        assert!(!window.accept(2023));
        assert!(window.accept(9998));
        assert!(!window.accept(9998));
    }
}
