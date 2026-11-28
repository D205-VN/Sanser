//! Live measurements for the stream advisor. Memory only: a probe lives for
//! less than a minute and nothing here reaches the database or the logs.
//!
//! The viewer opens a probe for one of its hosts. The host notices it on its
//! next poll, measures its round trip and upload speed to this server and
//! reports its screen and CPU. Meanwhile the viewer measures its download
//! speed. The session relay runs on this server too, so the two halves add up
//! to the relay path; a direct path is usually at least as good.

use std::{
    collections::HashMap,
    sync::LazyLock,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Long enough for a host polling every 2 s to answer, short enough that a
/// forgotten probe disappears quickly.
pub const PROBE_TTL: Duration = Duration::from_secs(45);
/// One upload or download request carries this much.
pub const CHUNK_BYTES: usize = 256 * 1024;
/// Per probe and direction: at most 8 MiB, enough for about 3 s at 20 Mb/s.
pub const MAX_CHUNKS: u32 = 32;
const MAX_PROBES: usize = 1_000;

/// Incompressible bytes, so a proxy cannot shrink the download test.
pub static NOISE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..CHUNK_BYTES)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
});

/// What the host measured and knows about itself. Unknown values stay empty.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostMeasurement {
    /// The host is in a session; it did not measure so the session keeps its bandwidth.
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub server_ms: Option<u32>,
    #[serde(default)]
    pub upload_mbps: Option<f64>,
    #[serde(default)]
    pub display_width: Option<u32>,
    #[serde(default)]
    pub display_height: Option<u32>,
    #[serde(default)]
    pub refresh_hz: Option<u32>,
    #[serde(default)]
    pub cpu_cores: Option<u32>,
    #[serde(default)]
    pub cpu_model: Option<String>,
    #[serde(default)]
    pub memory_gb: Option<u32>,
    #[serde(default)]
    pub gpu: Option<String>,
}

impl HostMeasurement {
    /// Drops values no real computer reports, so the rules never see them.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        let within =
            |value: Option<u32>, low: u32, high: u32| value.filter(|v| (low..=high).contains(v));
        self.server_ms = within(self.server_ms, 1, 10_000);
        self.upload_mbps = self
            .upload_mbps
            .filter(|mbps| mbps.is_finite() && (0.05..=10_000.0).contains(mbps));
        self.display_width = within(self.display_width, 320, 16_384);
        self.display_height = within(self.display_height, 240, 16_384);
        self.refresh_hz = within(self.refresh_hz, 20, 500);
        self.cpu_cores = within(self.cpu_cores, 1, 1_024);
        self.memory_gb = within(self.memory_gb, 1, 4_096);
        self.gpu = clean_label(self.gpu);
        self.cpu_model = clean_label(self.cpu_model);
        self
    }
}

