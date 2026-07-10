use std::collections::VecDeque;
use thiserror::Error;

const MAX_PACKET_BYTES: usize = 65_536;
const MAX_QUEUE_PACKETS: usize = 4_096;
const MAX_QUEUE_DURATION_US: u64 = 2_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioPacket {
    pub sequence: u64,
    pub timestamp_us: u64,
    pub duration_us: u64,
    pub payload: Vec<u8>,
}

impl AudioPacket {
    pub fn validate(&self) -> Result<(), AudioPacketError> {
        if self.duration_us == 0 || self.duration_us > 120_000 {
            return Err(AudioPacketError::Duration(self.duration_us));
        }
        if self.payload.is_empty() || self.payload.len() > MAX_PACKET_BYTES {
            return Err(AudioPacketError::PayloadLength(self.payload.len()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioQueueConfig {
    pub max_packets: usize,
    pub max_buffered_us: u64,
}

impl AudioQueueConfig {
    pub fn validate(self) -> Result<Self, AudioQueueConfigError> {
        if self.max_packets == 0 || self.max_packets > MAX_QUEUE_PACKETS {
            return Err(AudioQueueConfigError::Packets(self.max_packets));
        }
        if self.max_buffered_us == 0 || self.max_buffered_us > MAX_QUEUE_DURATION_US {
            return Err(AudioQueueConfigError::Duration(self.max_buffered_us));
        }
        Ok(self)
    }
}

impl Default for AudioQueueConfig {
    fn default() -> Self {
        Self {
            max_packets: 12,
            max_buffered_us: 120_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioQueuePush {
    pub dropped_packets: usize,
    pub duplicate: bool,
}

/// Duration- and count-bounded audio queue. Old audio is discarded first;
/// playback must never accumulate seconds of stale latency.
#[derive(Clone, Debug)]
pub struct AudioQueue {
    packets: VecDeque<AudioPacket>,
    buffered_us: u64,
    config: AudioQueueConfig,
}

impl AudioQueue {
    pub fn new(config: AudioQueueConfig) -> Result<Self, AudioQueueConfigError> {
        Ok(Self {
            packets: VecDeque::with_capacity(config.max_packets.min(64)),
            buffered_us: 0,
            config: config.validate()?,
        })
    }

    pub fn push(&mut self, packet: AudioPacket) -> Result<AudioQueuePush, AudioPacketError> {
        packet.validate()?;
        if self
            .packets
            .iter()
            .any(|queued| queued.sequence == packet.sequence)
        {
            return Ok(AudioQueuePush {
                dropped_packets: 0,
                duplicate: true,
            });
        }
        if packet.duration_us > self.config.max_buffered_us {
            return Err(AudioPacketError::ExceedsQueueDuration(packet.duration_us));
        }

        let mut dropped = 0;
        while self.packets.len() >= self.config.max_packets
            || self.buffered_us.saturating_add(packet.duration_us) > self.config.max_buffered_us
        {
            let Some(stale) = self.packets.pop_front() else {
                break;
            };
            self.buffered_us = self.buffered_us.saturating_sub(stale.duration_us);
            dropped += 1;
        }
        self.buffered_us = self.buffered_us.saturating_add(packet.duration_us);
        self.packets.push_back(packet);
        Ok(AudioQueuePush {
            dropped_packets: dropped,
            duplicate: false,
        })
    }

    pub fn pop(&mut self) -> Option<AudioPacket> {
        let packet = self.packets.pop_front()?;
        self.buffered_us = self.buffered_us.saturating_sub(packet.duration_us);
        Some(packet)
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.buffered_us = 0;
    }

    #[must_use]
    pub const fn buffered_us(&self) -> u64 {
        self.buffered_us
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }
}

impl Default for AudioQueue {
    fn default() -> Self {
        Self {
            packets: VecDeque::with_capacity(12),
            buffered_us: 0,
            config: AudioQueueConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AudioQueueConfigError {
    #[error("audio queue packet capacity {0} is outside 1..=4096")]
    Packets(usize),
    #[error("audio queue duration {0}us is outside 1..=2000000us")]
    Duration(u64),
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AudioPacketError {
    #[error("audio packet duration {0}us is outside 1..=120000us")]
    Duration(u64),
    #[error("audio packet payload length {0} is outside 1..=65536")]
    PayloadLength(usize),
    #[error("audio packet duration {0}us exceeds the configured queue duration")]
    ExceedsQueueDuration(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(sequence: u64) -> AudioPacket {
        AudioPacket {
            sequence,
            timestamp_us: sequence * 10_000,
            duration_us: 10_000,
            payload: vec![1],
        }
    }

    #[test]
    fn stale_audio_is_dropped_instead_of_growing_latency() {
        let mut queue = AudioQueue::new(AudioQueueConfig {
            max_packets: 2,
            max_buffered_us: 20_000,
        })
        .unwrap_or_else(|error| panic!("queue config failed: {error}"));
        queue
            .push(packet(1))
            .unwrap_or_else(|error| panic!("{error}"));
        queue
            .push(packet(2))
            .unwrap_or_else(|error| panic!("{error}"));
        let report = queue
            .push(packet(3))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(report.dropped_packets, 1);
        assert_eq!(queue.pop().map(|value| value.sequence), Some(2));
        assert_eq!(queue.buffered_us(), 10_000);
    }
}
