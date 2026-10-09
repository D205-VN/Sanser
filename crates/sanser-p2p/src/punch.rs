//! UDP hole punching orchestration.

use crate::connectivity::{
    CandidatePair, PROBE_COMMIT_BIT, PROBE_FINAL_BIT, PROBE_NOMINATION_BIT, PROBE_RESPONSE_BIT,
    PUNCH_SCHEDULE, PairState, ProbeFields, build_probe_packet, compute_pair_hash,
    parse_probe_packet,
};
use crate::error::P2pError;
use serde::Serialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

/// State of a punch attempt against one candidate pair.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PunchState {
    #[default]
    Idle,
    /// Waiting for synchronized start time.
    WaitingForPeer,
    /// Actively sending probes.
    Punching,
    /// Received a valid probe response.
    Succeeded,
    /// All scheduled probes exhausted without response.
    Exhausted,
    /// Cancelled because a better pair was found.
    Cancelled,
}

/// Tracks a single hole-punch sequence for one candidate pair.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PunchAttempt {
    pub pair_id: String,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    pub state: PunchState,
    pub probes_sent: u32,
    pub probes_received: u32,
    /// Measured RTT if a probe-ack round trip completed.
    pub rtt_ms: Option<f64>,
}

impl PunchAttempt {
    #[must_use]
    pub fn new(pair_id: String, local: SocketAddr, remote: SocketAddr) -> Self {
        Self {
            pair_id,
            local,
            remote,
            state: PunchState::Idle,
            probes_sent: 0,
            probes_received: 0,
            rtt_ms: None,
        }
    }
}

/// Locally measured probe statistics. No peer clock subtraction is used.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeStatistics {
    pub samples: usize,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    /// Median absolute deviation, robust to a single scheduling outlier.
    pub jitter_ms: f64,
    /// Only post-verification probes old enough to have timed out count as lost.
    pub loss_percent: f64,
    pub score_ms: f64,
}

#[derive(Clone, Debug)]
pub struct ConnectivitySelection {
    pub pair: CandidatePair,
    pub probe: Option<ProbeStatistics>,
    pub reason: &'static str,
}

#[derive(Default)]
struct ProbeMeasurements {
    pending: HashMap<u32, (Instant, u64, bool)>,
    rtts: Vec<f64>,
    verified_at: Option<Instant>,
    last_sent: Option<Instant>,
    measurement_sent: usize,
    measurement_received: usize,
    measurement_send_failures: usize,
}

impl ProbeMeasurements {
    fn response(&mut self, transaction: u32, timestamp: u64, now: Instant) -> Option<f64> {
        let &(sent, expected_timestamp, measured) = self.pending.get(&transaction)?;
        if timestamp != expected_timestamp {
            return None;
        }
        self.pending.remove(&transaction); // A duplicate cannot become another sample.
        let rtt = now.duration_since(sent).as_secs_f64() * 1000.0;
        if measured {
            self.measurement_received += 1;
        }
        // Keep one initial proof and four fresh probes. Late initial-burst
        // responses (possibly queued during peer startup) cannot pollute P50.
        if (self.rtts.is_empty() || measured) && self.rtts.len() < 5 {
            self.rtts.push(rtt);
        }
        Some(rtt)
    }

    fn statistics(&self, now: Instant) -> Option<ProbeStatistics> {
        let mut sorted = self.rtts.clone();
        if sorted.is_empty() {
            return None;
        }
        sorted.sort_by(f64::total_cmp);
        let median =
            |values: &[f64]| (values[(values.len() - 1) / 2] + values[values.len() / 2]) / 2.0;
        let median_ms = median(&sorted);
        let mut deviations: Vec<_> = sorted.iter().map(|rtt| (rtt - median_ms).abs()).collect();
        deviations.sort_by(f64::total_cmp);
        let jitter_ms = median(&deviations);
        let expiry_ms = (sorted.last().unwrap() * 2.0).max(100.0);
        let lost = self
            .pending
            .values()
            .filter(|(sent, _, measured)| {
                *measured && now.duration_since(*sent).as_secs_f64() * 1000.0 >= expiry_ms
            })
            .count();
        let lost = lost + self.measurement_send_failures;
        let completed = self.measurement_received + lost;
        let loss = if completed == 0 {
            0.0
        } else {
            lost as f64 / completed as f64
        };
        Some(ProbeStatistics {
            samples: sorted.len(),
            median_ms,
            min_ms: sorted[0],
            max_ms: *sorted.last().unwrap(),
            jitter_ms,
            loss_percent: loss * 100.0,
            score_ms: median_ms + 2.0 * jitter_ms + loss * median_ms.max(50.0),
        })
    }
}

