use super::{RELAY_CONNECT_TIMEOUT, RelayCipher, udp_port_not_ready};
use futures_util::{SinkExt, StreamExt};
use rand::{RngCore, rngs::OsRng};
use sanser_network::relay_datagram::{Kind, MAX_PACKET_BYTES, OVERHEAD, Offer, PacketAuth};
use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::oneshot,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{
        Message,
        http::{Request, header},
    },
};

pub(super) struct UdpTransport {
    socket: UdpSocket,
    auth: PacketAuth,
    websocket: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

/// Probe the optional UDP service before falling back to the existing WSS relay.
pub(super) async fn connect(mut request: Request<()>) -> Result<UdpTransport, String> {
    let mut endpoint =
        url::Url::parse(&request.uri().to_string()).map_err(|_| "invalid UDP control URL")?;
    endpoint.set_path(&format!("{}/udp", endpoint.path()));
    *request.uri_mut() = endpoint
        .as_str()
        .parse()
        .map_err(|_| "invalid UDP control URI")?;
    tokio::time::timeout(Duration::from_secs(5), connect_inner(request))
        .await
        .map_err(|_| "UDP relay negotiation timed out".to_owned())?
}

async fn connect_inner(request: Request<()>) -> Result<UdpTransport, String> {
    let (mut websocket, response) = connect_async_with_config(request, None, true)
        .await
        .map_err(|_| "UDP relay control unavailable")?;
    if response
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        != Some("sanser-relay-v1")
    {
        return Err("UDP relay control protocol mismatch".into());
    }
    let offer: Offer = loop {
        match websocket.next().await {
            Some(Ok(Message::Text(message))) => {
                let mut envelope: serde_json::Value =
                    serde_json::from_str(&message).map_err(|_| "invalid UDP relay offer")?;
                if envelope["type"] != "relay.udpOffer" {
                    return Err("UDP relay offer missing".into());
                }
                break serde_json::from_value(envelope["offer"].take())
                    .map_err(|_| "invalid UDP relay allocation")?;
            }
            Some(Ok(Message::Ping(payload))) => websocket
                .send(Message::Pong(payload))
                .await
                .map_err(|_| "UDP relay control closed")?,
            _ => return Err("UDP relay control closed before offer".into()),
        }
    };
    let target = tokio::net::lookup_host(offer.address.as_str())
        .await
        .map_err(|_| "UDP relay address unresolved")?
        .next()
        .ok_or("UDP relay address missing")?;
    if target.port() == 0 || target.ip().is_unspecified() || target.ip().is_multicast() {
        return Err("invalid UDP relay address".into());
    }
    let socket = if target.is_ipv4() {
        UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await
    } else {
        UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).await
    }
    .map_err(|_| "UDP relay socket unavailable")?;
    socket
        .connect(target)
        .await
        .map_err(|_| "UDP relay route unavailable")?;
    let mut auth = PacketAuth::new(offer.allocation_id, offer.key);
    let mut nonce = [0; 16];
    OsRng.fill_bytes(&mut nonce);
    let mut retry = tokio::time::interval(Duration::from_millis(250));
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buffer = [0; 256];
    let mut confirmed = false;
    let mut peer_ready = false;
    loop {
        tokio::select! {
            _ = retry.tick() => {
                let packet = auth.encode(Kind::Register, &nonce).map_err(|_| "UDP relay registration failed")?;
                socket.send(&packet).await.map_err(|_| "UDP relay unreachable")?;
            }
            received = socket.recv(&mut buffer) => {
                let n = received.map_err(|_| "UDP relay unreachable")?;
                let Ok((kind, payload)) = auth.decode(&buffer[..n], true) else { continue; };
                match kind {
                    Kind::Challenge if payload.len() == 16 => {
                        let packet = auth.encode(Kind::Confirm, payload).map_err(|_| "UDP relay confirmation failed")?;
                        socket.send(&packet).await.map_err(|_| "UDP relay unreachable")?;
                    }
                    Kind::Confirmed => confirmed = true,
                    _ => {},
                }
            }
            message = websocket.next() => match message {
                Some(Ok(Message::Text(message))) => {
                    let envelope: serde_json::Value = serde_json::from_str(&message).map_err(|_| "invalid UDP relay control")?;
                    if envelope["type"] == "relay.udpReady" { peer_ready = true; }
                    else if envelope["type"] == "relay.peerOffline" { return Err("UDP relay peer unavailable".into()); }
                }
                Some(Ok(Message::Ping(payload))) => websocket.send(Message::Pong(payload)).await.map_err(|_| "UDP relay control closed")?,
                Some(Ok(Message::Pong(_))) => {},
                _ => return Err("UDP relay control closed".into()),
            }
        }
        if confirmed && peer_ready {
            return Ok(UdpTransport {
                socket,
                auth,
                websocket,
            });
        }
    }
}

