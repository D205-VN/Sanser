//! UDP media relay; WSS authorizes allocations and reports peer lifecycle only.

use crate::{
    auth::authenticate_websocket,
    error::AppError,
    relay::{RelayQuery, authorize_relay, normalize_uuid},
    routes::devices,
    state::AppState,
    websocket::validate_origin,
};
use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use rand::{RngCore, rngs::OsRng};
use sanser_network::relay_datagram::{Kind, MAX_PACKET_BYTES, Offer, PacketAuth};
use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, Instant},
};
use tokio::{
    net::UdpSocket,
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;

type AllocationId = [u8; 16];
type Slot = (String, String, String); // user, session, device
const LEASE: Duration = Duration::from_secs(60);

struct Allocation {
    slot: Slot,
    peer_device_id: String,
    auth: PacketAuth,
    endpoint: Option<SocketAddr>,
    challenge: Option<(SocketAddr, [u8; 16], Instant)>,
    expires: Instant,
    window: Instant,
    bytes: usize,
    events: mpsc::Sender<&'static str>,
}

#[derive(Default)]
struct Allocations {
    entries: HashMap<AllocationId, Allocation>,
    slots: HashMap<Slot, AllocationId>,
}

#[derive(Default)]
pub struct UdpRelayHub {
    allocations: Mutex<Allocations>,
}

impl UdpRelayHub {
    async fn register(
        &self,
        slot: Slot,
        peer: String,
        address: String,
    ) -> Result<(Offer, mpsc::Receiver<&'static str>), AppError> {
        let mut state = self.allocations.lock().await;
        if state.entries.len() >= 8192 {
            return Err(AppError::Unavailable);
        }
        if let Some(old) = state.slots.remove(&slot) {
            if let Some(old) = state.entries.remove(&old) {
                let _ = old.events.try_send("relay.peerOffline");
                let peer_slot = (old.slot.0, old.slot.1, old.peer_device_id);
                if let Some(peer) = state
                    .slots
                    .get(&peer_slot)
                    .and_then(|id| state.entries.get(id))
                {
                    let _ = peer.events.try_send("relay.peerOffline");
                }
            }
        }
        let id = *uuid::Uuid::new_v4().as_bytes();
        let mut key = [0; 32];
        OsRng.fill_bytes(&mut key);
        let (events, receiver) = mpsc::channel(8);
        let now = Instant::now();
        state.slots.insert(slot.clone(), id);
        state.entries.insert(
            id,
            Allocation {
                slot,
                peer_device_id: peer,
                auth: PacketAuth::new(id, key),
                endpoint: None,
                challenge: None,
                expires: now + LEASE,
                window: now,
                bytes: 0,
                events,
            },
        );
        Ok((
            Offer {
                allocation_id: id,
                key,
                address,
            },
            receiver,
        ))
    }

    async fn remove(&self, id: AllocationId) {
        let mut state = self.allocations.lock().await;
        if let Some(entry) = state.entries.remove(&id) {
            if state.slots.get(&entry.slot) == Some(&id) {
                state.slots.remove(&entry.slot);
            }
            let peer_slot = (entry.slot.0, entry.slot.1, entry.peer_device_id);
            if let Some(peer) = state
                .slots
                .get(&peer_slot)
                .and_then(|peer| state.entries.get(peer))
            {
                let _ = peer.events.try_send("relay.peerOffline");
            }
        }
    }

    async fn refresh(&self, id: AllocationId) {
        if let Some(entry) = self.allocations.lock().await.entries.get_mut(&id) {
            entry.expires = Instant::now() + LEASE;
        }
    }

    async fn process(&self, packet: &[u8], source: SocketAddr) -> Option<(Vec<u8>, SocketAddr)> {
        let id = PacketAuth::allocation_id(packet)?;
        let mut state = self.allocations.lock().await;
        let now = Instant::now();
        let entry = state.entries.get_mut(&id)?;
        if entry.expires <= now {
            return None;
        }
        let (kind, payload) = entry.auth.decode(packet, false).ok()?;
        if now.duration_since(entry.window) >= Duration::from_secs(1) {
            entry.window = now;
            entry.bytes = 0;
        }
        entry.bytes = entry.bytes.saturating_add(packet.len());
        if entry.bytes > 32 * 1024 * 1024 {
            return None;
        }
        if kind == Kind::Register && payload.len() == 16 {
            if entry.endpoint.is_some_and(|address| address != source) {
                return None;
            }
            let cookie = match entry.challenge {
                Some((address, cookie, deadline)) if address == source && deadline > now => cookie,
                _ => {
                    let mut cookie = [0; 16];
                    OsRng.fill_bytes(&mut cookie);
                    entry.challenge = Some((source, cookie, now + Duration::from_secs(5)));
                    cookie
                }
            };
            return Some((entry.auth.encode(Kind::Challenge, &cookie).ok()?, source));
        }
        if kind == Kind::Confirm {
            let (address, cookie, deadline) = entry.challenge?;
            if address != source || deadline <= now || payload != cookie {
                return None;
            }
            entry.endpoint = Some(source);
            let reply = entry.auth.encode(Kind::Confirmed, &[]).ok()?;
            let peer_slot = (
                entry.slot.0.clone(),
                entry.slot.1.clone(),
                entry.peer_device_id.clone(),
            );
            let device = entry.slot.2.clone();
            let events = entry.events.clone();
            if let Some(peer) = state
                .slots
                .get(&peer_slot)
                .and_then(|other| state.entries.get(other))
            {
                if peer.peer_device_id == device && peer.endpoint.is_some() && peer.expires > now {
                    let _ = peer.events.try_send("relay.udpReady");
                    let _ = events.try_send("relay.udpReady");
                }
            }
            return Some((reply, source));
        }
        if entry.endpoint != Some(source) {
            return None;
        }
        if kind == Kind::Keepalive && payload.is_empty() {
            return Some((entry.auth.encode(Kind::Alive, &[]).ok()?, source));
        }
        if kind != Kind::Data || payload.is_empty() {
            return None;
        }
        let peer_slot = (
            entry.slot.0.clone(),
            entry.slot.1.clone(),
            entry.peer_device_id.clone(),
        );
        let device = entry.slot.2.clone();
        let peer_id = *state.slots.get(&peer_slot)?;
        let peer = state.entries.get_mut(&peer_id)?;
        if peer.peer_device_id != device || peer.expires <= now {
            return None;
        }
        Some((
            peer.auth.encode(Kind::Forward, payload).ok()?,
            peer.endpoint?,
        ))
    }

    pub async fn run(&self, socket: UdpSocket, cancellation: CancellationToken) {
        let mut buffer = vec![0; MAX_PACKET_BYTES + 1];
        loop {
            let received = tokio::select! {
                _ = cancellation.cancelled() => break,
                received = socket.recv_from(&mut buffer) => received,
            };
            let Ok((length, source)) = received else {
                continue;
            };
            if let Some((packet, target)) = self.process(&buffer[..length], source).await {
                // No unbounded media queue and no reliable media retransmission.
                // A saturated socket drops a datagram; native repair/feedback
                // handles it without delaying subsequent input behind TCP data.
                let _ = socket.try_send_to(&packet, target);
            }
        }
    }
}

async fn send_control(
    writer: &mut futures_util::stream::SplitSink<axum::extract::ws::WebSocket, Message>,
    message: Message,
) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(5), writer.send(message)).await,
        Ok(Ok(()))
    )
}

