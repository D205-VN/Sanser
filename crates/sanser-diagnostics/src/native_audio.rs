//! Numeric health counters, separate from latency in milliseconds. Deltas are
//! observed intervals only: first samples, long gaps and resets are baselines.
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCounter {
    pub latest: u64,
    pub delta: Option<u64>,
    pub interval_ms: Option<u64>,
    pub observed_increase: u64,
    pub observed_duration_ms: u64,
    pub resets: u64,
    pub samples: u64,
    pub updated_at_ms: u64,
    pub source: &'static str,
    #[serde(skip)]
    epoch: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAudioHealth {
    pub counters: BTreeMap<&'static str, AudioCounter>,
}

fn fields(source: &str) -> &'static [(&'static str, &'static str)] {
    match source {
        "SNVOICE_HEALTH" => &[
            ("voiceCapturedFrames", "voiceCapturedFrames"),
            ("voiceDecoded", "voiceDecodedPackets"),
            ("voiceConcealed", "voiceConcealedPackets"),
            ("voiceFecAttempts", "voiceFecAttempts"),
            ("voiceDropped", "voiceDroppedPackets"),
            ("voiceLate", "voiceLatePackets"),
            ("voiceInvalid", "voiceInvalidPackets"),
            ("voiceRawClippedFrames", "voiceRawClippedFrames"),
            ("voiceCleanClippedFrames", "voiceCleanClippedFrames"),
            ("voiceLimitedFrames", "voiceLimitedFrames"),
            ("voiceCaptureQueueDrops", "voiceCaptureQueueDrops"),
            ("voiceIncomingQueueDrops", "voiceIncomingQueueDrops"),
            ("voiceStaleCaptureBlocks", "voiceStaleCaptureBlocks"),
            ("voicePacketsSent", "voicePacketsSent"),
            ("voicePcmUnderflows", "voicePcmUnderflows"),
            ("voicePcmCapacityDrops", "voicePcmCapacityDrops"),
            ("voicePcmStaleDrops", "voicePcmStaleDrops"),
            ("voicePcmReprimes", "voicePcmReprimes"),
            ("voiceAecBlocks", "voiceAecBlocks"),
            ("voiceAecMissingReference", "voiceAecMissingReference"),
            ("voiceAecDryBlocks", "voiceAecDryBlocks"),
            ("voiceAecResets", "voiceAecResets"),
        ],
        "SNV2_AUDIO_HEALTH" => &[
            ("opusDecoded", "systemDecodedPackets"),
            ("opusPlc", "systemConcealedPackets"),
            ("opusErrors", "systemDecodeErrors"),
        ],
        "SNV2_AUDIO_OUTPUT_HEALTH" => &[
            ("pcmUnderflows", "systemPcmUnderflows"),
            ("pcmCapacityDrops", "systemPcmCapacityDrops"),
            ("pcmStaleDrops", "systemPcmStaleDrops"),
            ("pcmReprimes", "systemPcmReprimes"),
            ("pcmResets", "systemPcmResets"),
        ],
        "SNA1_AUDIO_STATS" => &[
            ("opusDecoded", "systemDecodedPackets"),
            ("opusPlc", "systemConcealedPackets"),
            ("opusErrors", "systemDecodeErrors"),
            ("underflow", "systemJitterUnderflows"),
            ("queueDrop", "systemJitterQueueDrops"),
            ("outputErrors", "systemOutputErrors"),
            ("outputQueueDrops", "systemOutputQueueDrops"),
            ("outputRecoveries", "systemOutputRecoveries"),
            ("outputRouteChanges", "systemOutputRouteChanges"),
            ("outputConverterErrors", "systemConverterErrors"),
            ("outputClockStalls", "systemClockStalls"),
        ],
        _ => &[],
    }
}

fn counter(value: f64) -> Option<u64> {
    if value.is_finite() && (0.0..=1e12).contains(&value) && value.fract() == 0.0 {
        // The upper bound is below both u64::MAX and f64's exact integer limit.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Some(value as u64)
    } else {
        None
    }
}