fn send_datagram(
    socket: &UdpSocket,
    auth: &mut PacketAuth,
    cipher: &mut RelayCipher,
    bytes: &[u8],
) -> Result<bool, String> {
    let encrypted = cipher
        .encrypt(bytes)
        .map_err(|_| "UDP relay encryption failed".to_owned())?;
    if encrypted.len() > MAX_PACKET_BYTES - OVERHEAD {
        return Err("native packet exceeds UDP relay limit".into());
    }
    let packet = auth
        .encode(Kind::Data, &encrypted)
        .map_err(|_| "UDP relay sequence exhausted".to_owned())?;
    match socket.try_send(&packet) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(_) => Err("UDP relay send failed".into()),
    }
}

pub(super) async fn run(
    mut transport: UdpTransport,
    local: UdpSocket,
    engine: SocketAddr,
    mut cipher: RelayCipher,
    mut stop: oneshot::Receiver<()>,
    observe: impl Fn(sanser_p2p::fec::Stats) + Send + Sync + 'static,
) -> Option<String> {
    let Some(mut fec) = cipher.fec.take() else {
        return Some("UDP FEC initialization unavailable".into());
    };
    let (mut writer, mut reader) = transport.websocket.split();
    let control = async {
        while let Some(message) = reader.next().await {
            match message {
                Ok(Message::Ping(payload)) => {
                    if writer.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Pong(_)) => {}
                Ok(Message::Text(text)) => {
                    let message: serde_json::Value =
                        serde_json::from_str(&text).unwrap_or_default();
                    if message["type"] == "relay.peerOffline" {
                        break;
                    }
                }
                _ => break,
            }
        }
        Some("UDP relay control closed; retry the accepted session".to_owned())
    };
    let media = async {
        let mut outgoing = vec![0; MAX_PACKET_BYTES + 1];
        let mut incoming = vec![0; MAX_PACKET_BYTES + 1];
        let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let startup_deadline = tokio::time::Instant::now() + RELAY_CONNECT_TIMEOUT;
        let mut engine_ready = false;
        let mut last_received = tokio::time::Instant::now();
        let mut last_report = std::time::Instant::now();
        loop {
            tokio::select! {
                received = local.recv_from(&mut outgoing) => {
                    let (n, source) = match received {
                        Ok(value) => value,
                        Err(error) if tokio::time::Instant::now() < startup_deadline && udp_port_not_ready(&error) => continue,
                        Err(_) => return Some("local UDP relay socket failed".into()),
                    };
                    if source != engine { continue; }
                    engine_ready = true;
                    let now=std::time::Instant::now();
                    match send_datagram(&transport.socket,&mut transport.auth,&mut cipher,&outgoing[..n]) {
                        Ok(true)=> {if let Some(parity)=fec.outgoing(&outgoing[..n],now) {
                            match send_datagram(&transport.socket,&mut transport.auth,&mut cipher,&parity) {
                                Ok(true)=>{},Ok(false)=>fec.local_drop(now),Err(error)=>return Some(error),
                            }
                        }},
                        Ok(false)=>fec.local_drop(now),Err(error)=>return Some(error),
                    }
                }
                received = transport.socket.recv(&mut incoming) => {
                    let Ok(n) = received else { return Some("UDP relay receive failed".into()); };
                    let Ok((kind, payload)) = transport.auth.decode(&incoming[..n], true) else { continue; };
                    last_received = tokio::time::Instant::now();
                    if kind != Kind::Forward { continue; }
                    let Ok(plaintext) = cipher.decrypt(payload) else { continue; };
                    let now=std::time::Instant::now();
                    if !sanser_p2p::fec::is_packet(&plaintext) && let Err(error)=local.try_send_to(&plaintext,engine) {
                        if error.kind()!=std::io::ErrorKind::WouldBlock && !(tokio::time::Instant::now()<startup_deadline && udp_port_not_ready(&error)) {return Some("UDP relay could not reach the native engine".into());}
                        fec.local_drop(now);
                    }
                    let(_,recovered)=fec.receive(&plaintext,now);
                    for packet in recovered {if local.try_send_to(&packet,engine).is_err(){fec.local_drop(now);}}
                }
                ()=tokio::time::sleep(fec.next_tick(std::time::Instant::now()))=> {
                    let now=std::time::Instant::now();
                    for packet in fec.tick(now) {match send_datagram(&transport.socket,&mut transport.auth,&mut cipher,&packet) {Ok(true)=>{},Ok(false)=>fec.local_drop(now),Err(error)=>return Some(error)}}

                }
                _ = heartbeat.tick() => {
                    let now=std::time::Instant::now();
                    if now.duration_since(last_report)>=Duration::from_secs(2) {observe(fec.stats(now));last_report=now;}
                    if !engine_ready && tokio::time::Instant::now() >= startup_deadline { return Some("native engine did not start sending UDP".into()); }
                    if last_received.elapsed() > Duration::from_secs(10) { return Some("UDP relay stopped responding; retry the session".into()); }
                    if let Ok(packet) = transport.auth.encode(Kind::Keepalive, &[]) { let _ = transport.socket.try_send(&packet); }
                }
            }
        }
    };
    tokio::select! { _ = &mut stop => None, result = control => result, result = media => result }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            client::IntoClientRequest,
            handshake::server::{Request as ServerRequest, Response},
        },
    };

    #[tokio::test]
    #[allow(clippy::too_many_lines, clippy::result_large_err)]
    async fn negotiates_udp_and_delivers_input_after_lost_and_reordered_video() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let offer = Offer {
            allocation_id: [3; 16],
            key: [9; 32],
            address: relay.local_addr().unwrap().to_string(),
        };
        let session = uuid::Uuid::new_v4();
        let left = uuid::Uuid::new_v4();
        let right = uuid::Uuid::new_v4();
        let credential = "UDP-bridge-end-to-end-test-credential";
        let mut peer_cipher = RelayCipher::new(session, right, left, credential).unwrap();
        let address = listener.local_addr().unwrap();
        let (stop_server, mut stopped) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws =
                accept_hdr_async(stream, |request: &ServerRequest, mut response: Response| {
                    assert_eq!(request.uri().path(), "/api/v2/relay/udp");
                    response.headers_mut().insert(
                        header::SEC_WEBSOCKET_PROTOCOL,
                        "sanser-relay-v1".parse().unwrap(),
                    );
                    Ok(response)
                })
                .await
                .unwrap();
            ws.send(Message::Text(
                serde_json::json!({"type":"relay.udpOffer","offer":offer})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            let mut auth = PacketAuth::new(offer.allocation_id, offer.key);
            let mut buffer = [0; 4096];
            let mut registered = false;
            let mut confirmed = false;
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    received = relay.recv_from(&mut buffer) => {
                        let (n, source) = received.unwrap();
                        let (kind, payload) = auth.decode(&buffer[..n], false).unwrap();
                        match kind {
                            Kind::Register => {
                                // The first registration is lost: retry must recover.
                                if !registered { registered = true; continue; }
                                relay.send_to(&auth.encode(Kind::Challenge, &[4;16]).unwrap(), source).await.unwrap();
                            }
                            Kind::Confirm => {
                                assert_eq!(payload, &[4;16]);
                                confirmed = true;
                                relay.send_to(&auth.encode(Kind::Confirmed, &[]).unwrap(), source).await.unwrap();
                                ws.send(Message::Text("{\"type\":\"relay.udpReady\"}".into())).await.unwrap();
                            }
                            Kind::Data => {
                                assert!(confirmed);
                                let native=peer_cipher.decrypt(payload).unwrap();
                                if sanser_p2p::fec::is_packet(&native) {continue;}
                                assert_eq!(native,b"native-video");
                                let lost = peer_cipher.encrypt(b"lost-video").unwrap();
                                let _ = auth.encode(Kind::Forward, &lost).unwrap();
                                let delayed = peer_cipher.encrypt(b"delayed-video").unwrap();
                                let delayed = auth.encode(Kind::Forward, &delayed).unwrap();
                                let input = peer_cipher.encrypt(b"key-up").unwrap();
                                let input = auth.encode(Kind::Forward, &input).unwrap();
                                relay.send_to(&input, source).await.unwrap();
                                relay.send_to(&delayed, source).await.unwrap();
                            }
                            Kind::Keepalive => { relay.send_to(&auth.encode(Kind::Alive, &[]).unwrap(), source).await.unwrap(); }
                            _ => panic!("unexpected client packet"),
                        }
                    }
                    message = ws.next() => match message {
                        Some(Ok(Message::Ping(payload))) => ws.send(Message::Pong(payload)).await.unwrap(),
                        Some(Ok(Message::Pong(_))) => {},
                        Some(Ok(Message::Binary(_))) => panic!("UDP media must never go through WSS"),
                        _ => break,
                    }
                }
            }
        });
        let mut request = format!("ws://{address}/api/v2/relay")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "sanser-relay-v1".parse().unwrap(),
        );
        let transport = connect(request).await.unwrap();
        let proxy = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let engine = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let bridge = tokio::spawn(run(
            transport,
            proxy,
            engine.local_addr().unwrap(),
            RelayCipher::new(session, left, right, credential).unwrap(),
            stopped,
            |_| {},
        ));
        engine
            .send_to(b"native-video", proxy_address)
            .await
            .unwrap();
        let mut buffer = [0; 128];
        for expected in [b"key-up".as_slice(), b"delayed-video".as_slice()] {
            let (n, _) =
                tokio::time::timeout(Duration::from_secs(1), engine.recv_from(&mut buffer))
                    .await
                    .expect("lost datagram must not block subsequent input")
                    .unwrap();
            assert_eq!(&buffer[..n], expected);
        }
        stop.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), bridge)
                .await
                .unwrap()
                .unwrap(),
            None
        );
        let _ = stop_server.send(());
        server.await.unwrap();
    }

    // Synthetic native-shaped ciphertext only: no display, playback or OS input.
    fn native_video(sequence: u64) -> Vec<u8> {
        let mut b = vec![0; 1120];
        for (index, byte) in b.iter_mut().enumerate() {
            *byte = ((index as u64 * 13 + sequence) % 251) as u8;
        }
        b[..4].copy_from_slice(b"SNV2");
        b[4] = 2;
        b[5] = 80;
        b[6] = 3;
        b[7] = 5;
        b[8..12].fill(0);
        b[28..32].copy_from_slice(&1_u32.to_be_bytes());
        b[32..40].copy_from_slice(&sequence.to_be_bytes());
        b[56..60].copy_from_slice(&1040_u32.to_be_bytes());
        b[60..64].copy_from_slice(&7_u32.to_be_bytes());
        b
    }
    #[tokio::test]
    #[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
    async fn encrypted_udp_relay_adapts_fec_and_recovers_dropped_video_without_blocking_input() {
        use std::{
            collections::BTreeSet,
            sync::{Arc, Mutex},
        };
        let session = uuid::Uuid::new_v4();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let credential = "synthetic-fec-relay-test-credential";
        let relay = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address = relay.local_addr().unwrap();
        let left = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let left_address = left.local_addr().unwrap();
        let right_address = right.local_addr().unwrap();
        left.connect(address).await.unwrap();
        right.connect(address).await.unwrap();
        let mut websockets = Vec::new();
        let mut retained_control = Vec::new();
        for _ in 0..2 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (client, server) = tokio::join!(
                tokio_tungstenite::connect_async(format!("ws://{address}")),
                async {
                    let (stream, _) = listener.accept().await.unwrap();
                    tokio_tungstenite::accept_async(stream).await.unwrap()
                }
            );
            websockets.push(client.unwrap().0);
            retained_control.push(server);
        }
        let auth_a = || PacketAuth::new([1; 16], [2; 32]);
        let auth_b = || PacketAuth::new([3; 16], [4; 32]);
        let lost = Arc::new(Mutex::new(BTreeSet::new()));
        let dropped = Arc::clone(&lost);
        let service = Arc::clone(&relay);
        let mut inspect = RelayCipher::new(session, b, a, credential).unwrap();
        let server = tokio::spawn(async move {
            let mut left = auth_a();
            let mut right = auth_b();
            let mut buffer = vec![0; 65536];
            loop {
                let (n, from) = service.recv_from(&mut buffer).await.unwrap();
                let (source, target, address) = if from == left_address {
                    (&mut left, &mut right, right_address)
                } else {
                    assert_eq!(from, right_address);
                    (&mut right, &mut left, left_address)
                };
                let (kind, payload) = source.decode(&buffer[..n], false).unwrap();
                let packet = match kind {
                    Kind::Keepalive => Some((source.encode(Kind::Alive, &[]).unwrap(), from)),
                    Kind::Data => {
                        assert!(
                            payload.starts_with(b"SNR1"),
                            "FEC/video must remain encrypted outside the clients"
                        );
                        if from == left_address {
                            let plain = inspect.decrypt(payload).unwrap();
                            if let Some(meta) = sanser_p2p::fec::video(&plain)
                                && meta.sequence % 16 == 7
                            {
                                dropped.lock().unwrap().insert(meta.sequence);
                                continue;
                            }
                        }
                        Some((target.encode(Kind::Forward, payload).unwrap(), address))
                    }
                    _ => panic!("unexpected datagram kind"),
                };
                if let Some((packet, address)) = packet {
                    service.send_to(&packet, address).await.unwrap();
                }
            }
        });
        let engine_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let engine_b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let local_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy_a = local_a.local_addr().unwrap();
        let proxy_b = local_b.local_addr().unwrap();
        let (stop_a, stopped_a) = oneshot::channel();
        let (stop_b, stopped_b) = oneshot::channel();
        let stats = Arc::new(Mutex::new(sanser_p2p::fec::Stats::default()));
        let observe = Arc::clone(&stats);
        let websocket_a = websockets.remove(0);
        let websocket_b = websockets.remove(0);
        let bridge_a = tokio::spawn(run(
            UdpTransport {
                socket: left,
                auth: auth_a(),
                websocket: websocket_a,
            },
            local_a,
            engine_a.local_addr().unwrap(),
            RelayCipher::new(session, a, b, credential).unwrap(),
            stopped_a,
            |_| {},
        ));
        let bridge_b = tokio::spawn(run(
            UdpTransport {
                socket: right,
                auth: auth_b(),
                websocket: websocket_b,
            },
            local_b,
            engine_b.local_addr().unwrap(),
            RelayCipher::new(session, b, a, credential).unwrap(),
            stopped_b,
            move |s| *observe.lock().unwrap() = s,
        ));
        let received = Arc::new(Mutex::new(BTreeSet::new()));
        let delivered = Arc::clone(&received);
        let viewer = Arc::clone(&engine_b);
        let viewer_task = tokio::spawn(async move {
            let mut buffer = [0; 2048];
            loop {
                let (n, _) = viewer.recv_from(&mut buffer).await.unwrap();
                let meta = sanser_p2p::fec::video(&buffer[..n])
                    .expect("protection control leaked to native");
                assert_eq!(&buffer[..n], native_video(meta.sequence));
                delivered.lock().unwrap().insert(meta.sequence);
            }
        });
        let deadline = std::time::Instant::now() + Duration::from_millis(4500);
        let mut sequence = 1;
        while std::time::Instant::now() < deadline {
            for _ in 0..16 {
                engine_a
                    .send_to(&native_video(sequence), proxy_a)
                    .await
                    .unwrap();
                sequence += 1;
            }
            for payload in [b"key-up".as_slice(), b"opus-audio"] {
                engine_b.send_to(payload, proxy_b).await.unwrap();
                let mut buffer = [0; 64];
                let (n, _) = tokio::time::timeout(
                    Duration::from_millis(500),
                    engine_a.recv_from(&mut buffer),
                )
                .await
                .expect("FEC blocked priority datagram")
                .unwrap();
                assert_eq!(&buffer[..n], payload);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while stats.lock().unwrap().recovered == 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let snapshot = *stats.lock().unwrap();
        assert!(
            snapshot.recovered > 0,
            "UDP relay never restored discarded ciphertext: {snapshot:?}"
        );
        assert!(
            lost.lock()
                .unwrap()
                .iter()
                .any(|seq| received.lock().unwrap().contains(seq))
        );
        assert!(!viewer_task.is_finished());
        viewer_task.abort();
        stop_a.send(()).unwrap();
        stop_b.send(()).unwrap();
        assert!(bridge_a.await.unwrap().is_none());
        assert!(bridge_b.await.unwrap().is_none());
        server.abort();
        drop(retained_control);
    }

    #[tokio::test]
    async fn unsupported_udp_endpoint_returns_error_for_legacy_wss_fallback() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let request = format!("ws://{address}/api/v2/relay")
            .into_client_request()
            .unwrap();
        assert!(connect(request).await.is_err());
        server.await.unwrap();
    }
}
