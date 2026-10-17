//! One numeric contract for legacy and SNV2 engines. These percentiles describe
//! periodic telemetry observations (some are interval averages), not raw frames.
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

const WINDOW: usize = 120;
const FRESH_MS: u64 = 5_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMetric {
    pub latest_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub samples: usize,
    pub updated_at_ms: u64,
    pub source: &'static str,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePerformance {
    pub metrics: BTreeMap<&'static str, NativeMetric>,
    #[serde(skip)]
    windows: BTreeMap<&'static str, VecDeque<f64>>,
}

impl NativePerformance {
    pub fn observe(&mut self, source: &str, values: &BTreeMap<String, f64>, at_ms: u64) {
        // Matching both source and field prevents an input retry RTT from
        // silently becoming a network RTT, and keeps capture wait out of work.
        let fields: &[(&str, &str, &str)] = match source {
            "SNCONTROL_TIMING" => &[
                ("appRttMs", "controlAppRtt", "legacy-control"),
                ("socketRttMs", "socketRtt", "legacy-control"),
                ("wireEstimateMs", "wireRttEstimate", "legacy-control"),
                ("macSendQueueMs", "clientSendQueue", "legacy-control"),
                ("hostReceiveQueueMs", "hostReceiveQueue", "legacy-control"),
                ("hostControlWorkMs", "hostControlWork", "legacy-control"),
                ("hostSendLockMs", "hostSendLock", "legacy-control"),
                ("hostSendCallMs", "hostSendCall", "legacy-control"),
                ("macReceiveQueueMs", "clientReceiveQueue", "legacy-control"),
            ],
            "SNV2_CONTROL_TIMING" => &[("appRttMs", "controlAppRtt", "snv2-control")],
            "SNROUTE_HEALTH" => &[("controlSilenceMs", "controlSilence", "legacy-control")],
            // SNINPUT_RTT is emitted by legacy control-pong, despite its name.
            "SNINPUT_RTT" => &[("rttMs", "controlAppRttFallback", "legacy-pong")],
            "SNINPUT_ACK" => &[
                ("rttMs", "inputAckRtt", "legacy-input"),
                ("hostProcessMs", "hostInput", "legacy-input"),
            ],
            "SNV2_INPUT_TIMING" => &[
                ("avgRttMs", "inputAckRtt", "snv2-input"),
                ("hostAvgWorkMs", "hostInput", "snv2-input"),
            ],
            "SNU1_ECHO" => &[
                ("rttMs", "udpEchoRtt", "legacy-echo"),
                ("hostHoldMs", "echoHostHold", "legacy-echo"),
            ],
            "SNU1_RX_QUEUE" => &[("receiveQueueAvgMs", "udpReceiveQueue", "legacy-receive")],
            "SNV1_HOST_TIMING" | "SNV1_STAGE_PROFILE" => &[
                ("captureAvgMs", "captureWork", "legacy-host"),
                ("captureWaitAvgMs", "captureWait", "legacy-host"),
                ("encodeAvgMs", "encodeWork", "legacy-host"),
                ("sendAvgMs", "sendTotal", "legacy-host"),
                ("udpPacedAvgMs", "pacingWait", "legacy-host"),
                ("udpSocketAvgMs", "socketSend", "legacy-host"),
            ],
            "SNV2_HOST_TIMING" => &[
                ("encodeQueueAvgMs", "encodeQueue", "snv2-host"),
                ("encodeAndPublishAvgMs", "encodeAndPublish", "snv2-host"),
            ],
            "SNV1_CLIENT_STATS" => &[
                ("decodeAvgMs", "decodeWork", "legacy-video"),
                ("jitterMs", "arrivalJitter", "legacy-video"),
            ],
            "SNV1_RENDER_STATS" => &[
                ("avgRenderAgeMs", "frameQueue", "legacy-render"),
                ("renderGpuMs", "renderGpu", "legacy-render"),
            ],
            "SNV2_VIDEO_HEALTH" => &[
                ("decodeWorkAvgMs", "decodeWork", "snv2-video"),
                ("decodeQueueAvgMs", "decodeQueue", "snv2-video"),
                ("conversionAvgMs", "colorConversion", "snv2-video"),
                ("paintAvgMs", "paintWork", "snv2-video"),
                ("decodedToPaintAvgMs", "decodedToPaint", "snv2-video"),
            ],
            _ => &[],
        };
        for &(field, metric, provenance) in fields {
            let Some(&value) = values.get(field) else {
                continue;
            };
            if !value.is_finite() || !(0.0..=1e12).contains(&value) {
                continue;
            }
            if self
                .metrics
                .get(metric)
                .is_some_and(|old| old.updated_at_ms > at_ms)
            {
                continue;
            }
            let window = self.windows.entry(metric).or_default();
            if window.len() == WINDOW {
                window.pop_front();
            }
            window.push_back(value);
            let mut sorted: Vec<f64> = window.iter().copied().collect();
            sorted.sort_by(f64::total_cmp);
            let percentile =
                |percent: usize| sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)];
            self.metrics.insert(
                metric,
                NativeMetric {
                    latest_ms: value,
                    p50_ms: percentile(50),
                    p95_ms: percentile(95),
                    max_ms: sorted[sorted.len() - 1],
                    samples: sorted.len(),
                    updated_at_ms: at_ms,
                    source: provenance,
                },
            );
        }
    }

    #[must_use]
    pub fn findings(&self, now_ms: u64, fps: u16) -> Vec<&'static str> {
        let fresh = |key| {
            self.metrics
                .get(key)
                .filter(|m| now_ms >= m.updated_at_ms && now_ms - m.updated_at_ms <= FRESH_MS)
                .map(|m| m.latest_ms)
        };
        let mut findings = Vec::new();
        if fresh("wireRttEstimate").is_some_and(|v| v > 80.0) {
            findings.push("path-or-os-delay");
        } else if fresh("controlAppRtt")
            .or_else(|| fresh("controlAppRttFallback"))
            .is_some_and(|v| v > 80.0)
        {
            findings.push("control-round-trip-high");
        }
        if let (Some(input), Some(control)) = (
            fresh("inputAckRtt"),
            fresh("controlAppRtt").or_else(|| fresh("controlAppRttFallback")),
        ) && input > control + 10.0
        {
            findings.push("input-delivery-overhead");
        }
        let budget = 1_000.0 / f64::from(fps.max(1));
        for (metric, threshold, finding) in [
            ("captureWork", budget, "capture-over-budget"),
            ("encodeWork", budget, "encode-over-budget"),
            ("encodeAndPublish", budget, "encode-publish-over-budget"),
            ("decodeWork", budget, "decode-over-budget"),
            ("decodeQueue", 5.0, "decode-queue-high"),
            ("frameQueue", 5.0, "frame-queue-high"),
            ("udpReceiveQueue", 5.0, "receive-queue-high"),
        ] {
            if fresh(metric).is_some_and(|v| v > threshold) {
                findings.push(finding);
            }
        }
        findings
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    fn values(items: &[(&str, f64)]) -> BTreeMap<String, f64> {
        items
            .iter()
            .map(|&(key, value)| (key.into(), value))
            .collect()
    }
    #[test]
    fn common_names_keep_input_and_control_separate_and_missing_unknown() {
        let mut p = NativePerformance::default();
        p.observe("SNV2_CONTROL_TIMING", &values(&[("appRttMs", 10.0)]), 100);
        p.observe("SNV2_INPUT_TIMING", &values(&[("avgRttMs", 190.0)]), 100);
        assert_eq!(p.metrics["controlAppRtt"].latest_ms, 10.0);
        assert_eq!(p.metrics["inputAckRtt"].latest_ms, 190.0);
        assert!(!p.metrics.contains_key("wireRttEstimate"));
        assert_eq!(p.findings(100, 60), vec!["input-delivery-overhead"]);
        let mut legacy = NativePerformance::default();
        legacy.observe("SNINPUT_RTT", &values(&[("rttMs", 10.0)]), 100);
        legacy.observe("SNINPUT_ACK", &values(&[("rttMs", 190.0)]), 100);
        assert_eq!(legacy.findings(100, 60), p.findings(100, 60));
    }
    #[test]
    fn bounded_percentiles_reject_invalid_stale_and_private_fields() {
        let mut p = NativePerformance::default();
        for n in 1_u32..=240 {
            p.observe(
                "SNV2_CONTROL_TIMING",
                &values(&[("appRttMs", f64::from(n)), ("token", 123.0)]),
                u64::from(n),
            );
        }
        assert_eq!(p.metrics["controlAppRtt"].samples, 120);
        assert_eq!(p.metrics["controlAppRtt"].p50_ms, 180.0);
        assert_eq!(p.metrics["controlAppRtt"].p95_ms, 234.0);
        p.observe(
            "SNV2_CONTROL_TIMING",
            &values(&[("appRttMs", f64::NAN)]),
            241,
        );
        p.observe("SNV2_CONTROL_TIMING", &values(&[("appRttMs", 1.0)]), 100);
        assert_eq!(p.metrics["controlAppRtt"].latest_ms, 240.0);
        assert_eq!(p.findings(6000, 60), Vec::<&str>::new());
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("token") && !json.contains("windows"));
    }
    #[test]
    fn capture_wait_and_combined_stages_are_not_misreported_as_network_or_capture() {
        let mut p = NativePerformance::default();
        p.observe(
            "SNV1_HOST_TIMING",
            &values(&[("captureWaitAvgMs", 100.0), ("captureAvgMs", 0.2)]),
            100,
        );
        assert_eq!(p.findings(100, 60), Vec::<&str>::new());
        p.observe(
            "SNV2_HOST_TIMING",
            &values(&[("encodeAndPublishAvgMs", 50.0)]),
            100,
        );
        assert!(!p.metrics.contains_key("encodeWork"));
        assert_eq!(p.findings(100, 60), vec!["encode-publish-over-budget"]);
        p.observe("SNV2_CONTROL_TIMING", &values(&[("appRttMs", 200.0)]), 100);
        assert!(p.findings(100, 60).contains(&"control-round-trip-high"));
        assert!(!p.findings(100, 60).contains(&"path-or-os-delay"));
    }
}
