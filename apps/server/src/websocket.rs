use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
    time::{Duration, Instant},
};

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, header},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use sanser_p2p::{CandidateError, P2pCandidate};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use crate::{
    auth::{AuthContext, authenticate_websocket},
    error::AppError,
    events,
    routes::devices,
    state::AppState,
};

const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;
const OUTBOUND_SIGNAL_CAPACITY: usize = 128;
const MAX_SIGNAL_PAYLOAD_BYTES: usize = 48 * 1024;
const MAX_CANDIDATE_PAYLOAD_BYTES: usize = 16 * 1024;
const MAX_CANDIDATES_PER_MESSAGE: usize = 16;
const MAX_CANDIDATES_PER_PEER: usize = 64;
const MAX_CANDIDATES_PER_SESSION: usize = 128;
const MAX_EPHEMERAL_CANDIDATE_SESSIONS: usize = 4_096;
const CANDIDATE_BUDGET_TTL: Duration = Duration::from_secs(5 * 60);
const SIGNAL_MESSAGES_PER_MINUTE: u16 = 240;

#[derive(Default)]
pub struct SignalHub {
    peers: Mutex<HashMap<PeerKey, SignalPeer>>,
    candidate_budgets: Mutex<HashMap<CandidateSessionKey, CandidateSessionBudget>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PeerKey {
    user_id: String,
    device_id: String,
}

struct SignalPeer {
    connection_id: Uuid,
    sender: mpsc::Sender<ForwardedSignal>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CandidateSessionKey {
    user_id: String,
    session_id: String,
}

struct CandidateSessionBudget {
    last_seen: Instant,
    peers: HashMap<String, PeerCandidateBudget>,
}

#[derive(Default)]
struct PeerCandidateBudget {
    latest_generation: u32,
    ids: HashMap<String, CandidateFingerprint>,
    endpoints: HashSet<CandidateFingerprint>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CandidateFingerprint {
    address: IpAddr,
    port: u16,
}

struct CandidateReservation {
    key: CandidateSessionKey,
    device_id: String,
    generation: u32,
    candidates: Vec<(String, CandidateFingerprint)>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardedSignal {
    session_id: String,
    sender_device_id: String,
    target_device_id: String,
    #[serde(rename = "type")]
    signal_type: String,
    payload: serde_json::Value,
}

impl SignalHub {
    async fn register(&self, key: PeerKey) -> (Uuid, mpsc::Receiver<ForwardedSignal>) {
        let connection_id = Uuid::new_v4();
        let (sender, receiver) = mpsc::channel(OUTBOUND_SIGNAL_CAPACITY);
        self.peers.lock().await.insert(
            key,
            SignalPeer {
                connection_id,
                sender,
            },
        );
        (connection_id, receiver)
    }

    async fn remove(&self, key: &PeerKey, connection_id: Uuid) {
        let removed = {
            let mut peers = self.peers.lock().await;
            if peers
                .get(key)
                .is_some_and(|peer| peer.connection_id == connection_id)
            {
                peers.remove(key);
                true
            } else {
                false
            }
        };
        if removed {
            let mut budgets = self.candidate_budgets.lock().await;
            budgets.retain(|session_key, budget| {
                if session_key.user_id == key.user_id {
                    budget.peers.remove(&key.device_id);
                }
                !budget.peers.is_empty()
            });
        }
    }

    async fn forward(&self, key: &PeerKey, signal: ForwardedSignal) -> Result<(), &'static str> {
        let sender = self
            .peers
            .lock()
            .await
            .get(key)
            .map(|peer| peer.sender.clone())
            .ok_or("target_offline")?;
        sender.try_send(signal).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => "target_backpressure",
            mpsc::error::TrySendError::Closed(_) => "target_offline",
        })
    }

