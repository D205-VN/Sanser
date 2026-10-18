//! Numeric-only optional video protection telemetry, separate from millisecond
//! pipeline measurements. No native payload, key or challenge is retained.
use crate::{
    connection_report::{Sample, parse_sample},
    models::EngineKind,
};
use sanser_p2p::fec::Stats;
use std::sync::{Arc, Mutex, mpsc::SyncSender};
use tauri::{AppHandle, Emitter};
pub type ReportSink = Arc<Mutex<Option<SyncSender<Sample>>>>;
pub fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
pub fn sample(stats: Stats) -> Option<Sample> {
    parse_sample(format!("FEC_HEALTH fecNegotiated={} fecTargetPercent={} fecVideoBytes={} fecParityBytes={} fecSent={} fecRecovered={} fecExpired={} fecRejected={} fecBudgetSkipped={} fecReceiverLate={} fecCongestionPauses={}",u8::from(stats.negotiated),stats.overhead_percent,stats.video_bytes,stats.parity_bytes,stats.parity_packets,stats.recovered,stats.expired,stats.invalid,stats.budget_skipped,stats.receiver_late,stats.congestion_pauses).as_bytes())
}
pub fn observe(
    app: &AppHandle,
    kind: EngineKind,
    session: uuid::Uuid,
    transport: &str,
    started_at_ms: u64,
    stats: Stats,
    metrics: Option<&SyncSender<Sample>>,
) {
    if let Some(sender) = metrics
        && let Some(sample) = sample(stats)
    {
        let _ = sender.try_send(sample);
    }
    let _ = app.emit(
        "fec-health",
        serde_json::json!({"kind":kind,"sessionId":session,"transport":transport,"startedAtMs":started_at_ms,"stats":stats}),
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn fec_counters_remain_numeric_and_separate_from_latency_samples() {
        let record = sample(Stats {
            negotiated: true,
            overhead_percent: 12,
            recovered: 3,
            ..Stats::default()
        })
        .unwrap();
        let value = serde_json::to_value(&record).unwrap();
        let values = value["values"].as_object().unwrap();
        assert_eq!(values.len(), 11);
        assert_eq!(values["fecRecovered"], serde_json::json!(3.0));
        assert_eq!(values["fecTargetPercent"], serde_json::json!(12.0));
        let serialized = serde_json::to_string(&record).unwrap();
        assert!(
            !serialized.contains("key")
                && !serialized.contains("payload")
                && !serialized.contains("endpoint")
        );
    }
}