impl NativeAudioHealth {
    pub fn observe(&mut self, source: &str, values: &BTreeMap<String, f64>, at_ms: u64) {
        let epoch = if source == "SNVOICE_HEALTH" {
            match values.get("voiceHealthEpoch") {
                Some(&value) => match counter(value) {
                    Some(epoch) => Some(epoch),
                    None => return,
                },
                None => None, // Older engines still supply unscoped counters.
            }
        } else {
            None
        };
        let provenance = match source {
            "SNVOICE_HEALTH" => "native-voice",
            "SNA1_AUDIO_STATS" => "legacy-system-audio",
            _ => "snv2-system-audio",
        };
        for &(field, name) in fields(source) {
            let Some(value) = values.get(field).copied().and_then(counter) else {
                continue;
            };
            if let Some(old) = self.counters.get_mut(name) {
                if at_ms <= old.updated_at_ms {
                    continue;
                }
                let reset = old.epoch != epoch || old.source != provenance || value < old.latest;
                let interval = at_ms - old.updated_at_ms;
                old.delta = (!reset && interval <= 5_000).then(|| value - old.latest);
                old.interval_ms = old.delta.map(|_| interval);
                old.observed_increase =
                    old.observed_increase.saturating_add(old.delta.unwrap_or(0));
                old.observed_duration_ms = old
                    .observed_duration_ms
                    .saturating_add(old.interval_ms.unwrap_or(0));
                old.resets = old.resets.saturating_add(u64::from(reset));
                old.samples = old.samples.saturating_add(1);
                old.latest = value;
                old.updated_at_ms = at_ms;
                old.source = provenance;
                old.epoch = epoch;
            } else {
                self.counters.insert(
                    name,
                    AudioCounter {
                        latest: value,
                        delta: None,
                        interval_ms: None,
                        observed_increase: 0,
                        observed_duration_ms: 0,
                        resets: 0,
                        samples: 1,
                        updated_at_ms: at_ms,
                        source: provenance,
                        epoch,
                    },
                );
            }
        }
    }

    #[must_use]
    pub fn increased(&self, key: &str, now_ms: u64) -> bool {
        self.counters.get(key).is_some_and(|value| {
            now_ms >= value.updated_at_ms
                && now_ms - value.updated_at_ms <= 5_000
                && value.delta.is_some_and(|delta| delta > 0)
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn values(epoch: f64, count: f64) -> BTreeMap<String, f64> {
        [
            ("voiceHealthEpoch".into(), epoch),
            ("voicePcmUnderflows".into(), count),
        ]
        .into()
    }
    #[test]
    fn counts_only_observed_intervals_and_rebases_on_epoch_gap_or_decrease() {
        let mut audio = NativeAudioHealth::default();
        audio.observe("SNVOICE_HEALTH", &values(1.0, 90.0), 1000);
        assert!(!audio.increased("voicePcmUnderflows", 1000));
        audio.observe("SNVOICE_HEALTH", &values(1.0, 93.0), 2000);
        assert!(audio.increased("voicePcmUnderflows", 2000));
        assert!(!audio.increased("voicePcmUnderflows", 7001));
        audio.observe("SNVOICE_HEALTH", &values(2.0, 100.0), 3000);
        assert_eq!(audio.counters["voicePcmUnderflows"].delta, None);
        audio.observe("SNVOICE_HEALTH", &values(2.0, 2.0), 4000);
        audio.observe("SNVOICE_HEALTH", &values(2.0, 9.0), 10_000);
        audio.observe("SNVOICE_HEALTH", &values(2.0, 10.0), 11_000);
        let observed = &audio.counters["voicePcmUnderflows"];
        assert_eq!(observed.observed_increase, 4);
        assert_eq!(observed.observed_duration_ms, 2000);
        assert_eq!(observed.resets, 2);
    }
    #[test]
    fn ignores_out_of_order_duplicates_nonintegers_and_unrecognized_fields() {
        let mut audio = NativeAudioHealth::default();
        audio.observe("SNVOICE_HEALTH", &values(1.0, 5.0), 100);
        for (at, count) in [
            (99, 999.0),
            (100, 999.0),
            (101, -1.0),
            (101, 1.5),
            (101, f64::NAN),
        ] {
            audio.observe("SNVOICE_HEALTH", &values(1.0, count), at);
        }
        audio.observe("private", &values(1.0, 1000.0), 200);
        audio.observe("SNVOICE_HEALTH", &[("token".into(), 123.0)].into(), 300);
        assert_eq!(audio.counters.len(), 1);
        assert_eq!(audio.counters["voicePcmUnderflows"].latest, 5);
        assert!(!serde_json::to_string(&audio).unwrap().contains("token"));
    }
    #[test]
    fn separates_voice_from_system_and_falls_back_for_older_engines() {
        let mut audio = NativeAudioHealth::default();
        let fields = [
            ("pcmUnderflows".into(), 3.0),
            ("voicePcmUnderflows".into(), 99.0),
        ]
        .into();
        audio.observe("SNV2_AUDIO_OUTPUT_HEALTH", &fields, 1000);
        audio.observe(
            "SNVOICE_HEALTH",
            &[("voicePcmUnderflows".into(), 20.0)].into(),
            1000,
        );
        audio.observe(
            "SNVOICE_HEALTH",
            &[("voicePcmUnderflows".into(), 21.0)].into(),
            2000,
        );
        assert_eq!(audio.counters["systemPcmUnderflows"].latest, 3);
        assert_eq!(audio.counters["voicePcmUnderflows"].delta, Some(1));
    }
}
