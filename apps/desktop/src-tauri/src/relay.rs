#[path = "udp_relay.rs"]
mod udp;

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use futures_util::{SinkExt, StreamExt};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use tokio::sync::oneshot;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header},
    },
};
use zeroize::Zeroize;

use crate::{
    error::DesktopError,
    models::{EngineKind, LaunchEngineRequest},
};

const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(25);
const RELAY_MAGIC: [u8; 4] = *b"SNR1";
const RELAY_HEADER_BYTES: usize = 44;
const RELAY_TAG_BYTES: usize = 16;
const RELAY_MAX_FRAME_BYTES: usize = 64 * 1024;
const RELAY_MAX_DATAGRAM_BYTES: usize =
    RELAY_MAX_FRAME_BYTES - RELAY_HEADER_BYTES - RELAY_TAG_BYTES;

struct RelayBridge {
    transport: &'static str,
    id: uuid::Uuid,
    session_id: uuid::Uuid,
    engine_port: u16,
    proxy_port: u16,
    stop: oneshot::Sender<()>,
}

#[derive(Default)]
struct RelayState {
    bridges: HashMap<(EngineKind, uuid::Uuid), RelayBridge>,
    pending: HashMap<(EngineKind, uuid::Uuid), uuid::Uuid>,
}

struct RelayReservation {
    state: Arc<Mutex<RelayState>>,
    key: (EngineKind, uuid::Uuid),
    id: uuid::Uuid,
}

impl RelayReservation {
    fn install(&self, bridge: RelayBridge) -> Result<(), DesktopError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("relay state is unavailable".into()))?;
        if state.pending.get(&self.key) != Some(&self.id) {
            return Err(DesktopError::Process(
                "relay negotiation was cancelled or replaced".into(),
            ));
        }
        state.pending.remove(&self.key);
        state.bridges.insert(self.key, bridge);
        Ok(())
    }
}

impl Drop for RelayReservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock()
            && state.pending.get(&self.key) == Some(&self.id)
        {
            state.pending.remove(&self.key);
        }
    }
}

#[derive(Default)]
pub struct RelayManager {
    state: Arc<Mutex<RelayState>>,
    failures: Arc<Mutex<HashMap<(EngineKind, uuid::Uuid), String>>>,
}

impl RelayManager {
    pub fn transport(&self, kind: EngineKind, session: &str) -> Option<String> {
        let session = uuid::Uuid::parse_str(session).ok()?;
        self.state
            .lock()
            .ok()?
            .bridges
            .get(&(kind, session))
            .map(|bridge| bridge.transport.to_owned())
    }