fn best_pair(
    pairs: &[CandidatePair],
    measurements: &[ProbeMeasurements],
    now: Instant,
) -> Option<usize> {
    pairs
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.state == PairState::Succeeded)
        .filter_map(|(index, _)| {
            measurements[index]
                .statistics(now)
                .map(|stats| (index, stats))
        })
        .min_by(|(a, sa), (b, sb)| {
            // Prefer repeat-verified paths; retain a sparse-sample fallback when
            // the remaining negotiation budget cannot provide three responses.
            (sa.samples < 3)
                .cmp(&(sb.samples < 3))
                .then_with(|| sa.score_ms.total_cmp(&sb.score_ms))
                // Candidate priority carries interface preference. No invented
                // interface cost: one wildcard socket does not prove a NIC route.
                .then_with(|| pairs[*b].priority.cmp(&pairs[*a].priority))
                .then_with(|| pairs[*a].pair_id.cmp(&pairs[*b].pair_id))
        })
        .map(|(index, _)| index)
}

fn selected_result(
    pair: &CandidatePair,
    probe: Option<ProbeStatistics>,
    controlling: bool,
) -> ConnectivitySelection {
    let reason = if !controlling {
        "peer nominated this pair; probe statistics measured locally"
    } else if probe.as_ref().is_none_or(|stats| stats.samples < 3) {
        "best verified path within time budget; fewer than three RTT samples"
    } else {
        "lowest measured latency/loss/jitter score among repeat-verified paths; priority breaks ties"
    };
    ConnectivitySelection {
        pair: pair.clone(),
        probe,
        reason,
    }
}

/// Compatibility entry point for callers that only need the nominated pair.
pub async fn check_connectivity(
    socket: &UdpSocket,
    pairs: &mut [CandidatePair],
    session_id: uuid::Uuid,
    local_device_hash: u64,
    peer_device_hash: u64,
    hmac_key: &[u8],
    controlling: bool,
    timeout: Duration,
) -> Result<CandidatePair, P2pError> {
    check_connectivity_measured(
        socket,
        pairs,
        session_id,
        local_device_hash,
        peer_device_hash,
        hmac_key,
        controlling,
        timeout,
    )
    .await
    .map(|selection| selection.pair)
}

