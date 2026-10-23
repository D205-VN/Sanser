//! Local, bounded performance reports. Only allowlisted numeric engine metrics
//! are retained; native output, credentials and screen/input content are not.
use crate::models::LaunchEngineRequest;
use serde::Serialize;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, SyncSender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_REPORTS: usize = 20;
const MAX_SAMPLES: usize = 120;
const MAX_REPORT_BYTES: u64 = 256 * 1024;
const SOURCES: &[&str] = &[
    "SNHOST_CAPTURE_RECOVERY",
    "SNINPUT_OS_REJECTED",
    "FEC_HEALTH",
    "DIRECT_PATH_HEALTH",
    "SNV2_TRANSPORT_HEALTH",
    "SNSHARE_PEER",
    "SNV2_HOST_TIMING",
    "SNV2_VIDEO_HEALTH",
    "SNV2_AUDIO_HEALTH",
    "SNV2_AUDIO_OUTPUT_HEALTH",
    "SNA1_AUDIO_STATS",
    "SNV2_INPUT_HEALTH",
    "SNV2_CONTROL_TIMING",
    "SNROUTE_HEALTH",
    "SNV2_INPUT_TIMING",
    "SNV2_INPUT_HOST_TIMING",
    "SNV2_INPUT_ACK_TIMEOUT",
    "SNV2_INPUT_SUSPENDED",
    "SNV2_INPUT_RECOVERED",
    "SNV2_INPUT_LEASE_EXPIRED",
    "SNV2_CAPTURE_PAUSED",
    "SNV2_CAPTURE_RESUMED",
    "SNINPUT_ACK",
    "SNU1_ECHO",
    "SNU1_RX_QUEUE",
    "SNCONTROL_TIMING",
    "SNINPUT_TIMING",
    "SNV1_STATS",
    "SNV1_STAGE_PROFILE",
    "SNV1_HOST_TIMING",
    "SNV1_ADAPT",
    "SNV1_CLIENT_STATS",
    "SNV1_RENDER_STATS",
    "SNINPUT_RTT",
    "SNINPUT_ACKED",
    "SNINPUT_LATENCY",
    "SNU1_STATS",
    "SNU1_STARTUP",
];
const FIELDS: &[&str] = &[
    "captureRecoveryCode",
    "inputRejected",
    "fecNegotiated",
    "fecTargetPercent",
    "fecVideoBytes",
    "fecParityBytes",
    "fecSent",
    "fecRecovered",
    "fecExpired",
    "fecRejected",
    "fecBudgetSkipped",
    "fecReceiverLate",
    "fecCongestionPauses",
    "pathCount",
    "pathSwitches",
    "pathSendDrops",
    "pathRttMs",
    "pathLossPercent",
    "opusDecoded",
    "opusPlc",
    "opusLate",
    "opusReordered",
    "opusDropped",
    "opusErrors",
    "opusTxPackets",
    "opusTxBytes",
    "played",
    "underflow",
    "queueDrop",
    "avHold",
    "avDrop",
    "bufferMs",
    "outputSubmittedFrames",
    "outputCompletedFrames",
    "outputSignalFrames",
    "outputQueuedFrames",
    "outputStarts",
    "outputRecoveries",
    "outputErrors",
    "outputQueueDrops",
    "outputLastError",
    "pcmUnderflows",
    "pcmCapacityDrops",
    "pcmStaleDrops",
    "pcmReprimes",
    "pcmResets",
    "ready",
    "running",
    "stopReason",
    "repaired",
    "repairCacheMisses",
    "dispatched",
    "replaced",
    "referenceDrops",
    "sendQueueDrops",
    "keyframeSignals",
    "keyframeRequests",
    "pauses",
    "videoStateCode",
    "pauseReason",
    "completedFrame",
    "sentFrame",
    "feedbackSerial",
    "feedbackCongested",
    "decoderOutputErrors",
    "decoderOutputDrops",
    "backoffMs",
    "encoderDropped",
    "captured",
    "submitted",
    "converted",
    "encodedQueueDrops",
    "decodeQueueExpired",
    "decodeQueueMaxMs",
    "decodeWorkAvgMs",
    "decodeQueueAvgMs",
    "conversionAvgMs",
    "paintAvgMs",
    "decodedToPaintAvgMs",
    "encodeQueueAvgMs",
    "encodeAndPublishAvgMs",
    "queuedKeys",
    "inputSendQueueAvgMs",
    "hostQueueAvgMs",
    "hostQueueMaxMs",
    "inputRetries",
    "deadlineMs",
    "ageMs",
    "attempts",
    "queued",
    "lastAck",
    "resetAcknowledged",
    "coalescedMoves",
    "queuedButtons",
    "deliveredKeys",
    "deliveredButtons",
    "hostHoldMs",
    "residualMs",
    "sendCallMs",
    "queuedDatagrams",
    "highWaterDatagrams",
    "receiveDropped",
    "receiveExpired",
    "receiveOversized",
    "receiveQueueAvgMs",
    "receiveQueueMaxMs",
    "hostSendLockMs",
    "hostSendCallMs",
    "appRttMs",
    "controlSilenceMs",
    "socketRttMs",
    "wireEstimateMs",
    "macSendQueueMs",
    "hostReceiveQueueMs",
    "hostControlWorkMs",
    "macReceiveQueueMs",
    "deliveredMbps",
    "estimatedDeliveryMbps",
    "gpuInput",
    "avgRenderAgeMs",
    "maxRenderAgeMs",
    "renderGpuMs",
    "targetDelayMs",
    "adaptiveDelayMs",
    "encoderSkipped",
    "fps",
    "targetFps",
    "mbps",
    "packets",
    "decoded",
    "dropped",
    "decodeErrors",
    "jitterMs",
    "decodeAvgMs",
    "decodeMaxMs",
    "latencyDropped",
    "latencyLateMaxMs",
    "keyframeWaitDropped",
    "rendered",
    "queueDepth",
    "droppedQueue",
    "droppedLate",
    "avgPresentLateMs",
    "maxPresentLateMs",
    "rttMs",
    "avgRttMs",
    "maxRttMs",
    "hostProcessMs",
    "hostProcessMaxMs",
    "pending",
    "hostAvgWorkMs",
    "hostMaxWorkMs",
    "captureAvgMs",
    "captureMaxMs",
    "captureWaitAvgMs",
    "capturePaused",
    "captureSamples",
    "gpuPoolBusyDrops",
    "encodeAvgMs",
    "encodeMaxMs",
    "sendAvgMs",
    "sendMaxMs",
    "udpPacedAvgMs",
    "udpWaitOvershootAvgMs",
    "udpSocketAvgMs",
    "datagrams",
    "incomplete",
    "missingFragments",
    "droppedAssemblies",
    "retransmitCompleted",
    "nackSent",
    "nackRecovered",
    "nackTimedOut",
    "replayRejected",
    "jitterLate",
    "jitterPending",
    "rawDatagrams",
    "unexpectedPeer",
    "controlDatagrams",
    "videoDatagrams",
    "completed",
    "authRejected",
    "peerRejected",
    "malformed",
    "startupTimeout",
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sample {
    at_ms: u64,
    source: String,
    values: BTreeMap<String, f64>,
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

pub(crate) fn parse_sample(line: &[u8]) -> Option<Sample> {
    let line = std::str::from_utf8(line).ok()?;
    let mut words = line.split_whitespace();
    let source = words.next()?;
    if !SOURCES.contains(&source) {
        return None;
    }
    let values: BTreeMap<_, _> = words
        .filter_map(|word| {
            let (key, value) = word.split_once('=')?;
            if !FIELDS.contains(&key) {
                return None;
            }
            let value: f64 = value.parse().ok()?;
            let valid_range = if key == "outputLastError" {
                (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) && value.fract() == 0.0
            } else {
                (0.0..=1e12).contains(&value)
            };
            (value.is_finite() && valid_range).then(|| (key.to_owned(), value))
        })
        .collect();
    (!values.is_empty()).then(|| Sample {
        at_ms: now_ms(),
        source: source.into(),
        values,
    })
}

/// Probe RTT includes application handling; live Wire RTT is an estimate that
/// still includes OS queues. A rise is a diagnostic signal, not proof of NAT/QoS.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RouteHealth {
    attempt_id: String,
    probe: sanser_p2p::ProbeStatistics,
    live_wire_rtt_ms: Option<f64>,
    echo_rtt_ms: Option<f64>,
    echo_host_hold_ms: Option<f64>,
    echo_updated_at_ms: Option<u64>,
    status: &'static str,
    consecutive_elevated: u32,
    #[serde(skip)]
    last_sample_at_ms: Option<u64>,
}
impl RouteHealth {
    fn observe(&mut self, wire_ms: f64, at_ms: u64, media_at_ms: Option<u64>) {
        self.live_wire_rtt_ms = Some(wire_ms);
        if self.probe.samples < 3 {
            self.status = "insufficient-probe-samples";
            return;
        }
        if media_at_ms.is_none_or(|started| at_ms.saturating_sub(started) < 2_000) {
            self.status = "awaiting-media-warmup";
            return;
        }
        if self
            .last_sample_at_ms
            .is_some_and(|last| at_ms.saturating_sub(last) > 5_000)
        {
            self.consecutive_elevated = 0;
        }
        self.last_sample_at_ms = Some(at_ms);
        let threshold = 80.0_f64
            .max(self.probe.median_ms * 2.0)
            .max(self.probe.median_ms + 40.0);
        if wire_ms > threshold {
            self.consecutive_elevated = self.consecutive_elevated.saturating_add(1);
            self.status = if self.consecutive_elevated >= 3 {
                "elevated-after-media"
            } else {
                "checking-elevated-rtt"
            };
        } else {
            self.consecutive_elevated = 0;
            self.status = if self.probe.median_ms >= 80.0 && wire_ms >= 80.0 {
                "slow-at-nomination"
            } else {
                "no-sustained-rise"
            };
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u8,
    #[serde(rename = "reportId")]
    id: String,
    version: &'static str,
    session_id: Option<String>,
    machine: String,
    platform: &'static str,
    kind: crate::models::EngineKind,
    wire_protocol: &'static str,
    requested_width: u16,
    requested_height: u16,
    requested_codec: crate::models::VideoCodec,
    route: &'static str,
    relay_transport: Option<&'static str>,
    direct_check: &'static str,
    route_health: Option<RouteHealth>,
    #[serde(skip)]
    media_started_at_ms: Option<u64>,
    requested_fps: u16,
    requested_bitrate_kbps: u32,
    ultra_low_latency: bool,
    started_at_ms: u64,
    updated_at_ms: u64,
    ended: bool,
    sample_count: u64,
    delay_sample_count: u64,
    status: &'static str,
    maxima: BTreeMap<String, f64>,
    performance: sanser_diagnostics::NativePerformance,
    performance_findings: Vec<&'static str>,
    samples: VecDeque<Sample>,
}
impl Report {
    fn new(request: &LaunchEngineRequest) -> Self {
        Self {
            schema_version: 2,
            id: uuid::Uuid::new_v4().to_string(),
            version: env!("CARGO_PKG_VERSION"),
            session_id: request
                .session_id
                .as_deref()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .map(|id| id.to_string()),
            machine: crate::device_identity::computer_name(),
            platform: std::env::consts::OS,
            kind: request.kind,
            wire_protocol: if request.wire_protocol.as_deref() == Some("snv2") {
                "snv2"
            } else {
                "legacy"
            },
            requested_width: request.width,
            requested_height: request.height,
            requested_codec: request.codec,
            direct_check: match request.direct_check.as_deref() {
                Some("passed") => "passed",
                Some("relay-selected") => "relay-selected",
                Some("timeout") => "timeout",
                Some("no-direct-route") => "no-direct-route",
                Some("gather-failed") => "gather-failed",
                Some("signaling-failed") => "signaling-failed",
                Some("failed") => "failed",
                _ => "not-reported",
            },
            route: if request.relay { "relay" } else { "direct" },
            relay_transport: if request.relay {
                match request.relay_transport.as_deref() {
                    Some("udp") => Some("udp"),
                    Some("wss") => Some("wss"),
                    _ => None,
                }
            } else {
                None
            },
            route_health: if request.relay {
                None
            } else {
                request
                    .direct_probe
                    .clone()
                    .zip(request.direct_attempt_id.clone())
                    .map(|(probe, attempt_id)| RouteHealth {
                        attempt_id,
                        probe,
                        live_wire_rtt_ms: None,
                        echo_rtt_ms: None,
                        echo_host_hold_ms: None,
                        echo_updated_at_ms: None,
                        status: "awaiting-media-warmup",
                        consecutive_elevated: 0,
                        last_sample_at_ms: None,
                    })
            },
            media_started_at_ms: None,
            requested_fps: request.fps,
            requested_bitrate_kbps: request.bitrate_kbps,
            ultra_low_latency: request.ultra_low_latency,
            started_at_ms: now_ms(),
            updated_at_ms: now_ms(),
            ended: false,
            sample_count: 0,
            delay_sample_count: 0,
            status: "awaiting-measurements",
            maxima: BTreeMap::new(),
            performance: sanser_diagnostics::NativePerformance::default(),
            performance_findings: Vec::new(),
            samples: VecDeque::new(),
        }
    }
    fn observe(&mut self, sample: Sample) {
        self.performance
            .observe(&sample.source, &sample.values, sample.at_ms);
        self.performance_findings = self.performance.findings(sample.at_ms, self.requested_fps);
        if matches!(
            sample.source.as_str(),
            "SNU1_STARTUP" | "SNV1_CLIENT_STATS" | "SNV1_RENDER_STATS" | "SNV2_VIDEO_HEALTH"
        ) && ["decoded", "rendered"]
            .iter()
            .any(|key| sample.values.get(*key).is_some_and(|value| *value > 0.0))
        {
            self.media_started_at_ms.get_or_insert(sample.at_ms);
        }
        if sample.source == "SNU1_ECHO"
            && let (Some(rtt), Some(hold)) =
                (sample.values.get("rttMs"), sample.values.get("hostHoldMs"))
            && let Some(health) = self.route_health.as_mut()
        {
            health.echo_rtt_ms = Some(*rtt);
            health.echo_host_hold_ms = Some(*hold);
            health.echo_updated_at_ms = Some(sample.at_ms);
        }
        if sample.source == "SNCONTROL_TIMING"
            && let Some(wire) = sample.values.get("wireEstimateMs")
            && let Some(health) = self.route_health.as_mut()
        {
            health.observe(*wire, sample.at_ms, self.media_started_at_ms);
        }
        self.sample_count += 1;
        // RTT and local processing durations use one machine's monotonic clock.
        // Cross-machine wall-clock frame ages are deliberately excluded.
        let delayed = sample.values.iter().any(|(key, value)| match key.as_str() {
            "controlSilenceMs" => *value >= 15_000.0,
            "inputSendQueueAvgMs" | "hostQueueAvgMs" | "hostAvgWorkMs" => *value > 5.0,
            "rttMs" | "avgRttMs" | "maxRttMs" | "appRttMs" | "socketRttMs" => {
                *value >= if self.ultra_low_latency { 15.0 } else { 100.0 }
            }
            "avgRenderAgeMs" | "renderGpuMs" => {
                *value >= if self.ultra_low_latency { 4.0 } else { 50.0 }
            }
            "decodeAvgMs"
            | "encodeAvgMs"
            | "captureAvgMs"
            | "sendAvgMs"
            | "decodeWorkAvgMs"
            | "decodeQueueAvgMs"
            | "conversionAvgMs"
            | "paintAvgMs"
            | "decodedToPaintAvgMs"
            | "encodeQueueAvgMs"
            | "encodeAndPublishAvgMs" => *value >= 50.0,
            "avgPresentLateMs" | "maxPresentLateMs" | "latencyLateMaxMs" => *value >= 80.0,
            "latencyDropped" | "keyframeWaitDropped" | "decodeQueueExpired" => *value > 0.0,
            _ => false,
        });
        self.delay_sample_count += u64::from(delayed);
        self.updated_at_ms = sample.at_ms;
        for (key, value) in &sample.values {
            let maximum = self
                .maxima
                .entry(format!("{}.{}", sample.source, key))
                .or_default();
            *maximum = maximum.max(*value);
        }
        let video_started = [
            "SNU1_STARTUP.decoded",
            "SNV1_CLIENT_STATS.decoded",
            "SNV1_RENDER_STATS.rendered",
            "SNV2_VIDEO_HEALTH.decoded",
        ]
        .iter()
        .any(|key| self.maxima.get(*key).is_some_and(|value| *value > 0.0));
        self.status = if self.maxima.get("SNU1_STARTUP.startupTimeout") == Some(&1.0) {
            "startup-timeout"
        } else if self.maxima.contains_key("SNU1_STARTUP.rawDatagrams") && !video_started {
            "awaiting-video"
        } else if self.delay_sample_count > 0 {
            "delay-observed"
        } else {
            "samples-collected"
        };
        if self.samples.len() == MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
}

// Keep persistence/analysis independent of the GUI runtime. In particular, a
// Windows unit-test executable does not carry Tauri's Common Controls manifest.
pub(crate) type RouteHealthObserver = Box<dyn Fn(&RouteHealth) + Send>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceUpdate {
    report_id: String,
    session_id: Option<String>,
    route_health: Option<RouteHealth>,
    kind: crate::models::EngineKind,
    wire_protocol: &'static str,
    route: &'static str,
    relay_transport: Option<&'static str>,
    started_at_ms: u64,
    ended: bool,
    performance: sanser_diagnostics::NativePerformance,
    findings: Vec<&'static str>,
}
impl Report {
    fn performance_update(&self) -> PerformanceUpdate {
        PerformanceUpdate {
            report_id: self.id.clone(),
            session_id: self.session_id.clone(),
            route_health: self.route_health.clone(),
            kind: self.kind,
            wire_protocol: self.wire_protocol,
            route: self.route,
            relay_transport: self.relay_transport,
            started_at_ms: self.started_at_ms,
            ended: self.ended,
            performance: self.performance.clone(),
            findings: self.performance.findings(now_ms(), self.requested_fps),
        }
    }
}
pub(crate) type PerformanceObserver = Box<dyn Fn(&PerformanceUpdate) + Send>;

pub(crate) fn start(
    directory: &Path,
    request: &LaunchEngineRequest,
    on_route_health: Option<RouteHealthObserver>,
    on_performance: Option<PerformanceObserver>,
) -> std::io::Result<SyncSender<Sample>> {
    fs::create_dir_all(directory)?;
    let mut report = Report::new(request);
    let path = directory.join(format!(
        "connection-{}-{}.json",
        report.started_at_ms,
        uuid::Uuid::new_v4()
    ));
    save(&path, &report)?;
    prune(directory)?;
    if let Some(observer) = &on_performance {
        observer(&report.performance_update());
    }
    let (sender, receiver) = mpsc::sync_channel::<Sample>(128);
    thread::Builder::new()
        .name("sanser-connection-report".into())
        .spawn(move || {
            let mut flushed = Instant::now();
            let mut notified = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_secs(5)) {
                    Ok(sample) => {
                        let control_sample =
                            matches!(sample.source.as_str(), "SNCONTROL_TIMING" | "SNU1_ECHO");
                        report.observe(sample);
                        if control_sample
                            && let Some(health) = &report.route_health
                            && let Some(observer) = &on_route_health
                        {
                            observer(health);
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        report.ended = true;
                        report.updated_at_ms = now_ms();
                        if let Some(observer) = &on_performance {
                            observer(&report.performance_update());
                        }
                        let _ = save(&path, &report);
                        break;
                    }
                }
                if notified.elapsed() >= Duration::from_secs(1) {
                    if let Some(observer) = &on_performance {
                        observer(&report.performance_update());
                    }
                    notified = Instant::now();
                }
                if flushed.elapsed() >= Duration::from_secs(5) {
                    let _ = save(&path, &report);
                    flushed = Instant::now();
                }
            }
        })?;
    Ok(sender)
}

fn save(path: &Path, report: &Report) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = serde_json::to_vec(report)?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err(std::io::Error::other("report size limit"));
    }
    let temp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    drop(file);
    fs::rename(temp, path)
}
fn paths(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("connection-")
                        && path
                            .extension()
                            .is_some_and(|extension| extension == "json")
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}
fn prune(directory: &Path) -> std::io::Result<()> {
    let paths = paths(directory)?;
    for path in paths.iter().take(paths.len().saturating_sub(MAX_REPORTS)) {
        fs::remove_file(path)?;
    }
    Ok(())
}
pub(crate) fn recent(directory: &Path) -> Vec<serde_json::Value> {
    paths(directory)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .take(MAX_REPORTS)
        .filter_map(|path| {
            if fs::metadata(&path).ok()?.len() > MAX_REPORT_BYTES {
                return None;
            }
            serde_json::from_slice(&fs::read(path).ok()?).ok()
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
// These assertions check exact decoded/serialized constants, not approximate
// floating-point results from real latency measurements.
#[allow(clippy::float_cmp)]
mod tests {
    #[test]
    fn capture_and_input_rejections_keep_codes_without_app_or_key_content() {
        let code = 0x887a_002b_u32;
        let capture = super::parse_sample(format!("SNHOST_CAPTURE_RECOVERY captureRecoveryCode={code} stage=Map_staging app=private.exe path=C:\\private").as_bytes()).unwrap();
        assert_eq!(capture.values.len(), 1);
        assert_eq!(capture.values["captureRecoveryCode"], f64::from(code));
        let input = super::parse_sample(
            b"SNINPUT_OS_REJECTED inputRejected=1 kind=3 key=secret retryHeldRelease=1",
        )
        .unwrap();
        assert_eq!(input.values.len(), 1);
        assert_eq!(input.values["inputRejected"], 1.0);
    }
    use super::*;
    #[test]
    fn live_path_report_retains_numbers_without_addresses_or_probe_keys() {
        let sample=parse_sample(b"DIRECT_PATH_HEALTH pathCount=3 pathSwitches=1 pathSendDrops=0 pathRttMs=13 pathLossPercent=0 endpoint=192.0.2.1:50000 key=secret token=secret").unwrap();
        assert_eq!(sample.values.len(), 5);
        assert_eq!(sample.values["pathRttMs"], 13.0);
        assert!(!serde_json::to_string(&sample).unwrap().contains("secret"));
    }
    #[test]
    fn audio_reports_keep_only_numeric_quality_counters() {
        let sample = parse_sample(b"SNV2_AUDIO_HEALTH opusDecoded=200 opusPlc=3 opusLate=2 opusErrors=0 opusTxBytes=16000 token=123 samples=456").unwrap();
        assert_eq!(sample.values.len(), 5);
        assert_eq!(sample.values["opusPlc"], 3.0);
        assert!(!sample.values.contains_key("token"));
        assert!(!sample.values.contains_key("samples"));
        let output = parse_sample(b"SNV2_AUDIO_OUTPUT_HEALTH pcmUnderflows=4 pcmCapacityDrops=1 pcmStaleDrops=2 pcmReprimes=3 pcmResets=1 samples=456 device=123 token=789").unwrap();
        assert_eq!(output.values.len(), 5);
        assert_eq!(output.values["pcmUnderflows"], 4.0);
        assert!(!output.values.contains_key("samples"));
        assert!(!output.values.contains_key("device"));
        assert!(!output.values.contains_key("token"));
        let legacy=parse_sample(b"SNA1_AUDIO_STATS opusDecoded=600 played=2 underflow=1 queueDrop=14 avHold=0 avDrop=0 bufferMs=5 outputSubmittedFrames=144000 outputCompletedFrames=143760 outputQueuedFrames=240 outputStarts=2 outputRecoveries=1 outputErrors=1 outputQueueDrops=14 outputLastError=-50 device=123 token=789").unwrap();
        assert_eq!(legacy.values["outputCompletedFrames"], 143_760.0);
        assert_eq!(legacy.values["outputLastError"], -50.0);
        assert_eq!(legacy.values["played"], 2.0);
        assert!(!legacy.values.contains_key("device"));
        assert!(!legacy.values.contains_key("token"));
    }
    #[test]
    fn live_recovery_evidence_is_bound_to_the_report_session_without_raw_content() {
        let launch = request();
        let mut report = Report::new(&launch);
        report.observe(
            parse_sample(b"SNROUTE_HEALTH controlSilenceMs=16000 endpoint=private token=123")
                .unwrap(),
        );
        let json = serde_json::to_value(report.performance_update()).unwrap();
        assert_eq!(json["sessionId"], launch.session_id.as_deref().unwrap());
        assert_eq!(
            json["performance"]["metrics"]["controlSilence"]["latestMs"],
            16000.0
        );
        assert_eq!(report.status, "delay-observed");
        assert!(!json.to_string().contains("private") && !json.to_string().contains("token"));
    }
    #[test]
    fn input_stage_reports_keep_queue_work_and_retry_counts_separate_without_content() {
        let mut report = Report::new(&request());
        report.observe(parse_sample(b"SNV2_INPUT_TIMING inputSendQueueAvgMs=2 inputRetries=3 coalescedMoves=100 key=4 text=123").unwrap());
        report.observe(parse_sample(b"SNV2_INPUT_HOST_TIMING hostQueueAvgMs=40 hostAvgWorkMs=6 hostQueueMaxMs=80 token=456").unwrap());
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["performance"]["metrics"]["inputSendQueue"]["latestMs"],
            2.0
        );
        assert_eq!(json["status"], "delay-observed");
        assert_eq!(
            json["performance"]["metrics"]["hostInputQueue"]["latestMs"],
            40.0
        );
        assert_eq!(json["performance"]["metrics"]["hostInput"]["latestMs"], 6.0);
        assert!(json["performance"]["metrics"]["inputAckRtt"].is_null());
        assert!(json["performance"]["metrics"]["inputRetries"].is_null());
        let serialized = json.to_string();
        assert!(
            !serialized.contains("\"key\"")
                && !serialized.contains("\"text\"")
                && !serialized.contains("\"token\"")
        );
    }
    #[test]
    fn common_performance_contract_keeps_rtt_meanings_and_missing_stages() {
        let mut launch = request();
        launch.wire_protocol = Some("snv2".into());
        let mut report = Report::new(&launch);
        report.observe(parse_sample(b"SNV2_CONTROL_TIMING appRttMs=12 token=123").unwrap());
        report.observe(parse_sample(b"SNV2_INPUT_TIMING avgRttMs=190 hostAvgWorkMs=2").unwrap());
        report.observe(
            parse_sample(
                b"SNV2_VIDEO_HEALTH decoded=10 decodeWorkAvgMs=3 decodeQueueAvgMs=20 screen=456",
            )
            .unwrap(),
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["schemaVersion"], 2);
        assert_eq!(
            json["performance"]["metrics"]["controlAppRtt"]["latestMs"],
            12.0
        );
        assert_eq!(
            json["performance"]["metrics"]["inputAckRtt"]["latestMs"],
            190.0
        );
        assert!(json["performance"]["metrics"]["wireRttEstimate"].is_null());
        assert!(
            report
                .performance_findings
                .contains(&"input-delivery-overhead")
        );
        assert!(report.performance_findings.contains(&"decode-queue-high"));
        assert!(!json.to_string().contains("token") && !json.to_string().contains("screen"));
        let legacy = parse_sample(b"SNINPUT_ACK rttMs=15 hostProcessMs=1 sequence=123").unwrap();
        report.observe(legacy);
        assert_eq!(
            report.performance.metrics["inputAckRtt"].source,
            "legacy-input"
        );
    }
    #[test]
    fn common_performance_observer_receives_initial_and_final_snapshots_without_gui()
    -> std::io::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("sanser-performance-{}", uuid::Uuid::new_v4()));
        let (notify, updates) = mpsc::channel();
        let sender = start(
            &directory,
            &request(),
            None,
            Some(Box::new(move |update| {
                notify.send(update.clone()).unwrap();
            })),
        )?;
        let initial = updates.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(!initial.ended && initial.performance.metrics.is_empty());
        sender
            .send(parse_sample(b"SNV2_CONTROL_TIMING appRttMs=200").unwrap())
            .unwrap();
        drop(sender);
        let final_update = loop {
            let update = updates.recv_timeout(Duration::from_secs(3)).unwrap();
            if update.ended {
                break update;
            }
        };
        assert_eq!(final_update.report_id, initial.report_id);
        assert_eq!(
            final_update.performance.metrics["controlAppRtt"].latest_ms,
            200.0
        );
        assert_eq!(final_update.findings, vec!["control-round-trip-high"]);
        // Notification occurs just before the final atomic file write.
        let deadline = Instant::now() + Duration::from_secs(3);
        while !recent(&directory)
            .first()
            .is_some_and(|r| r["ended"] == true)
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        fs::remove_dir_all(directory)
    }
    #[test]
    fn sharing_and_transport_health_retain_only_numeric_failure_evidence() {
        let sample=parse_sample(b"SNSHARE_PEER viewer=private-device ready=1 running=1 submitted=80 dispatched=1 replaced=79 referenceDrops=78 pauses=1 backoffMs=2000 videoState=paused videoStateCode=1 pauseReason=2 completedFrame=4 sentFrame=80 feedbackSerial=10 feedbackCongested=0 token=123").unwrap();
        assert_eq!(sample.values["replaced"], 79.0);
        assert_eq!(sample.values["videoStateCode"], 1.0);
        assert_eq!(sample.values["pauseReason"], 2.0);
        assert_eq!(sample.values["completedFrame"], 4.0);
        assert_eq!(sample.values["sentFrame"], 80.0);
        assert!(!sample.values.contains_key("videoState"));
        assert!(!sample.values.contains_key("viewer"));
        assert!(!sample.values.contains_key("token"));
        let sample=parse_sample(b"SNV2_TRANSPORT_HEALTH rawDatagrams=900 authRejected=0 videoDatagrams=890 completed=0 nackSent=4 stopReason=2 pending=0 endpoint=private-address").unwrap();
        assert_eq!(sample.values["completed"], 0.0);
        assert_eq!(sample.values["stopReason"], 2.0);
        assert!(!sample.values.contains_key("endpoint"));
        let sample=parse_sample(b"SNV2_VIDEO_HEALTH decoded=40 decoderOutputErrors=1 decoderOutputDrops=2 frameContent=123").unwrap();
        assert_eq!(sample.values["decoderOutputErrors"], 1.0);
        assert_eq!(sample.values["decoderOutputDrops"], 2.0);
        assert!(!sample.values.contains_key("frameContent"));
    }
    #[test]
    fn snv2_pipeline_timings_are_retained_without_input_or_packet_content() {
        let mut launch = request();
        launch.wire_protocol = Some("snv2".into());
        let mut report = Report::new(&launch);
        report.observe(parse_sample(b"SNV2_HOST_TIMING captured=600 submitted=59 completed=58 encodeQueueAvgMs=12 encodeAndPublishAvgMs=8 token=123").unwrap());
        report.observe(parse_sample(b"SNV2_VIDEO_HEALTH decoded=58 converted=57 rendered=55 decodeWorkAvgMs=2 conversionAvgMs=3 paintAvgMs=1 decodedToPaintAvgMs=60 content=456").unwrap());
        assert!(report.media_started_at_ms.is_some());
        assert_eq!(report.status, "delay-observed");
        assert_eq!(report.maxima["SNV2_VIDEO_HEALTH.decodedToPaintAvgMs"], 60.0);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["wireProtocol"], "snv2");
        assert_eq!(json["requestedWidth"], 1920);
        assert_eq!(json["requestedHeight"], 1200);
        assert_eq!(json["requestedCodec"], "h264");
        let text = json.to_string();
        assert!(!text.contains("never-store-this-token"));
        assert!(!text.contains("token") && !text.contains("content"));
    }
    fn request() -> LaunchEngineRequest {
        serde_json::from_value(serde_json::json!({
            "kind": "client", "sessionId": uuid::Uuid::new_v4().to_string(),
            "codec": "h264", "fps": 60, "bitrateKbps": 25000,
            "width": 1920, "height": 1200, "networkMode": "auto", "relay": true,
            "sessionToken": "never-store-this-token"
        }))
        .unwrap()
    }
    #[test]
    fn echo_report_retains_only_numeric_measurements_and_does_not_mark_video_started() {
        let mut request = request();
        request.relay = false;
        request.direct_attempt_id = Some("attempt".into());
        request.direct_probe = Some(sanser_p2p::ProbeStatistics {
            samples: 5,
            median_ms: 14.0,
            min_ms: 13.0,
            max_ms: 16.0,
            jitter_ms: 1.0,
            loss_percent: 0.0,
            score_ms: 16.0,
        });
        let mut report = Report::new(&request);
        report.observe(parse_sample(b"SNU1_ECHO rttMs=14 hostHoldMs=0.1 residualMs=13.9 sendCallMs=0.02 nonce=123 token=456").unwrap());
        let health = report.route_health.as_ref().unwrap();
        assert_eq!(health.echo_rtt_ms, Some(14.0));
        assert_eq!(health.live_wire_rtt_ms, None);
        assert!(report.media_started_at_ms.is_none());
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("nonce"));
        assert!(!json.contains("token"));
    }

    #[test]
    fn route_health_requires_media_warmup_and_sustained_increase() {
        let mut health = RouteHealth {
            attempt_id: "test".into(),
            probe: sanser_p2p::ProbeStatistics {
                samples: 5,
                median_ms: 13.0,
                min_ms: 11.0,
                max_ms: 16.0,
                jitter_ms: 1.0,
                loss_percent: 0.0,
                score_ms: 15.0,
            },
            live_wire_rtt_ms: None,
            echo_rtt_ms: None,
            echo_host_hold_ms: None,
            echo_updated_at_ms: None,
            status: "awaiting-media-warmup",
            consecutive_elevated: 0,
            last_sample_at_ms: None,
        };
        health.observe(200.0, 5_000, None);
        health.observe(200.0, 6_000, Some(5_000));
        assert_eq!(health.consecutive_elevated, 0);
        for time in [7_000, 8_000] {
            health.observe(200.0, time, Some(5_000));
        }
        assert_ne!(health.status, "elevated-after-media");
        health.observe(200.0, 9_000, Some(5_000));
        assert_eq!(health.status, "elevated-after-media");
        health.observe(15.0, 10_000, Some(5_000));
        assert_eq!(health.status, "no-sustained-rise");
        health.observe(200.0, 11_000, Some(5_000));
        health.observe(200.0, 20_000, Some(5_000));
        assert_eq!(health.consecutive_elevated, 1); // Stale samples do not form a streak.
        health.probe.samples = 1;
        health.observe(200.0, 21_000, Some(5_000));
        assert_eq!(health.status, "insufficient-probe-samples");
        health.probe.samples = 5;
        health.probe.median_ms = 183.0;
        health.observe(185.0, 22_000, Some(5_000));
        assert_eq!(health.status, "slow-at-nomination");
        assert_eq!(health.consecutive_elevated, 0);
        health.observe(15.0, 23_000, Some(5_000));
        assert_eq!(health.status, "no-sustained-rise");
    }

    #[test]
    fn udp_receive_queue_report_excludes_packet_contents() {
        let sample = parse_sample(b"SNU1_RX_QUEUE queuedDatagrams=4 highWaterDatagrams=1024 receiveDropped=20 receiveExpired=10 receiveOversized=0 receiveQueueAvgMs=2 receiveQueueMaxMs=8 token=123 payload=456").unwrap();
        assert_eq!(sample.values.len(), 7);
        assert_eq!(sample.values["receiveDropped"], 20.0);
        assert_eq!(sample.values["receiveQueueMaxMs"], 8.0);
    }

    #[test]
    fn direct_control_timing_keeps_queue_metrics_without_raw_timestamps_or_content() {
        let sample = parse_sample(b"SNCONTROL_TIMING appRttMs=180 socketRttMs=142 wireEstimateMs=12 macSendQueueMs=20 hostReceiveQueueMs=120 hostControlWorkMs=10 macReceiveQueueMs=18 t0=1000000 t2=900000000 token=123 key=65").unwrap();
        assert_eq!(sample.values.len(), 7);
        assert_eq!(sample.values["wireEstimateMs"], 12.0);
        assert_eq!(sample.values["hostReceiveQueueMs"], 120.0);
        assert!(parse_sample(b"SNINPUT_TIMING appRttMs=15 hostControlWorkMs=2").is_some());
    }

    #[test]
    fn report_records_confirmed_relay_transport_and_delivery_estimate() {
        let mut value = request();
        assert_eq!(Report::new(&value).relay_transport, None);
        value.relay_transport = Some("udp".into());
        let mut report = Report::new(&value);
        assert_eq!(report.relay_transport, Some("udp"));
        report.observe(
            parse_sample(b"SNV1_ADAPT deliveredMbps=16.0 estimatedDeliveryMbps=17.2 token=secret")
                .unwrap(),
        );
        assert_eq!(report.maxima.get("SNV1_ADAPT.deliveredMbps"), Some(&16.0));
        value.relay = false;
        assert_eq!(Report::new(&value).relay_transport, None);
    }

    #[test]
    fn startup_timeout_survives_late_overlay_samples_without_storing_peer_or_token() {
        let mut report = Report::new(&request());
        report.observe(parse_sample(b"SNU1_STARTUP rawDatagrams=30 unexpectedPeer=30 controlDatagrams=0 videoDatagrams=0 completed=0 decoded=0 startupTimeout=1 peer=192.0.2.1 token=123").unwrap());
        report.observe(parse_sample(b"SNV1_RENDER_STATS rendered=0 renderGpuMs=0.5").unwrap());
        assert_eq!(report.status, "startup-timeout");
        assert_eq!(
            report.maxima.get("SNU1_STARTUP.unexpectedPeer"),
            Some(&30.0)
        );
        assert_eq!(report.maxima.get("SNV1_RENDER_STATS.rendered"), Some(&0.0));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("192.0.2.1") && !json.contains("token"));
    }
    #[test]
    fn overlay_or_control_traffic_does_not_mark_video_startup_complete() {
        let mut report = Report::new(&request());
        report.observe(
            parse_sample(
                b"SNU1_STARTUP rawDatagrams=10 controlDatagrams=10 decoded=0 startupTimeout=0",
            )
            .unwrap(),
        );
        report.observe(parse_sample(b"SNV1_RENDER_STATS rendered=0 renderGpuMs=0.5").unwrap());
        assert_eq!(report.status, "awaiting-video");
        report.observe(
            parse_sample(b"SNU1_STARTUP rawDatagrams=100 completed=1 decoded=1 startupTimeout=0")
                .unwrap(),
        );
        assert_eq!(report.status, "samples-collected");
        // stdout from another worker may arrive out of order.
        report.observe(
            parse_sample(b"SNU1_STARTUP rawDatagrams=10 decoded=0 startupTimeout=0").unwrap(),
        );
        assert_eq!(report.status, "samples-collected");
    }
    #[test]
    fn detects_delay_and_retains_bounded_samples_without_claiming_missing_data_is_healthy() {
        let mut report = Report::new(&request());
        assert_eq!(report.status, "awaiting-measurements");
        for _ in 0..200 {
            report.observe(parse_sample(b"SNINPUT_RTT rttMs=150").unwrap());
        }
        assert_eq!(report.delay_sample_count, 200);
        assert_eq!(report.samples.len(), MAX_SAMPLES);
        assert_eq!(report.status, "delay-observed");
        report.observe(parse_sample(b"SNINPUT_RTT rttMs=5").unwrap());
        assert_eq!(report.maxima.get("SNINPUT_RTT.rttMs"), Some(&150.0));
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("never-store-this-token")
        );
    }
    #[test]
    fn ultra_report_tracks_frame_queue_budget_without_inventing_missing_samples() {
        let mut value = request();
        value.ultra_low_latency = true;
        let mut report = Report::new(&value);
        assert_eq!(report.status, "awaiting-measurements");
        report.observe(
            parse_sample(b"SNV1_RENDER_STATS avgRenderAgeMs=9.1 renderGpuMs=1.0").unwrap(),
        );
        assert_eq!(report.delay_sample_count, 1);
        assert_eq!(report.status, "delay-observed");
        assert_eq!(
            report.maxima.get("SNV1_RENDER_STATS.avgRenderAgeMs"),
            Some(&9.1)
        );
        assert!(!report.maxima.contains_key("SNINPUT_RTT.rttMs"));
    }

    #[test]
    fn report_delivers_route_health_without_a_gui_runtime() -> std::io::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("sanser-route-health-{}", uuid::Uuid::new_v4()));
        let mut launch = request();
        launch.relay = false;
        launch.direct_attempt_id = Some("current-attempt".into());
        launch.direct_probe = Some(sanser_p2p::ProbeStatistics {
            samples: 5,
            median_ms: 13.0,
            min_ms: 11.0,
            max_ms: 16.0,
            jitter_ms: 1.0,
            loss_percent: 0.0,
            score_ms: 15.0,
        });
        let (notification, received) = mpsc::channel();
        let sender = start(
            &directory,
            &launch,
            Some(Box::new(move |health| {
                notification.send(health.clone()).unwrap();
            })),
            None,
        )?;
        sender
            .send(parse_sample(b"SNCONTROL_TIMING wireEstimateMs=200").unwrap())
            .unwrap();
        let health = received.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(health.attempt_id, "current-attempt");
        assert_eq!(health.live_wire_rtt_ms, Some(200.0));
        assert_eq!(health.status, "awaiting-media-warmup");
        drop(sender);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !recent(&directory)
            .first()
            .is_some_and(|report| report["ended"] == true)
        {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            recent(&directory)[0]["routeHealth"]["probe"]["medianMs"],
            13.0
        );
        fs::remove_dir_all(directory)
    }

    #[test]
    fn reports_survive_end_of_session_and_export_without_credentials() -> std::io::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("sanser-reports-{}", uuid::Uuid::new_v4()));
        let sender = start(&directory, &request(), None, None)?;
        sender
            .send(parse_sample(b"SNV1_STAGE_PROFILE encodeAvgMs=75 token=123").unwrap())
            .unwrap();
        drop(sender);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let reports = recent(&directory);
            if reports
                .first()
                .is_some_and(|report| report["ended"] == true)
            {
                let report = &reports[0];
                assert_eq!(report["route"], "relay");
                assert_eq!(report["status"], "delay-observed");
                assert_eq!(report["delaySampleCount"], 1);
                assert!(!report.to_string().contains("token"));
                break;
            }
            assert!(Instant::now() < deadline, "report was not saved at EOF");
            thread::sleep(Duration::from_millis(10));
        }
        fs::remove_dir_all(directory)
    }
    #[test]
    fn idle_capture_wait_is_recorded_without_being_classified_as_slow_capture() {
        let mut report = Report::new(&request());
        report.observe(
            parse_sample(b"SNV1_HOST_TIMING captureAvgMs=2 captureWaitAvgMs=100").unwrap(),
        );
        assert_eq!(report.delay_sample_count, 0);
        assert_eq!(report.maxima["SNV1_HOST_TIMING.captureWaitAvgMs"], 100.0);
        report
            .observe(parse_sample(b"SNV1_HOST_TIMING captureAvgMs=70 captureWaitAvgMs=0").unwrap());
        assert_eq!(report.delay_sample_count, 1);
    }
    #[test]
    fn accepts_only_known_numeric_metrics_without_secrets_or_clock_skew() {
        let timing = parse_sample(b"SNV1_HOST_TIMING sendAvgMs=45 udpPacedAvgMs=4 udpWaitOvershootAvgMs=1.2 udpSocketAvgMs=38 captureAvgMs=-1 captureWaitAvgMs=40 token=123").unwrap();
        assert_eq!(timing.values.len(), 5);
        assert_eq!(timing.values["captureWaitAvgMs"], 40.0);
        assert_eq!(timing.values["sendAvgMs"], 45.0);
        assert_eq!(timing.values["udpSocketAvgMs"], 38.0);
        let sample =
            parse_sample(b"SNINPUT_ACKED rttMs=150 token=123 x=45 key=65 avgAgeMs=3000 pending=2");
        assert!(sample.as_ref().is_some_and(
            |sample| sample.values.len() == 2 && sample.values.get("rttMs") == Some(&150.0)
        ));
        assert!(parse_sample(b"Bearer secret-password").is_none());
        assert!(parse_sample(b"SNINPUT_RTT rttMs=NaN").is_none());
        assert!(parse_sample(b"SNINPUT_RTT rttMs=-1").is_none());
    }
    #[test]
    fn retention_removes_old_reports_only() -> std::io::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("sanser-reports-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("keep.txt"), "keep")?;
        for index in 0..25 {
            fs::write(directory.join(format!("connection-{index:03}.json")), "{}")?;
        }
        prune(&directory)?;
        assert_eq!(recent(&directory).len(), MAX_REPORTS);
        assert!(directory.join("keep.txt").exists());
        assert!(!directory.join("connection-000.json").exists());
        fs::remove_dir_all(directory)
    }

    #[test]
    fn video_repair_diagnostics_preserve_numeric_counts_only() {
        let sample = parse_sample(b"SNU1_STATS incomplete=2 missingFragments=3 nackSent=4 nackRecovered=1 nackTimedOut=0 replayRejected=2 sessionToken=123").unwrap();
        assert_eq!(sample.values["missingFragments"], 3.0);
        assert_eq!(sample.values["nackRecovered"], 1.0);
        assert!(!sample.values.contains_key("sessionToken"));
        let capture = parse_sample(b"SNV1_HOST_TIMING capturePaused=1").unwrap();
        assert_eq!(capture.values["capturePaused"], 1.0);
    }
}