    fn stop(&self, kind: EngineKind) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.retain(|key, _| key.0 != kind);
        }
        if let Ok(mut state) = self.state.lock() {
            state.pending.retain(|key, _| key.0 != kind);
            let keys = state
                .bridges
                .keys()
                .filter(|key| key.0 == kind)
                .copied()
                .collect::<Vec<_>>();
            for key in keys {
                if let Some(bridge) = state.bridges.remove(&key) {
                    let _ = bridge.stop.send(());
                }
            }
        }
    }
    pub fn stop_peer(&self, kind: EngineKind, session: uuid::Uuid) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.remove(&(kind, session));
        }
        if let Ok(mut state) = self.state.lock() {
            state.pending.remove(&(kind, session));
            if let Some(bridge) = state.bridges.remove(&(kind, session)) {
                let _ = bridge.stop.send(());
            }
        }
    }
    pub fn stop_for_engine(&self, kind: EngineKind) {
        self.stop(kind);
    }
    pub fn take_peer_failure(&self, kind: EngineKind, session: uuid::Uuid) -> Option<String> {
        self.failures.lock().ok()?.remove(&(kind, session))
    }
    pub fn take_failure(&self, kind: EngineKind) -> Option<String> {
        let mut failures = self.failures.lock().ok()?;
        let key = failures.keys().find(|key| key.0 == kind).copied()?;
        failures.remove(&key)
    }

    fn reserve(
        &self,
        kind: EngineKind,
        session: uuid::Uuid,
        sharing: bool,
    ) -> Result<RelayReservation, DesktopError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("relay state is unavailable".into()))?;
        let key = (kind, session);
        let occupied = state
            .bridges
            .keys()
            .chain(state.pending.keys())
            .filter(|item| item.0 == kind && **item != key)
            .count();
        if sharing && occupied >= 3 {
            return Err(DesktopError::Process(
                "sharing relay viewer limit reached".into(),
            ));
        }
        if !sharing {
            state.pending.retain(|item, _| item.0 != kind);
            let keys = state
                .bridges
                .keys()
                .filter(|item| item.0 == kind)
                .copied()
                .collect::<Vec<_>>();
            for old in keys {
                if let Some(bridge) = state.bridges.remove(&old) {
                    let _ = bridge.stop.send(());
                }
            }
        } else if let Some(bridge) = state.bridges.remove(&key) {
            let _ = bridge.stop.send(());
        }
        let id = uuid::Uuid::new_v4();
        state.pending.insert(key, id);
        Ok(RelayReservation {
            state: Arc::clone(&self.state),
            key,
            id,
        })
    }

    pub fn verify_launch(&self, request: &LaunchEngineRequest) -> Result<(), DesktopError> {
        if !request.relay {
            return Ok(());
        }
        let session_id = request
            .session_id
            .as_deref()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .ok_or_else(|| DesktopError::InvalidRequest("relay sessionId must be a UUID".into()))?;
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("relay state is unavailable".into()))?;
        let bridge = state
            .bridges
            .get(&(request.kind, session_id))
            .ok_or_else(|| {
                DesktopError::Process("the authenticated relay bridge is not ready".into())
            })?;
        let engine_port = match request.kind {
            EngineKind::Host => request.udp_bind_port,
            EngineKind::Client => request.port,
            EngineKind::LocalServer => None,
        };
        if bridge.session_id != session_id || engine_port != Some(bridge.engine_port) {
            return Err(DesktopError::InvalidRequest(
                "native engine does not match the prepared relay bridge".into(),
            ));
        }
        let expected_proxy = format!("127.0.0.1:{}", bridge.proxy_port);
        let proxy_matches = match request.kind {
            EngineKind::Host => {
                request.address.as_deref() == Some("127.0.0.1")
                    && request.port == Some(bridge.proxy_port)
            }
            EngineKind::Client => request.udp_connect.as_deref() == Some(expected_proxy.as_str()),
            EngineKind::LocalServer => false,
        };
        if !proxy_matches {
            return Err(DesktopError::InvalidRequest(
                "native engine target does not match the local relay bridge".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayStartRequest {
    kind: EngineKind,
    server_url: String,
    session_id: String,
    device_id: String,
    peer_device_id: String,
    access_token: String,
    session_credential: String,
    preferred_engine_port: Option<u16>,
    #[serde(default)]
    sharing_id: Option<String>,
}

impl Drop for RelayStartRequest {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.session_credential.zeroize();
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStartResult {
    engine_port: u16,
    proxy_port: u16,
    transport: &'static str,
}

#[tauri::command]
#[allow(clippy::too_many_lines)]
pub async fn relay_start(
    manager: State<'_, RelayManager>,
    mut request: RelayStartRequest,
) -> Result<RelayStartResult, DesktopError> {
    if request.kind == EngineKind::LocalServer {
        return Err(DesktopError::InvalidRequest(
            "local server cannot use the media relay".into(),
        ));
    }
    let session_id = uuid::Uuid::parse_str(request.session_id.trim())
        .map_err(|_| DesktopError::InvalidRequest("relay sessionId must be a UUID".into()))?;
    let device_id = uuid::Uuid::parse_str(request.device_id.trim())
        .map_err(|_| DesktopError::InvalidRequest("relay deviceId must be a UUID".into()))?;
    let peer_device_id = uuid::Uuid::parse_str(request.peer_device_id.trim())
        .map_err(|_| DesktopError::InvalidRequest("relay peerDeviceId must be a UUID".into()))?;
    if device_id == peer_device_id {
        return Err(DesktopError::InvalidRequest(
            "relay peers must be different devices".into(),
        ));
    }
    if request.access_token.len() < 32
        || request.access_token.len() > 2_048
        || !request.access_token.is_ascii()
        || request.access_token.chars().any(char::is_whitespace)
    {
        return Err(DesktopError::InvalidRequest(
            "relay access token has an invalid format".into(),
        ));
    }
    if !(32..=512).contains(&request.session_credential.len())
        || !request.session_credential.is_ascii()
        || request.session_credential.chars().any(char::is_whitespace)
    {
        return Err(DesktopError::InvalidRequest(
            "relay session credential has an invalid format".into(),
        ));
    }
    if request
        .preferred_engine_port
        .is_some_and(|port| port < 1_024)
    {
        return Err(DesktopError::InvalidRequest(
            "relay engine port must be between 1024 and 65535".into(),
        ));
    }

    if let Some(group) = &request.sharing_id {
        uuid::Uuid::parse_str(group)
            .map_err(|_| DesktopError::InvalidRequest("sharingId must be a UUID".into()))?;
        if request.kind != EngineKind::Host {
            return Err(DesktopError::InvalidRequest(
                "only a host uses multiple sharing relay bridges".into(),
            ));
        }
    }
    let reservation = manager.reserve(request.kind, session_id, request.sharing_id.is_some())?;
    if let Ok(mut failures) = manager.failures.lock() {
        if request.sharing_id.is_some() {
            failures.remove(&(request.kind, session_id));
        } else {
            failures.retain(|key, _| key.0 != request.kind);
        }
    }
    let engine_reservation = tokio::net::UdpSocket::bind((
        Ipv4Addr::UNSPECIFIED,
        request.preferred_engine_port.unwrap_or(0),
    ))
    .await
    .map_err(|error| {
        DesktopError::Process(format!("unable to reserve relay engine port: {error}"))
    })?;
    let engine_port = engine_reservation
        .local_addr()
        .map_err(|error| DesktopError::Process(error.to_string()))?
        .port();
    let proxy_socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            DesktopError::Process(format!("unable to open local relay bridge: {error}"))
        })?;
    let proxy_port = proxy_socket
        .local_addr()
        .map_err(|error| DesktopError::Process(error.to_string()))?
        .port();

    let endpoint = relay_endpoint(&request.server_url, session_id, device_id)?;
    let cipher = RelayCipher::new(
        session_id,
        device_id,
        peer_device_id,
        &request.session_credential,
    )?;
    request.session_credential.zeroize();
    let mut websocket_request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|error| DesktopError::Process(format!("invalid relay request: {error}")))?;
    let protocols =
        HeaderValue::from_str(&format!("sanser-relay-v1, bearer.{}", request.access_token))
            .map_err(|_| {
                DesktopError::InvalidRequest("relay token cannot be sent securely".into())
            })?;
    request.access_token.zeroize();
    websocket_request
        .headers_mut()
        .insert(header::SEC_WEBSOCKET_PROTOCOL, protocols);
    websocket_request.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static(if cfg!(target_os = "windows") {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }),
    );

    let udp = match udp::connect(websocket_request.clone()).await {
        Ok(transport) => Some(transport),
        Err(reason) => {
            eprintln!("SANSER_RELAY udpUnavailable={reason}; trying WSS fallback");
            None
        }
    };
    let websocket = if udp.is_none() {
        let (mut websocket, response) = tokio::time::timeout(
            RELAY_CONNECT_TIMEOUT,
            connect_async_with_config(websocket_request, None, true),
        )
        .await
        .map_err(|_| DesktopError::Process("relay WebSocket connection timed out".into()))?
        .map_err(|error| {
            DesktopError::Process(format!("relay WebSocket connection failed: {error}"))
        })?;
        if response
            .headers()
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok())
            != Some("sanser-relay-v1")
        {
            return Err(DesktopError::Process(
                "relay server did not negotiate the secure relay protocol".into(),
            ));
        }

        tokio::time::timeout(RELAY_CONNECT_TIMEOUT, async {
            loop {
                match websocket.next().await {
                    Some(Ok(Message::Text(message)))
                        if message.as_str().contains("relay.peerReady") =>
                    {
                        return Ok::<(), DesktopError>(());
                    }
                    Some(Ok(Message::Ping(payload))) => websocket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|error| DesktopError::Process(error.to_string()))?,
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(DesktopError::Process(
                            "relay closed while waiting for the peer".into(),
                        ));
                    }
                    Some(Err(error)) => {
                        return Err(DesktopError::Process(format!(
                            "relay failed while waiting for the peer: {error}"
                        )));
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| DesktopError::Process("relay peer did not become ready in time".into()))??;

        Some(websocket)
    } else {
        None
    };
    let transport_name = if udp.is_some() { "udp" } else { "wss" };
    eprintln!("SANSER_RELAY transport={transport_name}");

    // Release the engine reservation only after both relay peers are present;
    // the native sidecar binds this port immediately after the command returns.
    drop(engine_reservation);
    let engine_endpoint = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), engine_port);
    let (stop_tx, stop_rx) = oneshot::channel();
    let bridge_id = uuid::Uuid::new_v4();
    let bridge = RelayBridge {
        transport: transport_name,
        id: bridge_id,
        session_id,
        engine_port,
        proxy_port,
        stop: stop_tx,
    };
    reservation.install(bridge)?;
    let bridges = Arc::clone(&manager.state);
    let failures = Arc::clone(&manager.failures);
    let kind = request.kind;
    tauri::async_runtime::spawn(async move {
        let failure = if let Some(udp) = udp {
            udp::run(udp, proxy_socket, engine_endpoint, cipher, stop_rx).await
        } else if let Some(websocket) = websocket {
            run_bridge(proxy_socket, engine_endpoint, websocket, cipher, stop_rx).await
        } else {
            Some("relay negotiation did not select a transport".into())
        };
        if let Ok(mut active) = bridges.lock()
            && active
                .bridges
                .get(&(kind, session_id))
                .is_some_and(|bridge| bridge.id == bridge_id)
        {
            active.bridges.remove(&(kind, session_id));
            if let Some(failure) = failure
                && let Ok(mut recorded) = failures.lock()
            {
                recorded.insert((kind, session_id), failure);
            }
        }
    });
    Ok(RelayStartResult {
        engine_port,
        proxy_port,
        transport: transport_name,
    })
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn relay_stop(
    manager: State<'_, RelayManager>,
    kind: EngineKind,
    session_id: Option<String>,
) -> Result<(), DesktopError> {
    if let Some(session) = session_id {
        let session = uuid::Uuid::parse_str(&session)
            .map_err(|_| DesktopError::InvalidRequest("sessionId must be a UUID".into()))?;
        manager.stop_peer(kind, session);
    } else {
        manager.stop(kind);
    }
    Ok(())
}
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn relay_peer_failure(
    manager: State<'_, RelayManager>,
    kind: EngineKind,
    session_id: String,
) -> Result<Option<String>, DesktopError> {
    let session = uuid::Uuid::parse_str(&session_id)
        .map_err(|_| DesktopError::InvalidRequest("sessionId must be a UUID".into()))?;
    Ok(manager.take_peer_failure(kind, session))
}

