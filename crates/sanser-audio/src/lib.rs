//! Audio primitives shared by native capture and playback paths.
//!
//! The crate owns no device handles. It keeps the latency-sensitive parts
//! deterministic: negotiated formats, bounded buffering and PCM16 gain.

mod format;
mod jitter;
mod queue;

pub use format::{AudioFormat, AudioFormatError};
pub use jitter::{AdaptiveJitter, JitterConfig, JitterConfigError};
pub use queue::{
    AudioPacket, AudioPacketError, AudioQueue, AudioQueueConfig, AudioQueueConfigError,
    AudioQueuePush,
};

/// Applies gain to interleaved signed PCM16 samples without allocating.
///
/// `volume_percent` accepts `0..=200`; values above 100% are saturated instead
/// of wrapping. A muted buffer is cleared directly.
pub fn apply_pcm16_gain(
    samples: &mut [i16],
    volume_percent: u16,
    muted: bool,
) -> Result<(), InvalidVolume> {
    if volume_percent > 200 {
        return Err(InvalidVolume(volume_percent));
    }
    if muted || volume_percent == 0 {
        samples.fill(0);
        return Ok(());
    }
    let gain = i32::from(volume_percent);
    for sample in samples {
        let scaled = i32::from(*sample) * gain / 100;
        *sample = match i16::try_from(scaled) {
            Ok(value) => value,
            Err(_) if scaled.is_negative() => i16::MIN,
            Err(_) => i16::MAX,
        };
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("volume {0}% is outside 0..=200")]
pub struct InvalidVolume(pub u16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_is_in_place_and_saturating() {
        let mut samples = [20_000, -20_000, 100];
        apply_pcm16_gain(&mut samples, 200, false)
            .unwrap_or_else(|error| panic!("gain failed: {error}"));
        assert_eq!(samples, [i16::MAX, i16::MIN, 200]);
        apply_pcm16_gain(&mut samples, 100, true)
            .unwrap_or_else(|error| panic!("mute failed: {error}"));
        assert_eq!(samples, [0, 0, 0]);
    }
}
