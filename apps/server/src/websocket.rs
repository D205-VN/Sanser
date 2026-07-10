use std::{
    collections::{HashMap, HashSet},
    time::Duration,
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

#[derive(Default)]
pub struct SignalHub {
    peers: Mutex<HashMap<PeerKey, SignalPeer>>,
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
        let mut peers = self.peers.lock().await;
        if peers
            .get(key)
            .is_some_and(|peer| peer.connection_id == connection_id)
        {
            peers.remove(key);
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
                        let parsed = serde_json::from_str::<IncomingSignal>(&text);
                        match parsed {
                            Ok(signal) => {
                                match authorize_signal(&state, &auth.user_id, &device_id, signal).await {
                                    Ok((target_device_id, signal)) => {
                                        let target = PeerKey {
                                            user_id: auth.user_id.clone(),
                                            device_id: target_device_id.clone(),
                                        };
                                        let forwarded = ForwardedSignal {
                                            session_id: signal.session_id,
                                            sender_device_id: device_id.clone(),
                                            target_device_id,
                                            signal_type: signal.signal_type,
                                            payload: signal.payload,
                                        };
                                        if let Err(code) = state.signaling.forward(&target, forwarded).await {
                                            if send_ws_error(&mut writer, code, "signal target is unavailable").await.is_err() { break; }
                                        }
                                    }
                                    Err(error) => {
                                        invalid_messages = invalid_messages.saturating_add(1);
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

async fn authorize_signal(
    state: &AppState,
    user_id: &str,
    sender_device_id: &str,
    mut signal: IncomingSignal,
) -> Result<(String, IncomingSignal), SignalError> {
    signal.session_id = normalize_uuid(&signal.session_id, "sessionId")
        .map_err(|_| SignalError::new("invalid_session", "sessionId must be a UUID"))?;
    if !matches!(
        signal.signal_type.as_str(),
        "offer" | "answer" | "iceCandidate" | "renegotiate" | "connectionState"
    ) {
        return Err(SignalError::new(
            "invalid_signal_type",
            "unsupported signaling message type",
        ));
    }
    let row = sqlx::query(
        "SELECT requester_device_id, host_device_id, state FROM connection_sessions \
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
    Ok((target, signal))
}

fn validate_origin(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
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
}

impl std::fmt::Display for SignalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}