fn relay_endpoint(
    server_url: &str,
    session_id: uuid::Uuid,
    device_id: uuid::Uuid,
) -> Result<url::Url, DesktopError> {
    let mut url = url::Url::parse(server_url.trim())
        .map_err(|_| DesktopError::InvalidRequest("relay server URL is invalid".into()))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    match url.scheme() {
        "https" => url
            .set_scheme("wss")
            .map_err(|()| DesktopError::InvalidRequest("relay URL scheme is invalid".into()))?,
        "http" if loopback => url
            .set_scheme("ws")
            .map_err(|()| DesktopError::InvalidRequest("relay URL scheme is invalid".into()))?,
        _ => {
            return Err(DesktopError::InvalidRequest(
                "relay requires HTTPS, or HTTP only on loopback".into(),
            ));
        }
    }
    url.set_path(&format!(
        "{}/api/v2/relay",
        url.path().trim_end_matches('/')
    ));
    url.set_query(Some(
        &url::form_urlencoded::Serializer::new(String::new())
            .append_pair("sessionId", &session_id.to_string())
            .append_pair("deviceId", &device_id.to_string())
            .finish(),
    ));
    Ok(url)
}

async fn run_bridge<S>(
    socket: tokio::net::UdpSocket,
    engine_endpoint: SocketAddr,
    websocket: tokio_tungstenite::WebSocketStream<S>,
    cipher: RelayCipher,
    mut stop: oneshot::Receiver<()>,
) -> Option<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // Poll both directions independently: a congested video upload must not
    // prevent mouse/key packets arriving, or prevent the user disconnecting.
    let (mut writer, mut reader) = websocket.split();
    let cipher = Mutex::new(cipher);
    let (control_tx, mut control_rx) = tokio::sync::mpsc::channel(8);
    let startup_deadline = tokio::time::Instant::now() + RELAY_CONNECT_TIMEOUT;
    let sending = async {
        let mut buffer = vec![0_u8; RELAY_MAX_DATAGRAM_BYTES];
        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(15),
            Duration::from_secs(15),
        );
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut engine_ready = false;
        let engine_startup = tokio::time::sleep_until(startup_deadline);
        tokio::pin!(engine_startup);
        loop {
            let message = tokio::select! {
                () = &mut engine_startup, if !engine_ready => {
                    return Some("local native engine did not start sending UDP packets within 25 seconds; retry the session".into());
                }
                message = control_rx.recv() => {
                    message?
                }
                received = socket.recv_from(&mut buffer) => {
                    let (length, source) = match received {
                        Ok(packet) => packet,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) if tokio::time::Instant::now() < startup_deadline && udp_port_not_ready(&error) => continue,
                        Err(error) => return Some(format!("local relay UDP bridge stopped receiving packets: {error}")),
                    };
                    if source != engine_endpoint { continue; }
                    engine_ready = true;
                    let encrypted = {
                        let Ok(mut cipher) = cipher.lock() else { return Some("relay cipher lock failed".into()); };
                        cipher.encrypt(&buffer[..length])
                    };
                    let Ok(encrypted) = encrypted else { return Some("local relay packet encryption failed".into()); };
                    Message::Binary(encrypted.into())
                }
                _ = heartbeat.tick() => Message::Ping(Vec::new().into()),
            };
            if let Err(error) = writer.send(message).await {
                return Some(format!("relay stopped while sending media: {error}"));
            }
        }
    };
    let receiving = async {
        let mut consecutive_decrypt_failures = 0_u8;
        while let Some(incoming) = reader.next().await {
            match incoming {
                Ok(Message::Binary(payload)) => {
                    let decrypted = {
                        let Ok(mut cipher) = cipher.lock() else {
                            return Some("relay cipher lock failed".into());
                        };
                        cipher.decrypt(&payload)
                    };
                    let decrypted = if let Ok(decrypted) = decrypted {
                        consecutive_decrypt_failures = 0;
                        decrypted
                    } else {
                        consecutive_decrypt_failures =
                            consecutive_decrypt_failures.saturating_add(1);
                        if consecutive_decrypt_failures >= 32 {
                            return Some("relay frame authentication repeatedly failed; refresh the accepted session on both devices".into());
                        }
                        continue;
                    };
                    if let Err(error) = socket.send_to(&decrypted, engine_endpoint).await {
                        if tokio::time::Instant::now() < startup_deadline
                            && udp_port_not_ready(&error)
                        {
                            continue;
                        }
                        return Some(format!(
                            "local relay UDP bridge could not deliver a packet to the native engine: {error}"
                        ));
                    }
                }
                Ok(Message::Ping(payload)) => {
                    // The bounded control queue must not stall incoming input.
                    if control_tx.try_send(Message::Pong(payload)).is_err() {
                        return Some("relay heartbeat response is congested".into());
                    }
                }
                Ok(Message::Text(message)) if message.as_str().contains("relay.peerOffline") => {
                    return Some("relay peer went offline; retry the accepted session".into());
                }
                Ok(Message::Close(_)) => break,
                Err(error) => return Some(format!("relay connection failed: {error}")),
                _ => {}
            }
        }
        Some("relay connection closed; retry the accepted session".into())
    };
    tokio::select! {
        _ = &mut stop => None,
        result = sending => result,
        result = receiving => result,
    }
}

