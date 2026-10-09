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
            (value.is_finite() && (0.0..=1e12).contains(&value)).then(|| (key.to_owned(), value))
        })
        .collect();
    (!values.is_empty()).then(|| Sample {
        at_ms: now_ms(),
        source: source.into(),
        values,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u8,
    version: &'static str,
    session_id: Option<String>,
    machine: String,
    platform: &'static str,
    kind: crate::models::EngineKind,
    route: &'static str,
    relay_transport: Option<&'static str>,
    direct_check: &'static str,
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
    samples: VecDeque<Sample>,
}
impl Report {
    fn new(request: &LaunchEngineRequest) -> Self {
        Self {
            schema_version: 1,
            version: env!("CARGO_PKG_VERSION"),
            session_id: request
                .session_id
                .as_deref()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                .map(|id| id.to_string()),
            machine: crate::device_identity::computer_name(),
            platform: std::env::consts::OS,
            kind: request.kind,
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
            samples: VecDeque::new(),
        }
    }
    fn observe(&mut self, sample: Sample) {
        self.sample_count += 1;
        // RTT and local processing durations use one machine's monotonic clock.
        // Cross-machine wall-clock frame ages are deliberately excluded.
        let delayed = sample.values.iter().any(|(key, value)| match key.as_str() {
            "rttMs" | "avgRttMs" | "maxRttMs" | "appRttMs" | "socketRttMs" => {
                *value >= if self.ultra_low_latency { 15.0 } else { 100.0 }
            }
            "avgRenderAgeMs" | "renderGpuMs" => {
                *value >= if self.ultra_low_latency { 4.0 } else { 50.0 }
            }
            "decodeAvgMs" | "encodeAvgMs" | "captureAvgMs" | "sendAvgMs" | "hostAvgWorkMs" => {
                *value >= 50.0
            }
            "avgPresentLateMs" | "maxPresentLateMs" | "latencyLateMaxMs" => *value >= 80.0,
            "latencyDropped" | "keyframeWaitDropped" => *value > 0.0,
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

pub(crate) fn start(
    directory: &Path,
    request: &LaunchEngineRequest,
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
    let (sender, receiver) = mpsc::sync_channel(128);
    thread::Builder::new()
        .name("sanser-connection-report".into())
        .spawn(move || {
            let mut flushed = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_secs(5)) {
                    Ok(sample) => report.observe(sample),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        report.ended = true;
                        report.updated_at_ms = now_ms();
                        let _ = save(&path, &report);
                        break;
                    }
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
mod tests {
    use super::*;
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
    fn reports_survive_end_of_session_and_export_without_credentials() -> std::io::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("sanser-reports-{}", uuid::Uuid::new_v4()));
        let sender = start(&directory, &request())?;
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
}