fn clean_label(value: Option<String>) -> Option<String> {
    value
        .map(|model| {
            model
                .chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|model| !model.is_empty())
}

struct Probe {
    user_id: String,
    host_device_id: String,
    created: Instant,
    claimed: bool,
    uploaded: u32,
    downloaded: u32,
    result: Option<HostMeasurement>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Transfer {
    Allowed,
    Exhausted,
    Missing,
}

#[derive(Default)]
pub struct ProbeStore {
    probes: Mutex<HashMap<String, Probe>>,
}

impl ProbeStore {
    /// Opens a probe; `None` when the store is full of live probes.
    pub async fn create(&self, user_id: &str, host_device_id: &str) -> Option<String> {
        let now = Instant::now();
        let mut probes = self.probes.lock().await;
        probes.retain(|_, probe| now.duration_since(probe.created) < PROBE_TTL);
        if probes.len() >= MAX_PROBES {
            return None;
        }
        let id = Uuid::new_v4().to_string();
        probes.insert(
            id.clone(),
            Probe {
                user_id: user_id.to_owned(),
                host_device_id: host_device_id.to_owned(),
                created: now,
                claimed: false,
                uploaded: 0,
                downloaded: 0,
                result: None,
            },
        );
        Some(id)
    }

    /// Probes waiting for this host. Each is handed out once.
    pub async fn claim(&self, user_id: &str, host_device_id: &str) -> Vec<String> {
        let now = Instant::now();
        let mut probes = self.probes.lock().await;
        probes
            .iter_mut()
            .filter(|(_, probe)| {
                !probe.claimed
                    && probe.user_id == user_id
                    && probe.host_device_id == host_device_id
                    && now.duration_since(probe.created) < PROBE_TTL
            })
            .map(|(id, probe)| {
                probe.claimed = true;
                id.clone()
            })
            .collect()
    }

    /// Counts one chunk in either direction against the probe's budget.
    pub async fn transfer(&self, id: &str, user_id: &str, upload: bool) -> Transfer {
        let now = Instant::now();
        let mut probes = self.probes.lock().await;
        let Some(probe) = probes.get_mut(id).filter(|probe| {
            probe.user_id == user_id && now.duration_since(probe.created) < PROBE_TTL
        }) else {
            return Transfer::Missing;
        };
        let used = if upload {
            &mut probe.uploaded
        } else {
            &mut probe.downloaded
        };
        if *used >= MAX_CHUNKS {
            return Transfer::Exhausted;
        }
        *used += 1;
        Transfer::Allowed
    }

    /// Stores the host's answer; `false` when the probe is gone.
    pub async fn finish(&self, id: &str, user_id: &str, result: HostMeasurement) -> bool {
        let now = Instant::now();
        let mut probes = self.probes.lock().await;
        match probes.get_mut(id).filter(|probe| {
            probe.user_id == user_id && now.duration_since(probe.created) < PROBE_TTL
        }) {
            Some(probe) => {
                probe.result = Some(result.sanitized());
                true
            }
            None => false,
        }
    }

    /// `None` when the probe is gone; otherwise the host's answer so far.
    pub async fn result(&self, id: &str, user_id: &str) -> Option<Option<HostMeasurement>> {
        let now = Instant::now();
        let probes = self.probes.lock().await;
        probes
            .get(id)
            .filter(|probe| {
                probe.user_id == user_id && now.duration_since(probe.created) < PROBE_TTL
            })
            .map(|probe| probe.result.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_probe_is_claimed_once_by_its_own_host_and_budgets_transfers() {
        let store = ProbeStore::default();
        let id = store
            .create("user", "host")
            .await
            .expect("room for a probe");
        assert!(store.claim("other", "host").await.is_empty());
        assert!(store.claim("user", "elsewhere").await.is_empty());
        assert_eq!(store.claim("user", "host").await, vec![id.clone()]);
        assert!(store.claim("user", "host").await.is_empty());

        assert_eq!(store.transfer(&id, "other", true).await, Transfer::Missing);
        for _ in 0..MAX_CHUNKS {
            assert_eq!(store.transfer(&id, "user", true).await, Transfer::Allowed);
        }
        assert_eq!(store.transfer(&id, "user", true).await, Transfer::Exhausted);
        assert_eq!(store.transfer(&id, "user", false).await, Transfer::Allowed);

        assert_eq!(store.result(&id, "user").await, Some(None));
        let answer = HostMeasurement {
            upload_mbps: Some(42.5),
            refresh_hz: Some(9_000),
            cpu_model: Some("  Apple M2\u{7}  ".into()),
            ..HostMeasurement::default()
        };
        assert!(store.finish(&id, "user", answer).await);
        let stored = store.result(&id, "user").await.flatten().expect("answer");
        assert_eq!(stored.upload_mbps, Some(42.5));
        assert_eq!(stored.refresh_hz, None);
        assert_eq!(stored.cpu_model.as_deref(), Some("Apple M2"));
        assert_eq!(store.result(&id, "other").await, None);
    }

    #[test]
    fn noise_does_not_repeat_a_short_pattern() {
        assert_eq!(NOISE.len(), CHUNK_BYTES);
        let distinct = NOISE.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(distinct > 200);
        assert_ne!(NOISE[..1024], NOISE[1024..2048]);
    }
}