pub async fn relay_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RelayQuery>,
) -> Result<Response, AppError> {
    let address = state
        .config
        .udp_relay_public
        .clone()
        .ok_or(AppError::Unavailable)?;
    validate_origin(&state, &headers)?;
    let auth = authenticate_websocket(&state, &headers).await?;
    let session_id = normalize_uuid(&query.session_id, "sessionId")?;
    let device_id = normalize_uuid(&query.device_id, "deviceId")?;
    devices::fetch_owned(&state, &auth.user_id, &device_id).await?;
    let peer = authorize_relay(&state, &auth, &session_id, &device_id).await?;
    Ok(ws.protocols(["sanser-relay-v1"]).max_message_size(4096).max_frame_size(4096).on_upgrade(move |socket| async move {
        let Ok((offer, mut events)) = state.udp_relay.register((auth.user_id.clone(), session_id.clone(), device_id.clone()), peer, address).await else { return; };
        let id = offer.allocation_id;
        let (mut writer, mut reader) = socket.split();
        if let Ok(message) = serde_json::to_string(&serde_json::json!({"type":"relay.udpOffer", "offer":offer})) {
            if send_control(&mut writer, Message::Text(message.into())).await {
                let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
                let mut last_control_seen = Instant::now();
                loop {
                    tokio::select! {
                        event = events.recv() => {
                            let Some(event) = event else { break; };
                            if !send_control(&mut writer, Message::Text(serde_json::json!({"type":event}).to_string().into())).await || event == "relay.peerOffline" { break; }
                        }
                        _ = heartbeat.tick() => {
                            if last_control_seen.elapsed() > Duration::from_secs(45) { break; }
                            if authorize_relay(&state, &auth, &session_id, &device_id).await.is_err() { break; }
                            state.udp_relay.refresh(id).await;
                            if !send_control(&mut writer, Message::Ping(Vec::new().into())).await { break; }
                        }
                        message = reader.next() => match message {
                            Some(Ok(Message::Ping(data))) => { if !send_control(&mut writer, Message::Pong(data)).await { break; } }
                            Some(Ok(Message::Pong(_))) => { last_control_seen = Instant::now(); },
                            _ => break, // This channel never carries media.
                        }
                    }
                }
            }
        }
        state.udp_relay.remove(id).await;
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    async fn allocate(
        hub: &UdpRelayHub,
        user: &str,
        session: &str,
        device: &str,
        peer: &str,
    ) -> (Offer, PacketAuth, mpsc::Receiver<&'static str>) {
        let (offer, events) = hub
            .register(
                (user.into(), session.into(), device.into()),
                peer.into(),
                "127.0.0.1:12345".into(),
            )
            .await
            .unwrap();
        let auth = PacketAuth::new(offer.allocation_id, offer.key);
        (offer, auth, events)
    }

    async fn confirm(hub: &UdpRelayHub, auth: &mut PacketAuth, source: SocketAddr) {
        let register = auth.encode(Kind::Register, &[7; 16]).unwrap();
        let (challenge, target) = hub.process(&register, source).await.unwrap();
        assert_eq!(target, source);
        assert_eq!(
            challenge.len(),
            register.len(),
            "unverified address must not amplify traffic"
        );
        let (kind, cookie) = auth.decode(&challenge, true).unwrap();
        assert_eq!(kind, Kind::Challenge);
        let packet = auth.encode(Kind::Confirm, cookie).unwrap();
        let (reply, _) = hub.process(&packet, source).await.unwrap();
        assert_eq!(auth.decode(&reply, true).unwrap().0, Kind::Confirmed);
    }

    #[tokio::test]
    async fn requires_address_proof_and_rejects_forgery_replay_and_wrong_source() {
        let hub = UdpRelayHub::default();
        let (_, mut a, mut a_events) = allocate(&hub, "user", "session", "a", "b").await;
        let (_, mut b, mut b_events) = allocate(&hub, "user", "session", "b", "a").await;
        let sa = "127.0.0.1:1111".parse().unwrap();
        let sb = "127.0.0.1:2222".parse().unwrap();
        assert!(
            hub.process(&a.encode(Kind::Data, b"unconfirmed").unwrap(), sa)
                .await
                .is_none()
        );
        confirm(&hub, &mut a, sa).await;
        assert!(a_events.try_recv().is_err());
        confirm(&hub, &mut b, sb).await;
        assert_eq!(a_events.try_recv().unwrap(), "relay.udpReady");
        assert_eq!(b_events.try_recv().unwrap(), "relay.udpReady");
        let first = a.encode(Kind::Data, b"video").unwrap();
        let mut forged = first.clone();
        forged[32] ^= 1;
        assert!(hub.process(&forged, sa).await.is_none());
        let (forward, target) = hub.process(&first, sa).await.unwrap();
        assert_eq!(target, sb);
        assert_eq!(
            b.decode(&forward, true).unwrap(),
            (Kind::Forward, b"video".as_slice())
        );
        assert!(hub.process(&first, sa).await.is_none());
        assert!(
            hub.process(&a.encode(Kind::Data, b"wrong address").unwrap(), sb)
                .await
                .is_none()
        );
        assert!(
            hub.process(&a.encode(Kind::Register, &[1; 16]).unwrap(), sb)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn allocations_are_scoped_expire_and_cannot_remove_replacements() {
        let hub = UdpRelayHub::default();
        let (old, mut a, mut events) = allocate(&hub, "user", "session", "a", "b").await;
        let (_, mut stranger, _) = allocate(&hub, "other-user", "session", "b", "a").await;
        let sa = "127.0.0.1:1111".parse().unwrap();
        let sb = "127.0.0.1:2222".parse().unwrap();
        confirm(&hub, &mut a, sa).await;
        confirm(&hub, &mut stranger, sb).await;
        assert!(
            hub.process(&a.encode(Kind::Data, b"private").unwrap(), sa)
                .await
                .is_none()
        );
        let (new, mut replacement, _) = allocate(&hub, "user", "session", "a", "b").await;
        assert_eq!(events.try_recv().unwrap(), "relay.peerOffline");
        hub.remove(old.allocation_id).await;
        confirm(&hub, &mut replacement, sa).await;
        assert!(
            hub.process(&a.encode(Kind::Keepalive, &[]).unwrap(), sa)
                .await
                .is_none()
        );
        hub.allocations
            .lock()
            .await
            .entries
            .get_mut(&new.allocation_id)
            .unwrap()
            .expires = Instant::now();
        assert!(
            hub.process(&replacement.encode(Kind::Keepalive, &[]).unwrap(), sa)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn actual_udp_socket_forwards_after_loss_and_reordering_without_waiting() {
        let hub = std::sync::Arc::new(UdpRelayHub::default());
        let (_, mut a, _) = allocate(&hub, "user", "session", "a", "b").await;
        let (_, mut b, _) = allocate(&hub, "user", "session", "b", "a").await;
        let left = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        confirm(&hub, &mut a, left.local_addr().unwrap()).await;
        confirm(&hub, &mut b, right.local_addr().unwrap()).await;
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let cancellation = CancellationToken::new();
        let worker = {
            let hub = hub.clone();
            let stop = cancellation.clone();
            tokio::spawn(async move { hub.run(socket, stop).await })
        };
        let _lost = a.encode(Kind::Data, b"lost-video").unwrap();
        let delayed = a.encode(Kind::Data, b"delayed-video").unwrap();
        let input = a.encode(Kind::Data, b"key-up").unwrap();
        let mut buffer = [0; 1024];
        for (packet, expected) in [
            (input, b"key-up".as_slice()),
            (delayed, b"delayed-video".as_slice()),
        ] {
            left.send_to(&packet, address).await.unwrap();
            let (length, _) =
                tokio::time::timeout(Duration::from_secs(1), right.recv_from(&mut buffer))
                    .await
                    .expect("missing media must not hold later input")
                    .unwrap();
            assert_eq!(b.decode(&buffer[..length], true).unwrap().1, expected);
        }
        cancellation.cancel();
        worker.await.unwrap();
    }
}