    async fn reserve_candidates(
        &self,
        user_id: &str,
        session_id: &str,
        device_id: &str,
        batch: &ValidatedCandidateBatch,
    ) -> Result<CandidateReservation, SignalError> {
        let now = Instant::now();
        let mut budgets = self.candidate_budgets.lock().await;
        budgets.retain(|_, budget| now.duration_since(budget.last_seen) < CANDIDATE_BUDGET_TTL);

        let key = CandidateSessionKey {
            user_id: user_id.to_owned(),
            session_id: session_id.to_owned(),
        };
        if !budgets.contains_key(&key) && budgets.len() >= MAX_EPHEMERAL_CANDIDATE_SESSIONS {
            return Err(SignalError::new(
                "signaling_capacity",
                "ephemeral candidate capacity is temporarily exhausted",
            ));
        }

        let budget = budgets
            .entry(key.clone())
            .or_insert_with(|| CandidateSessionBudget {
                last_seen: now,
                peers: HashMap::new(),
            });

        let existing_peer = budget.peers.get(device_id);
        if existing_peer.is_some_and(|peer| batch.generation < peer.latest_generation) {
            return Err(stale_generation_error());
        }

        let same_generation =
            existing_peer.is_some_and(|peer| batch.generation == peer.latest_generation);
        let mut new_candidates = Vec::with_capacity(batch.candidates.len());
        for candidate in &batch.candidates {
            let endpoint = CandidateFingerprint {
                address: candidate.address,
                port: candidate.port,
            };
            if same_generation && let Some(peer) = existing_peer {
                if let Some(existing) = peer.ids.get(&candidate.id) {
                    if existing != &endpoint {
                        return Err(SignalError::new(
                            "candidate_id_collision",
                            "candidate id was reused for another endpoint",
                        ));
                    }
                    // Idempotent retry: do not consume quota, but still forward
                    // the batch because the earlier WebSocket delivery may
                    // have failed after it entered the target queue.
                    continue;
                }
                if peer.endpoints.contains(&endpoint) {
                    return Err(SignalError::new(
                        "duplicate_candidate",
                        "candidate endpoint was reused with another id",
                    ));
                }
            }
            new_candidates.push((candidate.id.clone(), endpoint));
        }

        let peer_count = if same_generation {
            existing_peer.map_or(0, |peer| peer.endpoints.len())
        } else {
            0
        };
        let mut session_count = budget
            .peers
            .values()
            .map(|peer| peer.endpoints.len())
            .sum::<usize>();
        if !same_generation {
            session_count =
                session_count.saturating_sub(existing_peer.map_or(0, |peer| peer.endpoints.len()));
        }
        if peer_count.saturating_add(new_candidates.len()) > MAX_CANDIDATES_PER_PEER {
            return Err(SignalError::new(
                "candidate_peer_limit",
                "candidate limit for this session peer was exceeded",
            ));
        }
        if session_count.saturating_add(new_candidates.len()) > MAX_CANDIDATES_PER_SESSION {
            return Err(SignalError::new(
                "candidate_session_limit",
                "candidate limit for this session was exceeded",
            ));
        }

        let peer = budget.peers.entry(device_id.to_owned()).or_default();
        budget.last_seen = now;
        if batch.generation > peer.latest_generation {
            peer.latest_generation = batch.generation;
            peer.ids.clear();
            peer.endpoints.clear();
        }
        for (id, endpoint) in &new_candidates {
            peer.ids.insert(id.clone(), endpoint.clone());
            peer.endpoints.insert(endpoint.clone());
        }
        Ok(CandidateReservation {
            key,
            device_id: device_id.to_owned(),
            generation: batch.generation,
            candidates: new_candidates,
        })
    }

    async fn advance_candidate_generation(
        &self,
        user_id: &str,
        session_id: &str,
        device_id: &str,
        generation: u32,
    ) -> Result<(), SignalError> {
        let now = Instant::now();
        let mut budgets = self.candidate_budgets.lock().await;
        budgets.retain(|_, budget| now.duration_since(budget.last_seen) < CANDIDATE_BUDGET_TTL);

        let key = CandidateSessionKey {
            user_id: user_id.to_owned(),
            session_id: session_id.to_owned(),
        };
        if !budgets.contains_key(&key) && budgets.len() >= MAX_EPHEMERAL_CANDIDATE_SESSIONS {
            return Err(SignalError::new(
                "signaling_capacity",
                "ephemeral candidate capacity is temporarily exhausted",
            ));
        }
        let budget = budgets
            .entry(key)
            .or_insert_with(|| CandidateSessionBudget {
                last_seen: now,
                peers: HashMap::new(),
            });
        let peer = budget.peers.entry(device_id.to_owned()).or_default();
        if generation < peer.latest_generation {
            return Err(stale_generation_error());
        }
        budget.last_seen = now;
        if generation > peer.latest_generation {
            peer.latest_generation = generation;
            peer.ids.clear();
            peer.endpoints.clear();
        }
        Ok(())
    }

