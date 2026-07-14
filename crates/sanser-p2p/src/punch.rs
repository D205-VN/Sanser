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
use std::time::Duration;
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

/// Runs the connectivity check and hole punching for a list of candidate pairs.
///
/// Returns the highest-priority pair that successfully establishes bidirectional connectivity.
///
/// # Errors
///
/// Returns a [`P2pError::NoDirectRoute`] if no candidate pair establishes a connection
/// within the timeout period.
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
    const NOMINATION_TRANSACTION_ID: u32 = u32::MAX;
    const NOMINATION_RETRY_INTERVAL: Duration = Duration::from_millis(50);
    const CONTROLLED_LINGER: Duration = Duration::from_millis(300);
    // Sort pairs by priority descending (highest priority first)
    pairs.sort_by(|a, b| b.priority.cmp(&a.priority));

    // Map each pair to their identifier hashes for fast lookups
    let mut pair_map = HashMap::new();
    let mut peer_verified = HashMap::new(); // Tracks if remote -> local has succeeded
    let mut local_verified = HashMap::new(); // Tracks if local -> remote (ACK) has succeeded
    let mut attempts = Vec::new();

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
                    return Ok(pair.clone());
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
                        if pair.state == PairState::Succeeded || pair.state == PairState::Failed {
                            continue;
                        }

                        let attempt = &mut attempts[idx];
                        let next_probe_idx = attempt.probes_sent as usize;

                        if next_probe_idx < PUNCH_SCHEDULE.len() {
                            let scheduled_delay = PUNCH_SCHEDULE[next_probe_idx].delay_ms;
                            if elapsed_ms >= scheduled_delay {
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
                                    if socket.send_to(&packet, pair.remote).await.is_ok() {
                                        attempt.probes_sent += 1;
                                        pair.state = PairState::InProgress;
                                    }
                                }
                            }
                        } else if attempt.probes_sent as usize >= PUNCH_SCHEDULE.len()
                            && attempt.state == PunchState::Punching
                            && !(peer_verified.contains_key(&pair.pair_id)
                                && local_verified.contains_key(&pair.pair_id))
                        {
                            attempt.state = PunchState::Exhausted;
                            pair.state = PairState::Failed;
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

                                // A valid HMAC is not sufficient to nominate an
                                // address. Lock checks to the endpoint that was
                                // authorized by signaling for this pair.
                                if from != pair.remote {
                                    continue;
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
                                                return Ok(pair.clone());
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
                                    // It's a response to one of our requests
                                    if parsed.transaction_id >= attempt.probes_sent {
                                        continue;
                                    }
                                    local_verified.insert(pair.pair_id.clone(), true);
                                    attempt.rtt_ms = Some(
                                        start_time
                                            .elapsed()
                                            .as_millis()
                                            .saturating_sub(u128::from(parsed.timestamp))
                                            as f64,
                                    );
                                }

                                // Only the controlling peer nominates. Both
                                // engines return the exact same pair after the
                                // controlled peer acknowledges that nomination.
                                if peer_verified.contains_key(&pair.pair_id)
                                    && local_verified.contains_key(&pair.pair_id)
                                {
                                    pair.state = PairState::Succeeded;
                                    attempt.state = PunchState::Succeeded;
                                    if controlling && nominated_pair.is_none() {
                                        pair.state = PairState::Nominated;
                                        nominated_pair = Some(pair_idx);
                                        last_nomination_sent = None;
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
    async fn two_peers_nominate_the_same_authenticated_pair() {
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
                Duration::from_secs(1),
            ),
            check_connectivity(
                &right,
                &mut right_pairs,
                session,
                22,
                11,
                key,
                false,
                Duration::from_secs(1),
            )
        );

        assert!(left_result.is_ok(), "left failed: {left_result:?}");
        assert!(right_result.is_ok(), "right failed: {right_result:?}");
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
