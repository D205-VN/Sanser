use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use axum::{
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use sqlx::Row;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use crate::{
    auth::{AuthContext, authenticate_websocket},
    error::AppError,
    routes::devices,
    state::AppState,
    websocket::validate_origin,
};

const MAX_RELAY_FRAME_BYTES: usize = 64 * 1024;
const RELAY_QUEUE_CAPACITY: usize = 512;
const MAX_ACTIVE_RELAY_SESSIONS: usize = 4_096;
const MAX_RELAY_BYTES_PER_SECOND: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RelayKey {
    user_id: String,
    session_id: String,
}

struct RelayPeer {
    connection_id: Uuid,
    peer_device_id: String,
    sender: mpsc::Sender<RelayOutbound>,
}

#[derive(Clone, Debug)]
enum RelayOutbound {
    Binary(Vec<u8>),
    PeerReady,
    PeerOffline,
}

#[derive(Default)]
pub struct RelayHub {
    sessions: Mutex<HashMap<RelayKey, HashMap<String, RelayPeer>>>,
}

impl RelayHub {
    async fn register(
        &self,
        key: RelayKey,
        device_id: String,
        peer_device_id: String,
    ) -> Result<(Uuid, mpsc::Receiver<RelayOutbound>), AppError> {
        let connection_id = Uuid::new_v4();
        let (sender, receiver) = mpsc::channel(RELAY_QUEUE_CAPACITY);
        let mut sessions = self.sessions.lock().await;
        if !sessions.contains_key(&key) && sessions.len() >= MAX_ACTIVE_RELAY_SESSIONS {
            return Err(AppError::Unavailable);
        }
        let peers = sessions.entry(key).or_default();
        peers.insert(
            device_id,
            RelayPeer {
                connection_id,
                peer_device_id: peer_device_id.clone(),
                sender: sender.clone(),
            },
        );
        if let Some(peer) = peers.get(&peer_device_id) {
            let _ = peer.sender.try_send(RelayOutbound::PeerReady);
            let _ = sender.try_send(RelayOutbound::PeerReady);
        }
        Ok((connection_id, receiver))
    }

    async fn forward(
        &self,
        key: &RelayKey,
        sender_device_id: &str,
        connection_id: Uuid,
        payload: Vec<u8>,
    ) -> Result<(), &'static str> {
        let sender = {
            let sessions = self.sessions.lock().await;
            let peers = sessions.get(key).ok_or("peer_offline")?;
            let current = peers.get(sender_device_id).ok_or("peer_offline")?;
            if current.connection_id != connection_id {
                return Err("connection_replaced");
            }
            let peer_device_id = current.peer_device_id.as_str();
            peers
                .get(peer_device_id)
                .map(|peer| peer.sender.clone())
                .ok_or("peer_offline")?
        };
        sender
            .try_send(RelayOutbound::Binary(payload))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "peer_backpressure",
                mpsc::error::TrySendError::Closed(_) => "peer_offline",
            })
    }

    async fn remove(&self, key: &RelayKey, device_id: &str, connection_id: Uuid) {
        let mut sessions = self.sessions.lock().await;
        let Some(peers) = sessions.get_mut(key) else {
            return;
        };
        let peer_device_id = peers.get(device_id).and_then(|peer| {
            (peer.connection_id == connection_id).then(|| peer.peer_device_id.clone())
        });
        if peer_device_id.is_some() {
            peers.remove(device_id);
        }
        if let Some(peer_device_id) = peer_device_id
            && let Some(peer) = peers.get(&peer_device_id)
        {
            let _ = peer.sender.try_send(RelayOutbound::PeerOffline);
        }
        if peers.is_empty() {
            sessions.remove(key);
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayQuery {
    pub(crate) session_id: String,
    pub(crate) device_id: String,
}

pub async fn relay_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RelayQuery>,
) -> Result<Response, AppError> {
    validate_origin(&state, &headers)?;
    let auth = authenticate_websocket(&state, &headers).await?;
    let session_id = normalize_uuid(&query.session_id, "sessionId")?;
    let device_id = normalize_uuid(&query.device_id, "deviceId")?;
    devices::fetch_owned(&state, &auth.user_id, &device_id).await?;
    let peer_device_id = authorize_relay(&state, &auth, &session_id, &device_id).await?;
    Ok(ws
        .protocols(["sanser-relay-v1"])
        .max_message_size(MAX_RELAY_FRAME_BYTES)
        .max_frame_size(MAX_RELAY_FRAME_BYTES)
        .on_upgrade(move |socket| {
            run_relay_socket(socket, state, auth, session_id, device_id, peer_device_id)
        }))
}

pub(crate) async fn authorize_relay(
    state: &AppState,
    auth: &AuthContext,
    session_id: &str,
    device_id: &str,
) -> Result<String, AppError> {
    let row = sqlx::query(
        "SELECT requester_device_id, host_device_id, state, selected_transport \
         FROM connection_sessions WHERE id = $1 AND user_id = $2",
    )
    .bind(session_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    let requester: String = row
        .try_get("requester_device_id")
        .map_err(AppError::from_db)?;
    let host: String = row.try_get("host_device_id").map_err(AppError::from_db)?;
    let session_state: String = row.try_get("state").map_err(AppError::from_db)?;
    let selected_transport: Option<String> = row
        .try_get("selected_transport")
        .map_err(AppError::from_db)?;
    if session_state != "accepted" {
        return Err(AppError::Conflict(
            "relay requires an accepted session".into(),
        ));
    }
    if selected_transport.as_deref() != Some("native") {
        return Err(AppError::Conflict(
            "relay requires an authenticated native session".into(),
        ));
    }
    if device_id == requester {
        Ok(host)
    } else if device_id == host {
        Ok(requester)
    } else {
        Err(AppError::Forbidden)
    }
}

async fn run_relay_socket(
    mut socket: WebSocket,
    state: AppState,
    auth: AuthContext,
    session_id: String,
    device_id: String,
    peer_device_id: String,
) {
    let key = RelayKey {
        user_id: auth.user_id,
        session_id,
    };
    let Ok((connection_id, mut outbound)) = state
        .relay
        .register(key.clone(), device_id.clone(), peer_device_id)
        .await
    else {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            socket.send(Message::Close(Some(CloseFrame {
                code: 1013,
                reason: "relay-capacity-unavailable".into(),
            }))),
        )
        .await;
        return;
    };
    let (mut writer, mut reader) = socket.split();
    let (control_tx, mut control_rx) = mpsc::channel(8);
    // Do not await a slow client's video writes inside the input-reading loop.
    let sending = async {
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let message = tokio::select! {
                outgoing = outbound.recv() => match outgoing {
                    Some(RelayOutbound::Binary(payload)) => Message::Binary(payload.into()),
                    Some(RelayOutbound::PeerReady) => Message::Text("{\"type\":\"relay.peerReady\"}".into()),
                    Some(RelayOutbound::PeerOffline) => Message::Text("{\"type\":\"relay.peerOffline\"}".into()),
                    None => return "relay-outbound-closed",
                },
                control = control_rx.recv() => {
                    let Some(message) = control else { return "relay-control-closed"; };
                    message
                }
                _ = heartbeat.tick() => Message::Ping(Vec::new().into()),
            };
            if writer.send(message).await.is_err() {
                return "relay-write-error";
            }
        }
    };
    let receiving = async {
        let mut byte_window_started = Instant::now();
        let mut byte_window_total = 0_usize;
        while let Some(incoming) = reader.next().await {
            match incoming {
                Ok(Message::Binary(payload)) => {
                    let now = Instant::now();
                    if now.duration_since(byte_window_started) >= Duration::from_secs(1) {
                        byte_window_started = now;
                        byte_window_total = 0;
                    }
                    byte_window_total = byte_window_total.saturating_add(payload.len());
                    if byte_window_total > MAX_RELAY_BYTES_PER_SECOND {
                        return "relay-rate-limit";
                    }
                    match state
                        .relay
                        .forward(&key, &device_id, connection_id, payload.to_vec())
                        .await
                    {
                        Ok(()) => {}
                        Err("connection_replaced") => return "relay-connection-replaced",
                        Err(_) => continue,
                    }
                }
                Ok(Message::Ping(payload)) => {
                    if control_tx.try_send(Message::Pong(payload)).is_err() {
                        return "relay-control-congested";
                    }
                }
                Ok(Message::Close(_)) => return "peer-close",
                Ok(Message::Text(_)) => return "relay-invalid-message",
                Err(_) => return "relay-read-error",
                _ => {}
            }
        }
        "relay-peer-eof"
    };
    let reason = tokio::select! {
        reason = sending => reason,
        reason = receiving => reason,
        () = state.shutdown.cancelled() => "server-restart",
    };
    // Controlled restarts should send a real close frame. Abrupt network/proxy
    // resets still require client recovery; a close write must never hang exit.
    let code = match reason {
        "server-restart" => 1012,
        "relay-rate-limit" | "relay-invalid-message" => 1008,
        _ => 1001,
    };
    let _ = tokio::time::timeout(
        Duration::from_secs(1),
        writer.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
    tracing::info!(reason, "Sanser relay closed");
    state.relay.remove(&key, &device_id, connection_id).await;
}

pub(crate) fn normalize_uuid(value: &str, name: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|value| value.to_string())
        .map_err(|_| AppError::Validation(format!("{name} must be a UUID")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn controlled_restart_sends_close_1012_and_removes_its_relay_registration() {
        use axum::{Router, routing::get};
        use tokio_tungstenite::tungstenite::Message as ClientMessage;
        let database = "postgres://sanser:synthetic@127.0.0.1/sanser_test?sslmode=require";
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy(database)
            .unwrap();
        let state = AppState::new(crate::config::Config::test(database.into()).unwrap(), pool);
        let shutdown = state.shutdown.clone();
        let hub = state.relay.clone();
        let app = Router::new().route(
            "/relay-test",
            get(move |ws: WebSocketUpgrade| {
                let state = state.clone();
                async move {
                    ws.on_upgrade(move |socket| {
                        run_relay_socket(
                            socket,
                            state,
                            AuthContext {
                                user_id: "test-user".into(),
                                auth_session_id: "test-auth".into(),
                                access_digest: "synthetic".into(),
                            },
                            Uuid::new_v4().to_string(),
                            "test-device".into(),
                            "test-peer".into(),
                        )
                    })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let (mut viewer, _) =
            tokio_tungstenite::connect_async(format!("ws://{address}/relay-test"))
                .await
                .unwrap();
        shutdown.cancel();
        let closed = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match viewer.next().await {
                    Some(Ok(ClientMessage::Close(frame))) => break frame.unwrap(),
                    Some(Ok(ClientMessage::Ping(data))) => {
                        viewer.send(ClientMessage::Pong(data)).await.unwrap()
                    }
                    other => panic!("restart omitted its WebSocket closing handshake: {other:?}"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            closed.code,
            tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Restart
        );
        assert_eq!(closed.reason.as_str(), "server-restart");
        let _ = viewer.flush().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !hub.sessions.lock().await.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn relay_hub_forwards_only_between_the_session_peers() {
        let hub = RelayHub::default();
        let key = RelayKey {
            user_id: "user".into(),
            session_id: Uuid::new_v4().to_string(),
        };
        let (left_id, mut left) = hub
            .register(key.clone(), "left".into(), "right".into())
            .await
            .unwrap();
        let (_, mut right) = hub
            .register(key.clone(), "right".into(), "left".into())
            .await
            .unwrap();
        assert!(matches!(left.recv().await, Some(RelayOutbound::PeerReady)));
        assert!(matches!(right.recv().await, Some(RelayOutbound::PeerReady)));

        hub.forward(&key, "left", left_id, vec![1, 2, 3])
            .await
            .unwrap();
        assert!(matches!(
            right.recv().await,
            Some(RelayOutbound::Binary(payload)) if payload == vec![1, 2, 3]
        ));
    }
}