/// Runs the connectivity check and hole punching for a list of candidate pairs.
///
/// Collects repeated RTT samples for 500 ms after first success, then nominates
/// the best verified path. The controlled peer follows that same nomination.
///
/// # Errors
///
/// Returns a [`P2pError::NoDirectRoute`] if no candidate pair establishes a connection
/// within the timeout period.
pub async fn check_connectivity_measured(
    socket: &UdpSocket,
    pairs: &mut [CandidatePair],
    session_id: uuid::Uuid,
    local_device_hash: u64,
    peer_device_hash: u64,
    hmac_key: &[u8],
    controlling: bool,
    timeout: Duration,
) -> Result<ConnectivitySelection, P2pError> {
    const NOMINATION_TRANSACTION_ID: u32 = u32::MAX;
    const NOMINATION_RETRY_INTERVAL: Duration = Duration::from_millis(50);
    const CONTROLLED_LINGER: Duration = Duration::from_millis(300);
    const PROBE_RETRY_INTERVAL_MS: u64 = 500;
    // Sort pairs by priority descending (highest priority first)
    pairs.sort_by(|a, b| b.priority.cmp(&a.priority));

    // Map each pair to their identifier hashes for fast lookups
    let mut pair_map = HashMap::new();
    // STUN describes the endpoint observed by the STUN server, but an
    // endpoint-dependent NAT can allocate a different source port when this
    // socket talks to the peer. Learn that peer-reflexive endpoint only after
    // the probe HMAC, session and device identity have all been verified, then
    // pin it for the remainder of this connectivity check.
    let mut observed_remote = HashMap::new();
    let mut peer_verified = HashMap::new(); // Tracks if remote -> local has succeeded
    let mut local_verified = HashMap::new(); // Tracks if local -> remote (ACK) has succeeded
    let mut attempts = Vec::new();
    let mut measurements: Vec<ProbeMeasurements> = (0..pairs.len())
        .map(|_| ProbeMeasurements::default())
        .collect();
    let mut selection_deadline: Option<Instant> = None;
    let mut selected_probe = None;

    for (idx, pair) in pairs.iter().enumerate() {
        let hash = compute_pair_hash(&pair.pair_id);
        pair_map.insert(hash, idx);
        attempts.push(PunchAttempt::new(
            pair.pair_id.clone(),
            pair.local,
            pair.remote,
        ));
    }

    let start_time = std::time::Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    let mut nominated_pair: Option<usize> = None;
    let mut nomination_acknowledged = false;
    let mut commit_acknowledged = false;
    let mut controlled_nomination: Option<usize> = None;
    let mut controlled_finalized: Option<(usize, std::time::Instant)> = None;
    let mut last_nomination_sent = None;

    let mut buf = [0u8; 1024];

    loop {
        let elapsed = start_time.elapsed();
        if elapsed >= timeout {
            break;
        }

        tokio::select! {
            _ = interval.tick() => {
                if let Some((pair_idx, finalized_at)) = controlled_finalized
                    && finalized_at.elapsed() >= CONTROLLED_LINGER
                {
                    let pair = &mut pairs[pair_idx];
                    pair.state = PairState::Nominated;
                    attempts[pair_idx].state = PunchState::Succeeded;
                    return Ok(selected_result(pair,
                        selected_probe.or_else(|| measurements[pair_idx].statistics(Instant::now())), controlling));
                }
                if controlling && nominated_pair.is_none()
                    && selection_deadline.is_some_and(|deadline| Instant::now() >= deadline)
                {
                    if let Some(index) = best_pair(pairs, &measurements, Instant::now()) {
                        selected_probe = measurements[index].statistics(Instant::now());
                        pairs[index].state = PairState::Nominated;
                        nominated_pair = Some(index);
                    }
                }
                if let Some(pair_idx) = nominated_pair {
                    let now = std::time::Instant::now();
                    if last_nomination_sent.is_none_or(|last| now.duration_since(last) >= NOMINATION_RETRY_INTERVAL) {
                        let pair = &pairs[pair_idx];
                        let fields = ProbeFields {
                            session_id,
                            sender_device_hash: local_device_hash,
                            pair_hash: compute_pair_hash(&pair.pair_id),
                            transaction_id: NOMINATION_TRANSACTION_ID,
                            timestamp: start_time.elapsed().as_millis() as u64,
                            nonce: PROBE_NOMINATION_BIT
                                | if nomination_acknowledged {
                                    PROBE_COMMIT_BIT
                                } else {
                                    0
                                }
                                | if commit_acknowledged { PROBE_FINAL_BIT } else { 0 },
                        };
                        if let Ok(packet) = build_probe_packet(&fields, hmac_key) {
                            let _ = socket.send_to(&packet, pair.remote).await;
                        }
                        last_nomination_sent = Some(now);
                    }
                } else {
                    // Send ordinary checks according to the punch schedule.
                    let elapsed_ms = start_time.elapsed().as_millis() as u64;
                    for (idx, pair) in pairs.iter_mut().enumerate() {
                        if pair.state == PairState::Failed {
                            continue;
                        }

                        let attempt = &mut attempts[idx];
                        let next_probe_idx = attempt.probes_sent as usize;

                        // The initial burst is not the connectivity deadline. A
                        // peer may still be gathering candidates when it arrives.
                        // Continue bounded checks throughout the caller's budget.
                        {
                            let scheduled_delay = PUNCH_SCHEDULE.get(next_probe_idx).map_or_else(
                                || PUNCH_SCHEDULE.last().expect("nonempty punch schedule").delay_ms
                                    + (next_probe_idx - PUNCH_SCHEDULE.len() + 1) as u64 * PROBE_RETRY_INTERVAL_MS,
                                |probe| probe.delay_ms,
                            );
                            let measurement = &mut measurements[idx];
                            let verified = measurement.verified_at.is_some();
                            let due = if verified {
                                measurement.measurement_sent < 4
                                    && measurement.last_sent.is_none_or(|last| last.elapsed() >= Duration::from_millis(60))
                            } else { elapsed_ms >= scheduled_delay };
                            if due {
                                attempt.state = PunchState::Punching;
                                let fields = ProbeFields {
                                    session_id,
                                    sender_device_hash: local_device_hash,
                                    pair_hash: compute_pair_hash(&pair.pair_id),
                                    transaction_id: attempt.probes_sent,
                                    timestamp: start_time.elapsed().as_millis() as u64,
                                    nonce: attempt.probes_sent
                                        & !(PROBE_RESPONSE_BIT
                                            | PROBE_NOMINATION_BIT
                                            | PROBE_COMMIT_BIT
                                            | PROBE_FINAL_BIT),
                                };

                                if let Ok(packet) = build_probe_packet(&fields, hmac_key) {
                                    // Count attempts even on send failure to avoid
                                    // retrying an unreachable address every tick.
                                    attempt.probes_sent += 1;
                                    let sent = Instant::now();
                                    measurement.last_sent = Some(sent);
                                    if socket.send_to(&packet, pair.remote).await.is_ok() {
                                        if measurement.pending.len() < 128 {
                                            measurement.pending.insert(fields.transaction_id, (sent, fields.timestamp, verified));
                                        }
                                        if !verified { pair.state = PairState::InProgress; }
                                    } else if verified {
                                        measurement.measurement_send_failures += 1;
                                    }
                                    if verified { measurement.measurement_sent += 1; }
                                }
                            }
                        }
                    }
                }
            }
            recv_res = socket.recv_from(&mut buf) => {
                if let Ok((n, from)) = recv_res {
                    // Try to parse as an authenticated probe
                    if let Ok(parsed) = parse_probe_packet(&buf[..n], hmac_key) {
                        if parsed.session_id == session_id && parsed.sender_device_hash == peer_device_hash {
                            // Find the corresponding pair
                            if let Some(&pair_idx) = pair_map.get(&parsed.pair_hash) {
                                let pair = &mut pairs[pair_idx];
                                let attempt = &mut attempts[pair_idx];

                                // Validate ordinary responses before they can pin a peer endpoint.
                                if parsed.nonce & PROBE_RESPONSE_BIT != 0 && parsed.nonce & PROBE_NOMINATION_BIT == 0
                                    && !measurements[pair_idx].pending.get(&parsed.transaction_id)
                                        .is_some_and(|(_, timestamp, _)| *timestamp == parsed.timestamp)
                                { continue; }

                                if let Some(pinned) = observed_remote.get(&pair.pair_id) {
                                    if *pinned != from {
                                        continue;
                                    }
                                } else {
                                    observed_remote.insert(pair.pair_id.clone(), from);
                                    // Authenticated peer-reflexive discovery is
                                    // required when NAT rewrites the peer's port
                                    // differently from its STUN candidate.
                                    pair.remote = from;
                                }

                                attempt.probes_received += 1;
                                let is_response = parsed.nonce & PROBE_RESPONSE_BIT != 0;
                                let is_nomination = parsed.nonce & PROBE_NOMINATION_BIT != 0;
                                let is_commit = parsed.nonce & PROBE_COMMIT_BIT != 0;
                                let is_final = parsed.nonce & PROBE_FINAL_BIT != 0;

                                if is_nomination {
                                    if parsed.transaction_id != NOMINATION_TRANSACTION_ID {
                                        continue;
                                    }
                                    if is_response {
                                        if controlling && nominated_pair == Some(pair_idx) {
                                            if is_final && commit_acknowledged {
                                                pair.state = PairState::Nominated;
                                                attempt.state = PunchState::Succeeded;
                                                return Ok(selected_result(pair,
                                                    selected_probe.or_else(|| measurements[pair_idx].statistics(Instant::now())), controlling));
                                            }
                                            if is_commit && nomination_acknowledged {
                                                commit_acknowledged = true;
                                                last_nomination_sent = None;
                                            } else if !is_commit {
                                                nomination_acknowledged = true;
                                                last_nomination_sent = None;
                                            }
                                        }
                                    } else if !controlling {
                                        if (is_commit || is_final) && controlled_nomination != Some(pair_idx) {
                                            continue;
                                        }
                                        let fields = ProbeFields {
                                            session_id,
                                            sender_device_hash: local_device_hash,
                                            pair_hash: parsed.pair_hash,
                                            transaction_id: parsed.transaction_id,
                                            timestamp: parsed.timestamp,
                                            nonce: parsed.nonce | PROBE_RESPONSE_BIT,
                                        };
                                        if let Ok(packet) = build_probe_packet(&fields, hmac_key) {
                                            // A response burst plus yielding keeps
                                            // the selected socket alive long enough
                                            // for the other peer to commit too.
                                            for _ in 0..5 {
                                                let _ = socket.send_to(&packet, from).await;
                                                tokio::task::yield_now().await;
                                            }
                                        }
                                        if is_final {
                                            controlled_finalized.get_or_insert((pair_idx, std::time::Instant::now()));
                                        }
                                        if controlled_nomination != Some(pair_idx) {
                                            selected_probe = measurements[pair_idx].statistics(Instant::now());
                                        }
                                        controlled_nomination = Some(pair_idx);
                                    }
                                    continue;
                                }

                                if !is_response {
                                    peer_verified.insert(pair.pair_id.clone(), true);

                                    // Reply with an ACK/response probe (using odd nonce to indicate response)
                                    let fields = ProbeFields {
                                        session_id,
                                        sender_device_hash: local_device_hash,
                                        pair_hash: parsed.pair_hash,
                                        transaction_id: parsed.transaction_id,
                                        // Echo the sender's monotonic timestamp;
                                        // it is meaningful only in that sender's
                                        // clock domain and is used to measure RTT.
                                        timestamp: parsed.timestamp,
                                        nonce: parsed.nonce | PROBE_RESPONSE_BIT,
                                    };
                                    if let Ok(packet) = build_probe_packet(&fields, hmac_key) {
                                        let _ = socket.send_to(&packet, from).await;
                                    }
                                } else {
                                    let Some(rtt) = measurements[pair_idx].response(
                                        parsed.transaction_id, parsed.timestamp, Instant::now()) else { continue; };
                                    local_verified.insert(pair.pair_id.clone(), true);
                                    attempt.rtt_ms = Some(rtt);
                                }

                                // Only the controlling peer nominates. Both
                                // engines return the exact same pair after the
                                // controlled peer acknowledges that nomination.
                                if peer_verified.contains_key(&pair.pair_id)
                                    && local_verified.contains_key(&pair.pair_id)
                                {
                                    pair.state = PairState::Succeeded;
                                    attempt.state = PunchState::Succeeded;
                                    measurements[pair_idx].verified_at.get_or_insert_with(Instant::now);
                                    if controlling && selection_deadline.is_none() {
                                        // Leave time for nomination/commit/final and controlled linger.
                                        let remaining = timeout.saturating_sub(start_time.elapsed());
                                        let handshake_budget = Duration::from_millis(350)
                                            + Duration::from_secs_f64(attempt.rtt_ms.unwrap_or(0.0) * 3.0 / 1000.0);
                                        let window = Duration::from_millis(500).min(remaining.saturating_sub(handshake_budget));
                                        selection_deadline = Some(Instant::now() + window);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err(P2pError::NoDirectRoute)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(rtts: &[f64]) -> ProbeMeasurements {
        ProbeMeasurements {
            rtts: rtts.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn median_scoring_rejects_outliers_and_accounts_for_expired_loss() {
        let now = Instant::now();
        let fast = measured(&[12.0, 14.0, 13.0, 200.0, 12.0]);
        let stats = fast.statistics(now).unwrap();
        assert_eq!(stats.median_ms, 13.0);
        assert_eq!(stats.jitter_ms, 1.0);
        let mut lossy = measured(&[13.0, 13.0, 13.0]);
        lossy.measurement_received = 2;
        lossy
            .pending
            .insert(1, (now - Duration::from_millis(200), 1, true));
        lossy.pending.insert(2, (now, 2, true)); // Still in flight: not packet loss.
        lossy
            .pending
            .insert(3, (now - Duration::from_secs(2), 3, false)); // Startup miss.
        let stats = lossy.statistics(now).unwrap();
        assert!((stats.loss_percent - 100.0 / 3.0).abs() < 0.01);
        assert!(stats.score_ms > fast.statistics(now).unwrap().score_ms);
    }

    #[test]
    fn only_matching_unique_responses_contribute_rtt_samples() {
        let now = Instant::now();
        let mut samples = ProbeMeasurements::default();
        samples
            .pending
            .insert(7, (now - Duration::from_millis(14), 123, true));
        assert!(samples.response(8, 123, now).is_none());
        assert!(samples.response(7, 999, now).is_none());
        assert_eq!(samples.response(7, 123, now), Some(14.0));
        assert!(samples.response(7, 123, now).is_none());
        assert_eq!(samples.rtts.len(), 1);
    }

    #[tokio::test]
    async fn later_fast_candidate_beats_first_successful_slow_candidate() {
        use std::sync::Arc;
        let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let mut pairs: Vec<_> = [("slow", 100), ("fast", 10)]
            .into_iter()
            .map(|(id, priority)| CandidatePair {
                pair_id: id.into(),
                local: local.local_addr().unwrap(),
                remote: peer.local_addr().unwrap(),
                local_candidate_id: "local".into(),
                remote_candidate_id: id.into(),
                priority,
                state: PairState::Waiting,
            })
            .collect();
        let session = uuid::Uuid::new_v4();
        let started = Instant::now();
        let responder = tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            let mut buffer = [0; 1024];
            loop {
                let (n, from) = peer.recv_from(&mut buffer).await.unwrap();
                let fields = parse_probe_packet(&buffer[..n], b"selection-test").unwrap();
                if fields.nonce & PROBE_RESPONSE_BIT != 0 {
                    continue;
                }
                let fast = fields.pair_hash == compute_pair_hash("fast");
                if fast && started.elapsed() < Duration::from_millis(250) {
                    continue;
                }
                let nomination = fields.nonce & PROBE_NOMINATION_BIT != 0;
                let peer = Arc::clone(&peer);
                jobs.spawn(async move {
                    if !nomination {
                        // The peer's request proves reverse connectivity.
                        let reverse = ProbeFields {
                            sender_device_hash: 2,
                            ..fields.clone()
                        };
                        peer.send_to(
                            &build_probe_packet(&reverse, b"selection-test").unwrap(),
                            from,
                        )
                        .await
                        .unwrap();
                        tokio::time::sleep(Duration::from_millis(if fast { 14 } else { 200 }))
                            .await;
                    }
                    let response = ProbeFields {
                        sender_device_hash: 2,
                        nonce: fields.nonce | PROBE_RESPONSE_BIT,
                        ..fields
                    };
                    peer.send_to(
                        &build_probe_packet(&response, b"selection-test").unwrap(),
                        from,
                    )
                    .await
                    .unwrap();
                });
                while jobs.try_join_next().is_some() {}
            }
        });
        let result = check_connectivity_measured(
            &local,
            &mut pairs,
            session,
            1,
            2,
            b"selection-test",
            true,
            Duration::from_secs(3),
        )
        .await;
        responder.abort();
        let selection = result.unwrap();
        assert_eq!(selection.pair.pair_id, "fast");
        let probe = selection.probe.unwrap();
        assert!(probe.samples >= 3, "{probe:?}");
        assert!(probe.median_ms < 80.0, "{probe:?}");
        assert!(started.elapsed() >= Duration::from_millis(650));
    }

    #[tokio::test]
    async fn test_connectivity_check_timeout() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut pairs = vec![CandidatePair {
            pair_id: "pair-1".into(),
            local: socket.local_addr().unwrap(),
            remote: "127.0.0.1:55555".parse().unwrap(),
            local_candidate_id: "c-1".into(),
            remote_candidate_id: "c-2".into(),
            priority: 100,
            state: PairState::Waiting,
        }];
        let res = check_connectivity(
            &socket,
            &mut pairs,
            uuid::Uuid::new_v4(),
            1,
            2,
            b"key",
            true,
            Duration::from_millis(50),
        )
        .await;

        assert!(matches!(res, Err(P2pError::NoDirectRoute)));
    }

    #[tokio::test]
    async fn direct_check_recovers_when_peer_misses_the_initial_probe_burst() {
        let left = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let pair_id = crate::connectivity::make_pair_id("left", "right");
        let mut left_pairs = vec![CandidatePair {
            pair_id: pair_id.clone(),
            local: left.local_addr().unwrap(),
            remote: right.local_addr().unwrap(),
            local_candidate_id: "left".into(),
            remote_candidate_id: "right".into(),
            priority: 100,
            state: PairState::Waiting,
        }];
        let mut right_pairs = vec![CandidatePair {
            pair_id,
            local: right.local_addr().unwrap(),
            remote: left.local_addr().unwrap(),
            local_candidate_id: "right".into(),
            remote_candidate_id: "left".into(),
            priority: 100,
            state: PairState::Waiting,
        }];
        let session = uuid::Uuid::new_v4();
        let (a, b) = tokio::join!(
            check_connectivity(
                &left,
                &mut left_pairs,
                session,
                1,
                2,
                b"late-peer",
                true,
                Duration::from_secs(5)
            ),
            async {
                // Gathering/signaling may consume or miss probes before the
                // peer begins its authenticated connectivity check.
                tokio::time::sleep(Duration::from_millis(2_200)).await;
                let mut discard = [0; 1024];
                while right.try_recv_from(&mut discard).is_ok() {}
                check_connectivity(
                    &right,
                    &mut right_pairs,
                    session,
                    2,
                    1,
                    b"late-peer",
                    false,
                    Duration::from_secs(3),
                )
                .await
            }
        );
        assert!(
            a.is_ok(),
            "controlling peer prematurely abandoned direct UDP: {a:?}"
        );
        assert!(b.is_ok(), "controlled peer failed to join: {b:?}");
    }

    #[tokio::test]
    async fn two_peers_nominate_the_same_authenticated_pair() {
        for budget_ms in [450, 1000] {
            let left = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let pair_id = crate::connectivity::make_pair_id("left", "right");
            let mut left_pairs = vec![CandidatePair {
                pair_id: pair_id.clone(),
                local: left.local_addr().unwrap(),
                remote: right.local_addr().unwrap(),
                local_candidate_id: "left".into(),
                remote_candidate_id: "right".into(),
                priority: 100,
                state: PairState::Waiting,
            }];
            let mut right_pairs = vec![CandidatePair {
                pair_id,
                local: right.local_addr().unwrap(),
                remote: left.local_addr().unwrap(),
                local_candidate_id: "right".into(),
                remote_candidate_id: "left".into(),
                priority: 100,
                state: PairState::Waiting,
            }];
            let session = uuid::Uuid::new_v4();
            let key = b"per-session-probe-key";

            let (left_result, right_result) = tokio::join!(
                check_connectivity(
                    &left,
                    &mut left_pairs,
                    session,
                    11,
                    22,
                    key,
                    true,
                    Duration::from_millis(budget_ms),
                ),
                check_connectivity(
                    &right,
                    &mut right_pairs,
                    session,
                    22,
                    11,
                    key,
                    false,
                    Duration::from_millis(budget_ms),
                )
            );

            assert!(left_result.is_ok(), "left failed: {left_result:?}");
            assert!(right_result.is_ok(), "right failed: {right_result:?}");
            assert_eq!(left_result.unwrap().pair_id, right_result.unwrap().pair_id);
        }
    }

    #[tokio::test]
    async fn authenticated_probe_learns_peer_reflexive_endpoint() {
        let controlling = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let controlled = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let controlling_address = controlling.local_addr().unwrap();
        let controlled_address = controlled.local_addr().unwrap();
        let pair_id = crate::connectivity::make_pair_id("controlling", "controlled");
        let mut controlling_pairs = vec![CandidatePair {
            pair_id: pair_id.clone(),
            local: controlling_address,
            // Simulate a STUN port that differs from the source endpoint the
            // NAT actually uses when the controlled peer sends to us.
            remote: "127.0.0.1:9".parse().unwrap(),
            local_candidate_id: "controlling".into(),
            remote_candidate_id: "controlled".into(),
            priority: 100,
            state: PairState::Waiting,
        }];
        let mut controlled_pairs = vec![CandidatePair {
            pair_id,
            local: controlled_address,
            remote: controlling_address,
            local_candidate_id: "controlled".into(),
            remote_candidate_id: "controlling".into(),
            priority: 100,
            state: PairState::Waiting,
        }];
        let session = uuid::Uuid::new_v4();
        let key = b"peer-reflexive-probe-key";

        let (controlling_result, controlled_result) = tokio::join!(
            check_connectivity(
                &controlling,
                &mut controlling_pairs,
                session,
                31,
                42,
                key,
                true,
                Duration::from_secs(2),
            ),
            check_connectivity(
                &controlled,
                &mut controlled_pairs,
                session,
                42,
                31,
                key,
                false,
                Duration::from_secs(2),
            )
        );

        let controlling_selected = controlling_result.unwrap();
        let controlled_selected = controlled_result.unwrap();
        assert_eq!(controlling_selected.remote, controlled_address);
        assert_eq!(controlled_selected.remote, controlling_address);
        assert_eq!(controlling_selected.pair_id, controlled_selected.pair_id);
    }

    #[tokio::test]
    async fn controlling_nomination_keeps_multi_pair_selection_identical() {
        let left = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let high_id = crate::connectivity::make_pair_id("left-high", "right-high");
        let low_id = crate::connectivity::make_pair_id("left-low", "right-low");
        let mut left_pairs = vec![
            CandidatePair {
                pair_id: low_id.clone(),
                local: left.local_addr().unwrap(),
                remote: right.local_addr().unwrap(),
                local_candidate_id: "left-low".into(),
                remote_candidate_id: "right-low".into(),
                priority: 10,
                state: PairState::Waiting,
            },
            CandidatePair {
                pair_id: high_id.clone(),
                local: left.local_addr().unwrap(),
                remote: right.local_addr().unwrap(),
                local_candidate_id: "left-high".into(),
                remote_candidate_id: "right-high".into(),
                priority: 100,
                state: PairState::Waiting,
            },
        ];
        let mut right_pairs = vec![
            CandidatePair {
                pair_id: high_id,
                local: right.local_addr().unwrap(),
                remote: left.local_addr().unwrap(),
                local_candidate_id: "right-high".into(),
                remote_candidate_id: "left-high".into(),
                priority: 100,
                state: PairState::Waiting,
            },
            CandidatePair {
                pair_id: low_id,
                local: right.local_addr().unwrap(),
                remote: left.local_addr().unwrap(),
                local_candidate_id: "right-low".into(),
                remote_candidate_id: "left-low".into(),
                priority: 10,
                state: PairState::Waiting,
            },
        ];
        let session = uuid::Uuid::new_v4();
        let key = b"multi-pair-nomination-key";

        let (left_result, right_result) = tokio::join!(
            check_connectivity(
                &left,
                &mut left_pairs,
                session,
                101,
                202,
                key,
                true,
                Duration::from_secs(1),
            ),
            check_connectivity(
                &right,
                &mut right_pairs,
                session,
                202,
                101,
                key,
                false,
                Duration::from_secs(1),
            )
        );

        let left_selected = left_result.unwrap();
        let right_selected = right_result.unwrap();
        assert_eq!(left_selected.pair_id, right_selected.pair_id);
        assert_eq!(left_selected.priority, right_selected.priority);
        assert_eq!(left_selected.state, PairState::Nominated);
        assert_eq!(right_selected.state, PairState::Nominated);
    }
}