fn udp_port_not_ready(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionRefused
    )
}

struct RelayCipher {
    cipher: XChaCha20Poly1305,
    session_id: [u8; 16],
    local_device_id: [u8; 16],
    peer_device_id: [u8; 16],
    send_nonce_prefix: [u8; 16],
    send_sequence: u64,
    peer_nonce_prefix: Option<[u8; 16]>,
    receive_window: sanser_network::relay_datagram::ReplayWindow,
}

impl RelayCipher {
    fn new(
        session_id: uuid::Uuid,
        local_device_id: uuid::Uuid,
        peer_device_id: uuid::Uuid,
        session_credential: &str,
    ) -> Result<Self, DesktopError> {
        let mut hasher = Sha256::new();
        hasher.update(b"sanser-relay-frame-key-v1\0");
        hasher.update(session_id.as_bytes());
        hasher.update(session_credential.as_bytes());
        let mut key: [u8; 32] = hasher.finalize().into();
        let cipher = XChaCha20Poly1305::new_from_slice(&key)
            .map_err(|_| DesktopError::Process("relay cipher setup failed".into()))?;
        key.zeroize();
        let mut send_nonce_prefix = [0_u8; 16];
        OsRng.fill_bytes(&mut send_nonce_prefix);
        Ok(Self {
            cipher,
            session_id: *session_id.as_bytes(),
            local_device_id: *local_device_id.as_bytes(),
            peer_device_id: *peer_device_id.as_bytes(),
            send_nonce_prefix,
            send_sequence: 0,
            peer_nonce_prefix: None,
            receive_window: sanser_network::relay_datagram::ReplayWindow::default(),
        })
    }

    fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, DesktopError> {
        if plaintext.len() > RELAY_MAX_DATAGRAM_BYTES || self.send_sequence == u64::MAX {
            return Err(DesktopError::Process(
                "relay datagram exceeds the secure frame limit".into(),
            ));
        }
        let sequence = self.send_sequence;
        self.send_sequence = self.send_sequence.saturating_add(1);
        let mut header = [0_u8; RELAY_HEADER_BYTES];
        header[..4].copy_from_slice(&RELAY_MAGIC);
        header[4..20].copy_from_slice(&self.local_device_id);
        header[20..36].copy_from_slice(&self.send_nonce_prefix);
        header[36..44].copy_from_slice(&sequence.to_be_bytes());
        let nonce = relay_nonce(self.send_nonce_prefix, sequence);
        let aad = relay_aad(&header, self.session_id);
        let ciphertext = self
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| DesktopError::Process("relay frame encryption failed".into()))?;
        let mut frame = Vec::with_capacity(RELAY_HEADER_BYTES + ciphertext.len());
        frame.extend_from_slice(&header);
        frame.extend_from_slice(&ciphertext);
        Ok(frame)
    }

    fn decrypt(&mut self, frame: &[u8]) -> Result<Vec<u8>, DesktopError> {
        if !(RELAY_HEADER_BYTES + RELAY_TAG_BYTES..=RELAY_MAX_FRAME_BYTES).contains(&frame.len())
            || frame[..4] != RELAY_MAGIC
            || frame[4..20] != self.peer_device_id
        {
            return Err(DesktopError::Process(
                "relay received an invalid encrypted frame".into(),
            ));
        }
        let mut prefix = [0_u8; 16];
        prefix.copy_from_slice(&frame[20..36]);
        let sequence = u64::from_be_bytes(
            frame[36..44]
                .try_into()
                .map_err(|_| DesktopError::Process("relay sequence is invalid".into()))?,
        );
        if self
            .peer_nonce_prefix
            .is_some_and(|pinned| pinned != prefix)
        {
            return Err(DesktopError::Process(
                "relay rejected a replayed or migrated frame".into(),
            ));
        }
        let mut header = [0_u8; RELAY_HEADER_BYTES];
        header.copy_from_slice(&frame[..RELAY_HEADER_BYTES]);
        let nonce = relay_nonce(prefix, sequence);
        let aad = relay_aad(&header, self.session_id);
        let plaintext = self
            .cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &frame[RELAY_HEADER_BYTES..],
                    aad: &aad,
                },
            )
            .map_err(|_| DesktopError::Process("relay frame authentication failed".into()))?;
        self.peer_nonce_prefix.get_or_insert(prefix);
        if !self.receive_window.accept(sequence) {
            return Err(DesktopError::Process(
                "relay rejected a replayed frame".into(),
            ));
        }
        Ok(plaintext)
    }
}