    async fn release_candidates(&self, reservation: CandidateReservation) {
        let mut budgets = self.candidate_budgets.lock().await;
        if let Some(budget) = budgets.get_mut(&reservation.key) {
            if let Some(peer) = budget.peers.get_mut(&reservation.device_id)
                && peer.latest_generation == reservation.generation
            {
                for (id, endpoint) in reservation.candidates {
                    if peer.ids.get(&id) == Some(&endpoint) {
                        peer.ids.remove(&id);
                        peer.endpoints.remove(&endpoint);
                    }
                }
            }
        }
    }
}

fn stale_generation_error() -> SignalError {
    SignalError::new(
        "stale_candidate_generation",
        "candidate generation is older than the latest generation for this peer",
    )
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSocketQuery {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    since: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IncomingSignal {
    session_id: String,
    #[serde(default)]
    target_device_id: Option<String>,
    #[serde(rename = "type")]
    signal_type: String,
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CandidateBatchPayload {
    generation: u32,
    candidates: Vec<P2pCandidate>,
}

struct ValidatedCandidateBatch {
    generation: u32,
    candidates: Vec<P2pCandidate>,
}

enum ValidatedP2pPayload {
    Candidates(ValidatedCandidateBatch),
    CandidatesAcknowledged,
    GatheringComplete { generation: u32 },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GatheringCompletePayload {
    generation: u32,
}

pub async fn events_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WebSocketQuery>,
) -> Result<Response, AppError> {
    validate_origin(&state, &headers)?;
    let auth = authenticate_websocket(&state, &headers).await?;
    if query.since.is_some_and(|since| since < 0) {
        return Err(AppError::Validation(
            "since must be a Unix timestamp".into(),
        ));
    }
    Ok(ws
        .protocols(["sanser-v2"])
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_events_socket(socket, state, auth, query.since)))
}

pub async fn signaling_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WebSocketQuery>,
) -> Result<Response, AppError> {
    validate_origin(&state, &headers)?;
    let auth = authenticate_websocket(&state, &headers).await?;
    let device_id = query
        .device_id
        .as_deref()
        .ok_or_else(|| AppError::Validation("deviceId is required".into()))?;
    let device_id = normalize_uuid(device_id, "deviceId")?;
    devices::fetch_owned(&state, &auth.user_id, &device_id).await?;
    Ok(ws
        .protocols(["sanser-v2"])
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_signaling_socket(socket, state, auth, device_id)))
}

async fn run_events_socket(
    socket: WebSocket,
    state: AppState,
    auth: AuthContext,
    since: Option<i64>,
) {
    let mut subscription = state.events.subscribe();
    let replay = match events::recent(&state, &auth.user_id, since, 256).await {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(user_id = %auth.user_id, error = %error, "event replay failed");
            Vec::new()
        }
    };
    let (mut writer, mut reader) = socket.split();
    let mut sent = HashSet::with_capacity(512);
    for event in replay {
        sent.insert(event.id.clone());
        if send_json(&mut writer, &event).await.is_err() {
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            event = subscription.recv() => {
                match event {
                    Ok(event) if event.user_id == auth.user_id && sent.insert(event.id.clone()) => {
                        if sent.len() > 1024 {
                            sent.clear();
                            sent.insert(event.id.clone());
                        }
                        if send_json(&mut writer, &event).await.is_err() { break; }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let notice = serde_json::json!({"type":"events.resyncRequired"});
                        if send_json(&mut writer, &notice).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = reader.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if writer.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                if writer.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
}

async fn run_signaling_socket(
    socket: WebSocket,
    state: AppState,
    auth: AuthContext,
    device_id: String,
) {
    let key = PeerKey {
        user_id: auth.user_id.clone(),
        device_id: device_id.clone(),
    };
    let (connection_id, mut outbound) = state.signaling.register(key.clone()).await;
    let (mut writer, mut reader) = socket.split();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut invalid_messages = 0_u8;
    let mut rate_window_started = Instant::now();
    let mut rate_window_messages = 0_u16;

    loop {
        tokio::select! {
            outgoing = outbound.recv() => {
                match outgoing {
                    Some(signal) => {
                        if send_json(&mut writer, &signal).await.is_err() { break; }
                    }
                    None => break,
                }
            }
            incoming = reader.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let now = Instant::now();
                        if now.duration_since(rate_window_started) >= Duration::from_secs(60) {
                            rate_window_started = now;
                            rate_window_messages = 0;
                        }
                        if rate_window_messages >= SIGNAL_MESSAGES_PER_MINUTE {
                            invalid_messages = invalid_messages.saturating_add(1);
                            if send_ws_error(&mut writer, "signal_rate_limited", "signaling message rate limit exceeded").await.is_err() { break; }
                            if invalid_messages >= 8 { break; }
                            continue;
                        }
                        rate_window_messages = rate_window_messages.saturating_add(1);
                        let parsed = serde_json::from_str::<IncomingSignal>(&text);
                        match parsed {
                            Ok(signal) => {
                                match relay_signal(&state, &auth.user_id, &device_id, signal).await {
                                    Ok(()) => {}
                                    Err(error) => {
                                        // A peer opening its signaling socket a
                                        // little later is an expected race, not
                                        // malformed client behavior. Keep this
                                        // connection alive while its bounded
                                        // retry loop waits for the target.
                                        if !error.is_transient_delivery_failure() {
                                            invalid_messages = invalid_messages.saturating_add(1);
                                        }
                                        if send_ws_error(&mut writer, error.code(), &error.to_string()).await.is_err() { break; }
                                    }
                                }
                            }
                            Err(_) => {
                                invalid_messages = invalid_messages.saturating_add(1);
                                if send_ws_error(&mut writer, "invalid_signal", "invalid signaling message").await.is_err() { break; }
                            }
                        }
                        if invalid_messages >= 8 { break; }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if writer.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Binary(_))) => {
                        invalid_messages = invalid_messages.saturating_add(1);
                        if send_ws_error(&mut writer, "binary_not_supported", "signaling uses JSON text messages").await.is_err() { break; }
                    }
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                if writer.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
    state.signaling.remove(&key, connection_id).await;
}

async fn relay_signal(
    state: &AppState,
    user_id: &str,
    sender_device_id: &str,
    signal: IncomingSignal,
) -> Result<(), SignalError> {
    let (target_device_id, signal, p2p_payload) =
        authorize_signal(state, user_id, sender_device_id, signal).await?;
    let reservation = match p2p_payload.as_ref() {
        Some(ValidatedP2pPayload::Candidates(batch)) => Some(
            state
                .signaling
                .reserve_candidates(user_id, &signal.session_id, sender_device_id, batch)
                .await?,
        ),
        Some(ValidatedP2pPayload::GatheringComplete { generation }) => {
            state
                .signaling
                .advance_candidate_generation(
                    user_id,
                    &signal.session_id,
                    sender_device_id,
                    *generation,
                )
                .await?;
            None
        }
        Some(ValidatedP2pPayload::CandidatesAcknowledged) => None,
        None => None,
    };
    let target = PeerKey {
        user_id: user_id.to_owned(),
        device_id: target_device_id.clone(),
    };
    let forwarded = ForwardedSignal {
        session_id: signal.session_id,
        sender_device_id: sender_device_id.to_owned(),
        target_device_id,
        signal_type: signal.signal_type,
        payload: signal.payload,
    };
    if let Err(code) = state.signaling.forward(&target, forwarded).await {
        if let Some(reservation) = reservation {
            state.signaling.release_candidates(reservation).await;
        }
        return Err(match code {
            "target_backpressure" => SignalError::new(
                "target_backpressure",
                "signal target is not accepting messages quickly enough",
            ),
            _ => SignalError::new("target_offline", "signal target is unavailable"),
        });
    }
    Ok(())
}

async fn authorize_signal(
    state: &AppState,
    user_id: &str,
    sender_device_id: &str,
    mut signal: IncomingSignal,
) -> Result<(String, IncomingSignal, Option<ValidatedP2pPayload>), SignalError> {
    signal.session_id = normalize_uuid(&signal.session_id, "sessionId")
        .map_err(|_| SignalError::new("invalid_session", "sessionId must be a UUID"))?;
    if !matches!(
        signal.signal_type.as_str(),
        "offer"
            | "answer"
            | "iceCandidate"
            | "renegotiate"
            | "connectionState"
            | "p2p.candidates"
            | "p2p.candidatesAck"
            | "p2p.gatheringComplete"
    ) {
        return Err(SignalError::new(
            "invalid_signal_type",
            "unsupported signaling message type",
        ));
    }
    let p2p_payload = validate_signal_payload(&signal)?;
    let row = sqlx::query(
        "SELECT requester_device_id, host_device_id, state, selected_transport \
         FROM connection_sessions \
         WHERE id = $1 AND user_id = $2",
    )
    .bind(&signal.session_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| {
        tracing::error!(error = %error, "signaling authorization query failed");
        SignalError::new("server_error", "unable to authorize signal")
    })?
    .ok_or_else(|| SignalError::new("session_not_found", "session was not found"))?;
    let requester: String = row
        .try_get("requester_device_id")
        .map_err(|_| SignalError::new("server_error", "unable to authorize signal"))?;
    let host: String = row
        .try_get("host_device_id")
        .map_err(|_| SignalError::new("server_error", "unable to authorize signal"))?;
    let session_state: String = row
        .try_get("state")
        .map_err(|_| SignalError::new("server_error", "unable to authorize signal"))?;
    if session_state != "accepted" {
        return Err(SignalError::new(
            "session_not_accepted",
            "session must be accepted before signaling",
        ));
    }
    let selected_transport: Option<String> = row
        .try_get("selected_transport")
        .map_err(|_| SignalError::new("server_error", "unable to authorize signal"))?;
    if signal.signal_type.starts_with("p2p.") && selected_transport.as_deref() != Some("native") {
        return Err(SignalError::new(
            "p2p_transport_required",
            "native P2P signaling requires a native transport session",
        ));
    }
    let target = if sender_device_id == requester {
        host
    } else if sender_device_id == host {
        requester
    } else {
        return Err(SignalError::new(
            "forbidden",
            "device is not a participant in this session",
        ));
    };
    if let Some(requested_target) = signal.target_device_id.as_deref() {
        let requested_target = normalize_uuid(requested_target, "targetDeviceId")
            .map_err(|_| SignalError::new("invalid_target", "targetDeviceId must be a UUID"))?;
        if requested_target != target {
            return Err(SignalError::new(
                "forbidden",
                "target is not the other session participant",
            ));
        }
    }
    signal.target_device_id = Some(target.clone());
    Ok((target, signal, p2p_payload))
}

fn validate_signal_payload(
    signal: &IncomingSignal,
) -> Result<Option<ValidatedP2pPayload>, SignalError> {
    let payload_size = serde_json::to_vec(&signal.payload)
        .map_err(|_| SignalError::new("invalid_signal", "unable to encode signaling payload"))?
        .len();
    if payload_size > MAX_SIGNAL_PAYLOAD_BYTES {
        return Err(SignalError::new(
            "signal_payload_too_large",
            "signaling payload exceeds the allowed size",
        ));
    }

    match signal.signal_type.as_str() {
        "p2p.candidates" => {
            if payload_size > MAX_CANDIDATE_PAYLOAD_BYTES {
                return Err(SignalError::new(
                    "candidate_payload_too_large",
                    "candidate payload exceeds the allowed size",
                ));
            }
            let payload = serde_json::from_value::<CandidateBatchPayload>(signal.payload.clone())
                .map_err(|_| {
                SignalError::new(
                    "invalid_candidate_payload",
                    "candidate payload has an invalid shape",
                )
            })?;
            validate_generation(payload.generation)?;
            if payload.candidates.is_empty()
                || payload.candidates.len() > MAX_CANDIDATES_PER_MESSAGE
            {
                return Err(SignalError::new(
                    "candidate_message_limit",
                    "candidate message must contain between 1 and 16 candidates",
                ));
            }

            let mut ids = HashSet::with_capacity(payload.candidates.len());
            let mut endpoints = HashSet::with_capacity(payload.candidates.len());
            for candidate in &payload.candidates {
                validate_candidate(candidate)?;
                if !ids.insert(candidate.id.as_str())
                    || !endpoints.insert((candidate.address, candidate.port))
                {
                    return Err(SignalError::new(
                        "duplicate_candidate",
                        "candidate message contains a duplicate candidate",
                    ));
                }
            }
            Ok(Some(ValidatedP2pPayload::Candidates(
                ValidatedCandidateBatch {
                    generation: payload.generation,
                    candidates: payload.candidates,
                },
            )))
        }
        "p2p.gatheringComplete" => {
            let payload =
                serde_json::from_value::<GatheringCompletePayload>(signal.payload.clone())
                    .map_err(|_| {
                        SignalError::new(
                            "invalid_gathering_payload",
                            "gathering-complete payload has an invalid shape",
                        )
                    })?;
            validate_generation(payload.generation)?;
            Ok(Some(ValidatedP2pPayload::GatheringComplete {
                generation: payload.generation,
            }))
        }
        "p2p.candidatesAck" => {
            let payload =
                serde_json::from_value::<GatheringCompletePayload>(signal.payload.clone())
                    .map_err(|_| {
                        SignalError::new(
                            "invalid_candidate_ack",
                            "candidate acknowledgement payload has an invalid shape",
                        )
                    })?;
            validate_generation(payload.generation)?;
            Ok(Some(ValidatedP2pPayload::CandidatesAcknowledged))
        }
        _ => Ok(None),
    }
}

fn validate_generation(generation: u32) -> Result<(), SignalError> {
    if generation != 0 {
        Ok(())
    } else {
        Err(SignalError::new(
            "invalid_candidate_generation",
            "candidate generation must be non-zero",
        ))
    }
}

fn validate_candidate(candidate: &P2pCandidate) -> Result<(), SignalError> {
    candidate.validate().map_err(|error| match error {
        CandidateError::InvalidId => SignalError::new(
            "invalid_candidate_id",
            "candidate id is empty, oversized, or contains unsupported characters",
        ),
        CandidateError::InvalidFoundation => SignalError::new(
            "invalid_candidate_foundation",
            "candidate foundation is empty, oversized, or contains unsupported characters",
        ),
        CandidateError::InvalidAddress(_) | CandidateError::AddressTypeMismatch => {
            SignalError::new(
                "invalid_candidate_address",
                "candidate address is not routable for its declared type",
            )
        }
        CandidateError::InvalidMapping => SignalError::new(
            "invalid_mapping_protocol",
            "candidate type and mappingProtocol do not match",
        ),
        CandidateError::InvalidInterfaceIndex => SignalError::new(
            "invalid_candidate_interface",
            "candidate interfaceIndex must be greater than zero",
        ),
        CandidateError::InvalidPort | CandidateError::InvalidPriority(_) => SignalError::new(
            "invalid_candidate",
            "candidate port and deterministic priority must be valid",
        ),
        CandidateError::InvalidCapacity(_)
        | CandidateError::CapacityExceeded(_)
        | CandidateError::IdCollision(_) => SignalError::new(
            "invalid_candidate",
            "candidate metadata failed bounded validation",
        ),
    })
}

pub(crate) fn validate_origin(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let Some(origin) = headers.get(header::ORIGIN) else {
        // Native libdatachannel and Tauri sidecars may not send an Origin header.
        return Ok(());
    };
    if state
        .config
        .allowed_origins
        .iter()
        .any(|allowed| allowed == origin)
    {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn normalize_uuid(value: &str, field: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation(format!("{field} must be a UUID")))
}

async fn send_json<T: Serialize>(
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    value: &T,
) -> Result<(), ()> {
    let json = serde_json::to_string(value).map_err(|_| ())?;
    writer
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

async fn send_ws_error(
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    code: &str,
    message: &str,
) -> Result<(), ()> {
    send_json(
        writer,
        &serde_json::json!({"type":"error", "error":{"code":code, "message":message}}),
    )
    .await
}

struct SignalError {
    code: &'static str,
    message: &'static str,
}

impl SignalError {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    const fn code(&self) -> &'static str {
        self.code
    }

    fn is_transient_delivery_failure(&self) -> bool {
        matches!(self.code, "target_offline" | "target_backpressure")
    }
}

impl std::fmt::Display for SignalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

#[cfg(test)]
mod tests {
    use sanser_p2p::{CandidateType, MappingProtocol, TransportProtocol, candidate_priority};
    use serde_json::json;

    use super::*;

    fn candidate(id: &str, address: &str, port: u16) -> serde_json::Value {
        let priority = candidate_priority(CandidateType::ServerReflexive, MappingProtocol::None, 1)
            .unwrap_or_else(|error| panic!("candidate priority failed: {error}"));
        json!({
            "id": id,
            "type": "serverReflexive",
            "address": address,
            "port": port,
            "protocol": "udp",
            "mappingProtocol": "none",
            "priority": priority,
            "foundation": "srflx"
        })
    }

    fn candidate_signal(candidates: Vec<serde_json::Value>) -> IncomingSignal {
        IncomingSignal {
            session_id: Uuid::nil().to_string(),
            target_device_id: None,
            signal_type: "p2p.candidates".to_owned(),
            payload: json!({"generation": 1, "candidates": candidates}),
        }
    }

    fn parsed_candidate(index: u16) -> P2pCandidate {
        let priority = candidate_priority(CandidateType::ServerReflexive, MappingProtocol::None, 1)
            .unwrap_or_else(|error| panic!("candidate priority failed: {error}"));
        P2pCandidate {
            id: format!("candidate-{index}"),
            candidate_type: CandidateType::ServerReflexive,
            address: IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1)),
            port: 10_000_u16.saturating_add(index),
            protocol: TransportProtocol::Udp,
            interface_index: Some(1),
            mapping_protocol: MappingProtocol::None,
            priority,
            foundation: "srflx".to_owned(),
        }
    }

    #[test]
    fn candidate_payload_accepts_valid_udp_metadata() {
        let signal = candidate_signal(vec![candidate("srflx-1", "1.1.1.1", 43_892)]);
        let result = validate_signal_payload(&signal);
        assert!(matches!(
            result,
            Ok(Some(ValidatedP2pPayload::Candidates(batch))) if batch.candidates.len() == 1
        ));
    }

    #[test]
    fn candidate_payload_rejects_unsafe_or_duplicate_metadata() {
        let loopback = candidate_signal(vec![candidate("host-1", "127.0.0.1", 50_000)]);
        assert_eq!(
            validate_signal_payload(&loopback)
                .err()
                .map(|error| error.code()),
            Some("invalid_candidate_address")
        );

        let duplicate = candidate("duplicate", "1.0.0.1", 50_001);
        let duplicates = candidate_signal(vec![duplicate.clone(), duplicate]);
        assert_eq!(
            validate_signal_payload(&duplicates)
                .err()
                .map(|error| error.code()),
            Some("duplicate_candidate")
        );

        let tcp = candidate_signal(vec![json!({
            "id": "tcp-1",
            "type": "host",
            "address": "192.168.1.5",
            "port": 50002,
            "protocol": "tcp",
            "priority": 100
        })]);
        assert_eq!(
            validate_signal_payload(&tcp)
                .err()
                .map(|error| error.code()),
            Some("invalid_candidate_payload")
        );
    }

    #[test]
    fn candidate_message_count_is_bounded_and_generation_is_nonzero() {
        let candidates = (0_u16..=MAX_CANDIDATES_PER_MESSAGE as u16)
            .map(|index| candidate(&format!("candidate-{index}"), "8.8.8.8", 20_000 + index))
            .collect();
        let too_many = candidate_signal(candidates);
        assert_eq!(
            validate_signal_payload(&too_many)
                .err()
                .map(|error| error.code()),
            Some("candidate_message_limit")
        );

        let invalid_generation = IncomingSignal {
            session_id: Uuid::nil().to_string(),
            target_device_id: None,
            signal_type: "p2p.gatheringComplete".to_owned(),
            payload: json!({"generation": 0}),
        };
        assert_eq!(
            validate_signal_payload(&invalid_generation)
                .err()
                .map(|error| error.code()),
            Some("invalid_candidate_generation")
        );

        let maximum_generation = IncomingSignal {
            session_id: Uuid::nil().to_string(),
            target_device_id: None,
            signal_type: "p2p.gatheringComplete".to_owned(),
            payload: json!({"generation": u32::MAX}),
        };
        assert!(validate_signal_payload(&maximum_generation).is_ok());

        let acknowledgement = IncomingSignal {
            session_id: Uuid::nil().to_string(),
            target_device_id: None,
            signal_type: "p2p.candidatesAck".to_owned(),
            payload: json!({"generation": 1}),
        };
        assert!(matches!(
            validate_signal_payload(&acknowledgement),
            Ok(Some(ValidatedP2pPayload::CandidatesAcknowledged))
        ));
    }

    #[tokio::test]
    async fn candidate_budget_is_per_peer_and_rolls_back_failed_delivery() {
        let hub = SignalHub::default();
        for batch_index in 0_u16..4 {
            let first = batch_index * MAX_CANDIDATES_PER_MESSAGE as u16;
            let candidates = (first..first + MAX_CANDIDATES_PER_MESSAGE as u16)
                .map(parsed_candidate)
                .collect();
            let batch = ValidatedCandidateBatch {
                generation: 1,
                candidates,
            };
            assert!(
                hub.reserve_candidates("user", "session", "peer", &batch)
                    .await
                    .is_ok()
            );
        }

        let overflow = ValidatedCandidateBatch {
            generation: 1,
            candidates: vec![parsed_candidate(MAX_CANDIDATES_PER_PEER as u16)],
        };
        assert_eq!(
            hub.reserve_candidates("user", "session", "peer", &overflow)
                .await
                .err()
                .map(|error| error.code()),
            Some("candidate_peer_limit")
        );

        for batch_index in 0_u16..4 {
            let first = 100 + batch_index * MAX_CANDIDATES_PER_MESSAGE as u16;
            let batch = ValidatedCandidateBatch {
                generation: 1,
                candidates: (first..first + MAX_CANDIDATES_PER_MESSAGE as u16)
                    .map(parsed_candidate)
                    .collect(),
            };
            assert!(
                hub.reserve_candidates("user", "session", "other-peer", &batch)
                    .await
                    .is_ok()
            );
        }
        let session_overflow = ValidatedCandidateBatch {
            generation: 1,
            candidates: vec![parsed_candidate(200)],
        };
        assert_eq!(
            hub.reserve_candidates("user", "session", "third-peer", &session_overflow)
                .await
                .err()
                .map(|error| error.code()),
            Some("candidate_session_limit")
        );

        let retryable = ValidatedCandidateBatch {
            generation: 1,
            candidates: vec![parsed_candidate(500)],
        };
        let reservation = hub
            .reserve_candidates("user", "other-session", "peer", &retryable)
            .await;
        assert!(reservation.is_ok());
        if let Ok(reservation) = reservation {
            hub.release_candidates(reservation).await;
        }
        assert!(
            hub.reserve_candidates("user", "other-session", "peer", &retryable)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn candidate_budget_tracks_only_latest_generation_and_rejects_stale_messages() {
        let hub = SignalHub::default();
        let generation_one = ValidatedCandidateBatch {
            generation: 1,
            candidates: (0..MAX_CANDIDATES_PER_MESSAGE as u16)
                .map(parsed_candidate)
                .collect(),
        };
        assert!(
            hub.reserve_candidates("user", "session", "peer", &generation_one)
                .await
                .is_ok()
        );

        let generation_two = ValidatedCandidateBatch {
            generation: 2,
            candidates: (100..100 + MAX_CANDIDATES_PER_MESSAGE as u16)
                .map(parsed_candidate)
                .collect(),
        };
        assert!(
            hub.reserve_candidates("user", "session", "peer", &generation_two)
                .await
                .is_ok()
        );
        assert_eq!(
            hub.reserve_candidates("user", "session", "peer", &generation_one)
                .await
                .err()
                .map(|error| error.code()),
            Some("stale_candidate_generation")
        );

        assert!(
            hub.advance_candidate_generation("user", "session", "peer", 3)
                .await
                .is_ok()
        );
        assert_eq!(
            hub.advance_candidate_generation("user", "session", "peer", 2)
                .await
                .err()
                .map(|error| error.code()),
            Some("stale_candidate_generation")
        );
    }

    #[tokio::test]
    async fn identical_candidate_retry_is_idempotent_and_forwardable() {
        let hub = SignalHub::default();
        let batch = ValidatedCandidateBatch {
            generation: 1,
            candidates: vec![parsed_candidate(7)],
        };
        let first = hub
            .reserve_candidates("user", "session", "peer", &batch)
            .await;
        assert!(first.is_ok());
        if let Ok(first) = first {
            assert_eq!(first.candidates.len(), 1);
        }

        let retry = hub
            .reserve_candidates("user", "session", "peer", &batch)
            .await;
        assert!(retry.is_ok());
        if let Ok(retry) = retry {
            assert!(retry.candidates.is_empty());
        }
    }

    #[test]
    fn offline_peer_does_not_consume_the_protocol_violation_budget() {
        assert!(SignalError::new("target_offline", "offline").is_transient_delivery_failure());
        assert!(SignalError::new("target_backpressure", "busy").is_transient_delivery_failure());
        assert!(!SignalError::new("forbidden", "forbidden").is_transient_delivery_failure());
    }
}
