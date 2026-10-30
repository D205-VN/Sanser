//! Lifetime-bound Direct UDP routing. The external socket is the same one used
//! for gathering/nomination; native engines see only a fixed loopback peer.
use crate::{
    error::DesktopError,
    models::{EngineKind, LaunchEngineRequest},
};
use sanser_p2p::standby::{self, Config};
use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
};
use tauri::{AppHandle, Emitter};

pub struct DirectTransport {
    pub socket: Arc<tokio::net::UdpSocket>,
    pub monitor: Option<Config>,
}
pub struct Prepared {
    transport: DirectTransport,
    local: tokio::net::UdpSocket,
    engine: UdpSocket,
}
pub struct Bridge {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
impl Prepared {
    pub fn new(
        transport: DirectTransport,
        request: &mut LaunchEngineRequest,
    ) -> Result<Self, DesktopError> {
        let prepare = || -> std::io::Result<_> {
            let reserve = || -> std::io::Result<UdpSocket> {
                for _ in 0..8 {
                    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
                    if socket.local_addr()?.port() <= 65533 {
                        return Ok(socket);
                    }
                }
                Err(std::io::Error::other("no native-compatible ephemeral port"))
            };
            let local = reserve()?;
            local.set_nonblocking(true)?;
            let engine = reserve()?;
            Ok((tokio::net::UdpSocket::from_std(local)?, engine))
        };
        let (local, engine) = prepare().map_err(|_| {
            DesktopError::Process("unable to reserve local Direct router ports".into())
        })?;
        let proxy = local
            .local_addr()
            .map_err(|_| DesktopError::Process("Direct router port unavailable".into()))?;
        let port = engine
            .local_addr()
            .map_err(|_| DesktopError::Process("Direct engine port unavailable".into()))?
            .port();
        if port > 65533 || proxy.port() > 65533 {
            return Err(DesktopError::Process(
                "Direct router port outside native range; retry connection".into(),
            ));
        }
        match request.kind {
            EngineKind::Host => {
                request.address = Some(Ipv4Addr::LOCALHOST.to_string());
                request.port = Some(proxy.port());
                request.udp_bind_port = Some(port);
            }
            EngineKind::Client => {
                request.port = Some(port);
                request.udp_connect = Some(proxy.to_string());
            }
            EngineKind::LocalServer => {
                return Err(DesktopError::InvalidRequest(
                    "Direct media router requires a native peer".into(),
                ));
            }
        }
        Ok(Self {
            transport,
            local,
            engine,
        })
    }
    pub fn start(
        self,
        app: AppHandle,
        kind: EngineKind,
        attempt: String,
        metrics: Option<std::sync::mpsc::SyncSender<crate::connection_report::Sample>>,
    ) -> Bridge {
        let Self {
            mut transport,
            local,
            engine,
        } = self;
        let engine_address = engine
            .local_addr()
            .unwrap_or(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        if let Some(config) = transport.monitor.take() {
            let session = config.session;
            let started_at_ms = crate::fec_health::now_ms();
            tauri::async_runtime::spawn(async move {
                let result=standby::run(transport.socket,local,engine_address,config,stopped,move |snapshot| {
                    crate::fec_health::observe(&app,kind,session,"direct",started_at_ms,snapshot.fec,metrics.as_ref());
                    // Addresses stay in explicit connection details. Persist only
                    // bounded numeric path metrics, never probe keys or tokens.
                    if let Some(sender)=&metrics {
                        let active=snapshot.paths.iter().find(|path|path.endpoint==snapshot.active_endpoint);
                        let line=format!("DIRECT_PATH_HEALTH pathCount={} pathSwitches={} pathSendDrops={} pathRttMs={} pathLossPercent={}",snapshot.paths.len(),snapshot.switches,snapshot.local_send_drops,active.and_then(|p|p.median_ms).unwrap_or(-1.0),active.map_or(100.0,|p|p.loss_percent));
                        if let Some(sample)=crate::connection_report::parse_sample(line.as_bytes()) { let _=sender.try_send(sample); }
                        if let Some(sample)=router_sample(snapshot.router) {let _=sender.try_send(sample);}
                    }
                    let _=app.emit("direct-paths",serde_json::json!({"sessionId":session,"attemptId":attempt,"snapshot":snapshot}));
                }).await;
                if result.is_err() {
                    eprintln!("Direct UDP router stopped after a socket error");
                }
            });
        }
        // The caller immediately starts the native sidecar with this port.
        drop(engine);
        Bridge { stop: Some(stop) }
    }
}

fn router_sample(stats: standby::RouterStats) -> Option<crate::connection_report::Sample> {
    let line = format!(
        "DIRECT_ROUTER_HEALTH routerExternalReceived={} routerExternalProbes={} routerPeerPayloads={} routerExternalRejected={} routerNativeReceived={} routerNativeRejected={} routerToNative={} routerToPeer={} routerToNativeDrops={} routerToPeerDrops={} routerExternalReceiveErrors={} routerNativeReceiveErrors={} routerStopReason={}",
        stats.external_received,
        stats.external_probes,
        stats.peer_payloads,
        stats.external_rejected,
        stats.native_received,
        stats.native_rejected,
        stats.to_native,
        stats.to_peer,
        stats.to_native_drops,
        stats.to_peer_drops,
        stats.external_receive_errors,
        stats.native_receive_errors,
        stats.stop_reason,
    );
    crate::connection_report::parse_sample(line.as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    #[test]
    fn router_report_keeps_each_socket_boundary_distinct() {
        let sample = router_sample(standby::RouterStats {
            external_received: 7,
            external_probes: 4,
            peer_payloads: 2,
            external_rejected: 1,
            native_received: 5,
            native_rejected: 3,
            to_native: 1,
            to_peer: 4,
            to_native_drops: 1,
            to_peer_drops: 1,
            external_receive_errors: 2,
            native_receive_errors: 3,
            stop_reason: 2,
        })
        .unwrap();
        let values = serde_json::to_value(sample).unwrap();
        assert_eq!(values["source"], "DIRECT_ROUTER_HEALTH");
        let values = values["values"].as_object().unwrap();
        assert_eq!(values.len(), 13);
        assert_eq!(values["routerExternalReceived"], 7.0);
        assert_eq!(values["routerExternalProbes"], 4.0);
        assert_eq!(values["routerPeerPayloads"], 2.0);
        assert_eq!(values["routerToNative"], 1.0);
        assert_eq!(values["routerToPeer"], 4.0);
        assert_eq!(values["routerStopReason"], 2.0);
    }
    #[tokio::test]
    async fn original_nat_binding_remains_reserved_and_native_peer_is_loopback() {
        for kind in [EngineKind::Client, EngineKind::Host] {
            let socket = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
            let original = socket.local_addr().unwrap();
            let mut request:LaunchEngineRequest=serde_json::from_value(serde_json::json!({
                "kind":kind,"sessionId":uuid::Uuid::new_v4(),"codec":"h264","fps":60,"bitrateKbps":25000,"width":1920,"height":1080,"networkMode":"direct","address":"198.51.100.1","port":50000,"udpBindPort":original.port()
            })).unwrap();
            let prepared = Prepared::new(
                DirectTransport {
                    socket: Arc::clone(&socket),
                    monitor: None,
                },
                &mut request,
            )
            .unwrap();
            assert!(
                UdpSocket::bind(original).is_err(),
                "router must retain the NAT/punch source port"
            );
            let engine = prepared.engine.local_addr().unwrap();
            let local = prepared.local.local_addr().unwrap();
            if kind == EngineKind::Client {
                assert_eq!(request.port, Some(engine.port()));
                assert_eq!(request.udp_connect, Some(local.to_string()));
            } else {
                assert_eq!(request.udp_bind_port, Some(engine.port()));
                assert_eq!(request.address, Some("127.0.0.1".into()));
                assert_eq!(request.port, Some(local.port()));
            }
            assert!(
                !request.relay,
                "a local Direct router is not an Internet relay"
            );
        }
    }
}
