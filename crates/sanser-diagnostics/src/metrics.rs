use sanser_core::{TransportKind, VideoCodec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LatencyBreakdown {
    pub capture_us: u64,
    pub convert_us: u64,
    pub encode_us: u64,
    pub send_queue_us: u64,
    pub network_us: u64,
    pub reassembly_us: u64,
    pub decode_us: u64,
    pub render_us: u64,
    pub input_capture_us: u64,
    pub input_network_us: u64,
    pub input_injection_us: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceMetrics {
    /// CPU load in hundredths of one percent (`10_000 == 100%`).
    pub cpu_percent_x100: u32,
    /// GPU load in hundredths of one percent (`10_000 == 100%`).
    pub gpu_percent_x100: u32,
    pub resident_memory_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionMetrics {
    pub sampled_at_us: u64,
    pub connection_state: String,
    pub transport: Option<TransportKind>,
    pub codec: VideoCodec,
    pub fps_x100: u32,
    pub bitrate_bps: u64,
    pub rtt_us: u64,
    pub jitter_us: u64,
    /// Packet loss in hundredths of one percent (`10_000 == 100%`).
    pub packet_loss_percent_x100: u32,
    pub audio_buffer_us: u64,
    pub dropped_frames: u64,
    pub nack_count: u64,
    pub retransmission_count: u64,
    pub ice_candidate_type: Option<String>,
    pub turn_in_use: bool,
    pub latency: LatencyBreakdown,
    pub resources: ResourceMetrics,
}

impl Default for SessionMetrics {
    fn default() -> Self {
        Self {
            sampled_at_us: 0,
            connection_state: "idle".to_owned(),
            transport: None,
            codec: VideoCodec::Auto,
            fps_x100: 0,
            bitrate_bps: 0,
            rtt_us: 0,
            jitter_us: 0,
            packet_loss_percent_x100: 0,
            audio_buffer_us: 0,
            dropped_frames: 0,
            nack_count: 0,
            retransmission_count: 0,
            ice_candidate_type: None,
            turn_in_use: false,
            latency: LatencyBreakdown::default(),
            resources: ResourceMetrics::default(),
        }
    }
}
