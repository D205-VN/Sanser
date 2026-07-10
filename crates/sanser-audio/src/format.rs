use sanser_core::AudioCodec;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MIN_SAMPLE_RATE: u32 = 8_000;
const MAX_SAMPLE_RATE: u32 = 192_000;
const MAX_CHANNELS: u8 = 8;
const MAX_FRAME_DURATION_US: u64 = 120_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub codec: AudioCodec,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub samples_per_frame: u16,
}

impl AudioFormat {
    pub fn validate(self) -> Result<Self, AudioFormatError> {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&self.sample_rate_hz) {
            return Err(AudioFormatError::SampleRate(self.sample_rate_hz));
        }
        if !(1..=MAX_CHANNELS).contains(&self.channels) {
            return Err(AudioFormatError::Channels(self.channels));
        }
        if self.samples_per_frame == 0 {
            return Err(AudioFormatError::EmptyFrame);
        }
        let duration = self.frame_duration_us();
        if duration == 0 || duration > MAX_FRAME_DURATION_US {
            return Err(AudioFormatError::FrameDuration(duration));
        }
        Ok(self)
    }

    #[must_use]
    pub fn frame_duration_us(self) -> u64 {
        u64::from(self.samples_per_frame) * 1_000_000 / u64::from(self.sample_rate_hz)
    }

    #[must_use]
    pub fn pcm16_payload_len(self) -> Option<usize> {
        usize::from(self.samples_per_frame)
            .checked_mul(usize::from(self.channels))?
            .checked_mul(size_of::<i16>())
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            codec: AudioCodec::Opus,
            sample_rate_hz: 48_000,
            channels: 2,
            samples_per_frame: 480,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AudioFormatError {
    #[error("sample rate {0}Hz is outside 8000..=192000Hz")]
    SampleRate(u32),
    #[error("channel count {0} is outside 1..=8")]
    Channels(u8),
    #[error("audio frame has no samples")]
    EmptyFrame,
    #[error("audio frame duration {0}us is outside 1..=120000us")]
    FrameDuration(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_low_latency_stereo() {
        let format = AudioFormat::default()
            .validate()
            .unwrap_or_else(|error| panic!("default format invalid: {error}"));
        assert_eq!(format.frame_duration_us(), 10_000);
        assert_eq!(format.pcm16_payload_len(), Some(1_920));
    }
}
