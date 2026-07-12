//! UDP hole punching orchestration.

use crate::connectivity::{
    CandidatePair, PairState, ProbeFields, build_probe_packet, parse_probe_packet,
    PUNCH_SCHEDULE, compute_pair_hash,
};
use crate::error::P2pError;
use serde::Serialize;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use std::collections::HashMap;

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
    hmac_key: &[u8],
    controlling: bool,
    timeout: Duration,
) -> Result<CandidatePair, P2pError> {
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
        attempts.push(PunchAttempt::new(pair.pair_id.clone(), pair.local, pair.remote));
    }

    let start_time = std::time::Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(10));

    let mut buf = [0u8; 1024];

    loop {
        let elapsed = start_time.elapsed();
        if elapsed >= timeout {
            break;
        }

        tokio::select! {
            _ = interval.tick() => {
                // Send probes based on the PUNCH_SCHEDULE
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
                            
                            // Build a request probe (nonce represents request if top bit is 0)
                            let fields = ProbeFields {
                                session_id,
                                sender_device_hash: local_device_hash,
                                pair_hash: compute_pair_hash(&pair.pair_id),
                                transaction_id: attempt.probes_sent,
                                timestamp: start_time.elapsed().as_millis() as u64,
                                nonce: attempt.probes_sent, // Request indicator
                            };

                            if let Ok(packet) = build_probe_packet(&fields, hmac_key) {
                                if socket.send_to(&packet, pair.remote).await.is_ok() {
                                    attempt.probes_sent += 1;
                                }
                            }
                        }
                    } else if attempt.probes_sent as usize >= PUNCH_SCHEDULE.len() && attempt.state == PunchState::Punching {
                        attempt.state = PunchState::Exhausted;
                        pair.state = PairState::Failed;
                    }
                }
            }
            recv_res = socket.recv_from(&mut buf) => {
                if let Ok((n, from)) = recv_res {
                    // Try to parse as an authenticated probe
                    if let Ok(parsed) = parse_probe_packet(&buf[..n], hmac_key) {
                        if parsed.session_id == session_id {
                            // Find the corresponding pair
                            if let Some(&pair_idx) = pair_map.get(&parsed.pair_hash) {
                                let pair = &mut pairs[pair_idx];
                                let attempt = &mut attempts[pair_idx];

                                attempt.probes_received += 1;

                                // If the nonce is even, it's a request. Send a response back.
                                if parsed.nonce % 2 == 0 {
                                    peer_verified.insert(pair.pair_id.clone(), true);
                                    
                                    // Reply with an ACK/response probe (using odd nonce to indicate response)
                                    let fields = ProbeFields {
                                        session_id,
                                        sender_device_hash: local_device_hash,
                                        pair_hash: parsed.pair_hash,
                                        transaction_id: parsed.transaction_id,
                                        timestamp: start_time.elapsed().as_millis() as u64,
                                        nonce: parsed.nonce | 1, // Response indicator
                                    };
                                    if let Ok(packet) = build_probe_packet(&fields, hmac_key) {
                                        let _ = socket.send_to(&packet, from).await;
                                    }
                                } else {
                                    // It's a response to one of our requests
                                    local_verified.insert(pair.pair_id.clone(), true);
                                    attempt.rtt_ms = Some(elapsed.as_millis() as f64 - parsed.timestamp as f64);
                                }

                                // If both directions are verified, established bidirectional success!
                                if peer_verified.contains_key(&pair.pair_id) && local_verified.contains_key(&pair.pair_id) {
                                    pair.state = PairState::Succeeded;
                                    attempt.state = PunchState::Succeeded;

                                    // If we are controlling, or we just want the highest priority to win,
                                    // we can return immediately or when controlling nominates.
                                    // To be robust and support racing, we can return the highest-priority succeeded pair.
                                    if controlling {
                                        pair.state = PairState::Nominated;
                                    }
                                    return Ok(pair.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // After timeout, check if any succeeded
    for pair in pairs.iter() {
        if pair.state == PairState::Succeeded || pair.state == PairState::Nominated {
            return Ok(pair.clone());
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
        let mut pairs = vec![
            CandidatePair {
                pair_id: "pair-1".into(),
                local: socket.local_addr().unwrap(),
                remote: "127.0.0.1:55555".parse().unwrap(),
                local_candidate_id: "c-1".into(),
                remote_candidate_id: "c-2".into(),
                priority: 100,
                state: PairState::Waiting,
            }
        ];
        let res = check_connectivity(
            &socket,
            &mut pairs,
            uuid::Uuid::new_v4(),
            1,
            b"key",
            true,
            Duration::from_millis(50),
        ).await;

        assert!(matches!(res, Err(P2pError::NoDirectRoute)));
    }
}