fn relay_nonce(prefix: [u8; 16], sequence: u64) -> [u8; 24] {
    let mut nonce = [0_u8; 24];
    nonce[..16].copy_from_slice(&prefix);
    nonce[16..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

fn relay_aad(header: &[u8; RELAY_HEADER_BYTES], session_id: [u8; 16]) -> [u8; 60] {
    let mut aad = [0_u8; 60];
    aad[..RELAY_HEADER_BYTES].copy_from_slice(header);
    aad[RELAY_HEADER_BYTES..].copy_from_slice(&session_id);
    aad
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tokio_tungstenite::connect_async;

    #[tokio::test]
    async fn congested_video_upload_does_not_block_incoming_input_or_stop() {
        use tokio::io::AsyncReadExt;
        use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

        // A full duplex transport whose outgoing video cannot fit until the
        // peer reads it. The peer deliberately stops reading that direction.
        let (local, mut remote) = tokio::io::duplex(1_024);
        let websocket = WebSocketStream::from_raw_socket(local, Role::Client, None).await;
        let session = uuid::Uuid::new_v4();
        let left = uuid::Uuid::new_v4();
        let right = uuid::Uuid::new_v4();
        let credential = "relay-congestion-test-credential";
        let cipher = RelayCipher::new(session, left, right, credential).unwrap();
        let mut peer_cipher = RelayCipher::new(session, right, left, credential).unwrap();
        let proxy = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let engine = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let (stop_tx, stop_rx) = oneshot::channel();
        let bridge = tokio::spawn(run_bridge(
            proxy,
            engine.local_addr().unwrap(),
            websocket,
            cipher,
            stop_rx,
        ));
        engine.send_to(&[42; 8_192], proxy_address).await.unwrap();
        // Seeing the first binary-frame byte proves the video write has begun.
        tokio::time::timeout(Duration::from_secs(1), remote.read_u8())
            .await
            .unwrap()
            .unwrap();
        let mut peer = WebSocketStream::from_raw_socket(remote, Role::Server, None).await;
        let input = peer_cipher.encrypt(b"mouse-button-release").unwrap();
        peer.send(Message::Binary(input.into())).await.unwrap();
        let mut buffer = [0; 64];
        let (length, _) =
            tokio::time::timeout(Duration::from_millis(500), engine.recv_from(&mut buffer))
                .await
                .expect("incoming input must not wait for the blocked video upload")
                .unwrap();
        assert_eq!(&buffer[..length], b"mouse-button-release");
        stop_tx.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), bridge)
                .await
                .unwrap()
                .unwrap(),
            None
        );
    }

    #[test]
    fn relay_endpoint_uses_wss_and_bounded_identity_parameters() {
        let session = uuid::Uuid::new_v4();
        let device = uuid::Uuid::new_v4();
        let endpoint = relay_endpoint("https://sanser.example/base", session, device).unwrap();
        assert_eq!(endpoint.scheme(), "wss");
        assert_eq!(endpoint.path(), "/base/api/v2/relay");
        assert!(endpoint.query().is_some_and(|query| {
            query.contains(&session.to_string()) && query.contains(&device.to_string())
        }));
    }

    #[test]
    fn relay_frames_are_end_to_end_encrypted_and_replay_safe() {
        let session = uuid::Uuid::new_v4();
        let left = uuid::Uuid::new_v4();
        let right = uuid::Uuid::new_v4();
        let mut sender =
            RelayCipher::new(session, left, right, "a-secure-session-credential-123456").unwrap();
        let mut receiver =
            RelayCipher::new(session, right, left, "a-secure-session-credential-123456").unwrap();
        let frame = sender.encrypt(b"opaque SNV2 packet").unwrap();
        assert!(!frame.windows(6).any(|window| window == b"opaque"));
        assert_eq!(receiver.decrypt(&frame).unwrap(), b"opaque SNV2 packet");
        assert!(receiver.decrypt(&frame).is_err());
    }

    #[test]
    fn relay_cipher_accepts_reordered_datagrams_without_accepting_replays() {
        let session = uuid::Uuid::new_v4();
        let left = uuid::Uuid::new_v4();
        let right = uuid::Uuid::new_v4();
        let credential = "a-secure-session-credential-123456";
        let mut sender = RelayCipher::new(session, left, right, credential).unwrap();
        let mut receiver = RelayCipher::new(session, right, left, credential).unwrap();
        let first = sender.encrypt(b"first").unwrap();
        let second = sender.encrypt(b"second").unwrap();
        let third = sender.encrypt(b"third").unwrap();
        assert_eq!(receiver.decrypt(&third).unwrap(), b"third");
        let mut forged = first.clone();
        *forged.last_mut().unwrap() ^= 1;
        assert!(receiver.decrypt(&forged).is_err());
        assert_eq!(receiver.decrypt(&first).unwrap(), b"first");
        assert_eq!(receiver.decrypt(&second).unwrap(), b"second");
        assert!(receiver.decrypt(&first).is_err());
    }

    #[tokio::test]
    async fn local_udp_bridge_carries_only_encrypted_websocket_frames() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let websocket_address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let (client_websocket, _) = connect_async(format!("ws://{websocket_address}"))
            .await
            .unwrap();
        let mut server_websocket = server.await.unwrap();

        let session = uuid::Uuid::new_v4();
        let left = uuid::Uuid::new_v4();
        let right = uuid::Uuid::new_v4();
        let credential = "a-secure-session-credential-123456";
        let left_cipher = RelayCipher::new(session, left, right, credential).unwrap();
        let mut right_cipher = RelayCipher::new(session, right, left, credential).unwrap();
        let proxy_socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let proxy_address = proxy_socket.local_addr().unwrap();
        let engine_socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let engine_address = engine_socket.local_addr().unwrap();
        let (stop_tx, stop_rx) = oneshot::channel();
        let bridge = tokio::spawn(run_bridge(
            proxy_socket,
            engine_address,
            client_websocket,
            left_cipher,
            stop_rx,
        ));

        engine_socket
            .send_to(b"native packet", proxy_address)
            .await
            .unwrap();
        let encrypted = loop {
            match server_websocket.next().await {
                Some(Ok(Message::Binary(frame))) => break frame,
                Some(Ok(Message::Ping(payload))) => {
                    server_websocket.send(Message::Pong(payload)).await.unwrap();
                }
                other => panic!("expected an encrypted relay frame, got {other:?}"),
            }
        };
        assert!(
            !encrypted
                .windows(b"native packet".len())
                .any(|window| window == b"native packet")
        );
        assert_eq!(right_cipher.decrypt(&encrypted).unwrap(), b"native packet");

        let response = right_cipher.encrypt(b"remote packet").unwrap();
        server_websocket
            .send(Message::Binary(response.into()))
            .await
            .unwrap();
        let mut buffer = [0_u8; 64];
        let (length, source) =
            tokio::time::timeout(Duration::from_secs(1), engine_socket.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(source, proxy_address);
        assert_eq!(&buffer[..length], b"remote packet");

        let _ = stop_tx.send(());
        assert_eq!(bridge.await.unwrap(), None);
    }

    #[test]
    fn relay_retries_only_udp_port_startup_errors() {
        use std::io::{Error, ErrorKind};
        assert!(udp_port_not_ready(&Error::from(ErrorKind::ConnectionReset)));
        assert!(udp_port_not_ready(&Error::from(
            ErrorKind::ConnectionRefused
        )));
        assert!(!udp_port_not_ready(&Error::from(
            ErrorKind::PermissionDenied
        )));
        assert!(!udp_port_not_ready(&Error::from(ErrorKind::InvalidInput)));
        #[cfg(windows)]
        assert!(udp_port_not_ready(&Error::from_raw_os_error(10054)));
    }

    #[tokio::test]
    async fn local_udp_bridge_survives_packets_before_engine_binds() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let (websocket, _) = connect_async(format!("ws://{address}")).await.unwrap();
        let mut remote = server.await.unwrap();
        let session = uuid::Uuid::new_v4();
        let local_device = uuid::Uuid::new_v4();
        let remote_device = uuid::Uuid::new_v4();
        let credential = "a-secure-session-credential-123456";
        let cipher = RelayCipher::new(session, local_device, remote_device, credential).unwrap();
        let mut remote_cipher =
            RelayCipher::new(session, remote_device, local_device, credential).unwrap();
        let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let proxy = socket.local_addr().unwrap();
        let reservation = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let engine_address = reservation.local_addr().unwrap();
        drop(reservation);
        let (stop_tx, stop_rx) = oneshot::channel();
        let bridge = tokio::spawn(run_bridge(
            socket,
            engine_address,
            websocket,
            cipher,
            stop_rx,
        ));

        // Simulate the other computer sending while the local sidecar starts.
        // Sending to this closed port generates WSAECONNRESET on Windows.
        for _ in 0..3 {
            let frame = remote_cipher.encrypt(b"early remote keepalive").unwrap();
            remote.send(Message::Binary(frame.into())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            !bridge.is_finished(),
            "relay must survive the native startup window"
        );
        let engine = tokio::net::UdpSocket::bind(engine_address).await.unwrap();
        engine.send_to(b"engine ready", proxy).await.unwrap();
        let packet = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match remote.next().await {
                    Some(Ok(Message::Binary(frame))) => break frame,
                    Some(Ok(Message::Ping(payload))) => {
                        remote.send(Message::Pong(payload)).await.unwrap();
                    }
                    other => panic!("expected native packet, got {other:?}"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(remote_cipher.decrypt(&packet).unwrap(), b"engine ready");
        remote
            .send(Message::Binary(
                remote_cipher
                    .encrypt(b"remote after startup")
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        let mut buffer = [0; 64];
        let (length, source) =
            tokio::time::timeout(Duration::from_secs(3), engine.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(source, proxy);
        assert_eq!(&buffer[..length], b"remote after startup");
        let _ = stop_tx.send(());
        assert_eq!(bridge.await.unwrap(), None);
    }

    #[tokio::test]
    async fn relay_bridge_reports_repeated_session_credential_mismatch() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let websocket_address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let (client_websocket, _) = connect_async(format!("ws://{websocket_address}"))
            .await
            .unwrap();
        let mut server_websocket = server.await.unwrap();

        let session = uuid::Uuid::new_v4();
        let left = uuid::Uuid::new_v4();
        let right = uuid::Uuid::new_v4();
        let left_cipher =
            RelayCipher::new(session, left, right, "left-session-credential-1234567890").unwrap();
        let mut wrong_peer_cipher =
            RelayCipher::new(session, right, left, "different-session-credential-12345").unwrap();
        let proxy_socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let engine_socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let (_stop_tx, stop_rx) = oneshot::channel();
        let bridge = tokio::spawn(run_bridge(
            proxy_socket,
            engine_socket.local_addr().unwrap(),
            client_websocket,
            left_cipher,
            stop_rx,
        ));

        for _ in 0..32 {
            let frame = wrong_peer_cipher.encrypt(b"credential mismatch").unwrap();
            server_websocket
                .send(Message::Binary(frame.into()))
                .await
                .unwrap();
        }
        let failure = tokio::time::timeout(Duration::from_secs(1), bridge)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(failure.contains("authentication repeatedly failed"));
    }

    #[test]
    fn removing_one_relay_viewer_does_not_stop_other_viewers_or_local_client() {
        let manager = RelayManager::default();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let mut receivers = Vec::new();
        for (kind, session) in [
            (EngineKind::Host, a),
            (EngineKind::Host, b),
            (EngineKind::Client, a),
        ] {
            let (stop, receiver) = oneshot::channel();
            receivers.push(receiver);
            manager.state.lock().unwrap().bridges.insert(
                (kind, session),
                RelayBridge {
                    transport: "udp",
                    id: uuid::Uuid::new_v4(),
                    session_id: session,
                    engine_port: 50000,
                    proxy_port: 50001,
                    stop,
                },
            );
        }
        manager.stop_peer(EngineKind::Host, a);
        assert!(receivers[0].try_recv().is_ok());
        assert!(receivers[1].try_recv().is_err());
        assert!(receivers[2].try_recv().is_err());
        assert!(
            manager
                .transport(EngineKind::Host, &b.to_string())
                .is_some()
        );
        manager.stop_for_engine(EngineKind::Host);
        assert!(receivers[1].try_recv().is_ok());
        assert!(receivers[2].try_recv().is_err());
    }
    #[test]
    fn pending_relay_negotiations_count_toward_capacity_and_release_on_failure() {
        let manager = RelayManager::default();
        let mut reservations = Vec::new();
        for _ in 0..3 {
            reservations.push(
                manager
                    .reserve(EngineKind::Host, uuid::Uuid::new_v4(), true)
                    .unwrap(),
            );
        }
        assert!(
            manager
                .reserve(EngineKind::Host, uuid::Uuid::new_v4(), true)
                .is_err()
        );
        reservations.pop();
        assert!(
            manager
                .reserve(EngineKind::Host, uuid::Uuid::new_v4(), true)
                .is_ok()
        );
        assert!(
            manager
                .reserve(EngineKind::Client, uuid::Uuid::new_v4(), false)
                .is_ok()
        );
    }

    #[test]
    fn a_stopped_or_replaced_negotiation_cannot_install_a_late_bridge() {
        let manager = RelayManager::default();
        let session = uuid::Uuid::new_v4();
        let stale = manager.reserve(EngineKind::Host, session, true).unwrap();
        let current = manager.reserve(EngineKind::Host, session, true).unwrap();
        drop(stale);
        assert_eq!(
            manager.state.lock().unwrap().pending.get(&current.key),
            Some(&current.id)
        );
        manager.stop_peer(EngineKind::Host, session);
        let (stop, _) = oneshot::channel();
        let bridge = RelayBridge {
            transport: "udp",
            id: uuid::Uuid::new_v4(),
            session_id: session,
            engine_port: 50000,
            proxy_port: 50001,
            stop,
        };
        assert!(current.install(bridge).is_err());
        assert!(
            manager
                .transport(EngineKind::Host, &session.to_string())
                .is_none()
        );
    }
}
