//! Mobile keeps the nominated socket alive and uses desktop's SNP2/standby/FEC
//! implementations. No mobile-specific authentication or NAT protocol is invented.
#![deny(unsafe_op_in_unsafe_fn)]
mod files;
use sanser_p2p::{
    CandidatePair, GathererConfig, P2pCandidate, PairState, check_connectivity_measured,
    gather_candidates_on_socket, make_pair_id, pair_priority, select_signaling_candidates, standby,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char},
    net::UdpSocket as StdUdp,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::UdpSocket,
    runtime::{Builder, Runtime},
    sync::{Notify, mpsc, oneshot},
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Request {
    Gather {
        attempt_id: Uuid,
        session_id: Uuid,
        generation: u32,
        stun_servers: Vec<String>,
    },
    Check {
        attempt_id: Uuid,
        device_id: Uuid,
        peer_device_id: Uuid,
        session_token: String,
        remote_candidates: Vec<P2pCandidate>,
    },
    Activate {
        attempt_id: Uuid,
    },
    Close {
        attempt_id: Uuid,
    },
}
struct State {
    candidates: Vec<P2pCandidate>,
    checking: bool,
    selected: Option<standby::Config>,
    stop: Option<oneshot::Sender<()>>,
    active: bool,
}
struct Route {
    session: Uuid,
    socket: Arc<UdpSocket>,
    cancelled: AtomicBool,
    cancel: Notify,
    state: Mutex<State>,
}
struct Manager {
    runtime: Runtime,
    routes: Mutex<HashMap<Uuid, Arc<Route>>>,
}
static MANAGER: OnceLock<Manager> = OnceLock::new();
fn manager() -> &'static Manager {
    MANAGER.get_or_init(|| Manager {
        runtime: Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("mobile P2P runtime"),
        routes: Mutex::new(HashMap::new()),
    })
}
fn lock<T>(value: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    value.lock().map_err(|_| "P2P state unavailable".into())
}
fn hash_device(id: Uuid) -> u64 {
    let mut h = Sha256::new();
    h.update(b"sanser-p2p-device-id-v1\0");
    h.update(id.as_bytes());
    let d = h.finalize();
    u64::from_be_bytes(d[..8].try_into().expect("eight digest bytes"))
}
fn probe_key(session: Uuid, credential: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"sanser-p2p-connectivity-probe-v1\0");
    h.update(session.as_bytes());
    h.update(credential.as_bytes());
    h.finalize().into()
}
fn gather_error_key(error: &sanser_p2p::P2pError) -> &'static str {
    match error {
        sanser_p2p::P2pError::Internal { reason }
            if reason.starts_with("Failed to list network interfaces:") =>
        {
            "p2p_interface_enumeration_failed"
        }
        sanser_p2p::P2pError::Internal { reason }
            if reason.starts_with("failed to collect a host candidate:") =>
        {
            "p2p_host_candidate_conflict"
        }
        _ => error.ui_key(),
    }
}
fn pairs(local: &[P2pCandidate], remote: &[P2pCandidate]) -> Vec<CandidatePair> {
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for a in local {
        for b in remote.iter().filter(|b| b.address.is_ipv4()) {
            let id = make_pair_id(&a.id, &b.id);
            if !seen.insert(id.clone()) {
                continue;
            }
            pairs.push(CandidatePair {
                pair_id: id,
                local: a.endpoint(),
                remote: b.endpoint(),
                local_candidate_id: a.id.clone(),
                remote_candidate_id: b.id.clone(),
                priority: pair_priority(b.priority, a.priority),
                state: PairState::Waiting,
            });
        }
    }
    pairs
}
impl Manager {
    fn route(&self, id: Uuid) -> Result<Arc<Route>, String> {
        lock(&self.routes)?
            .get(&id)
            .cloned()
            .ok_or("P2P attempt was closed".into())
    }
    fn close(&self, id: Uuid) -> Result<Value, String> {
        if let Some(route) = lock(&self.routes)?.remove(&id) {
            route.cancelled.store(true, Ordering::Release);
            route.cancel.notify_one();
            if let Some(stop) = lock(&route.state)?.stop.take() {
                let _ = stop.send(());
            }
        }
        Ok(json!({}))
    }
    fn request(&self, request: Request) -> Result<Value, String> {
        match request {
            Request::Close { attempt_id } => self.close(attempt_id),
            Request::Gather {
                attempt_id,
                session_id,
                generation,
                stun_servers,
            } => {
                if generation == 0
                    || stun_servers.len() > 3
                    || stun_servers.iter().any(|s| {
                        s.len() > 255
                            || !s.starts_with("stun:")
                            || s.contains(['\r', '\n', '@', '/', '?', '#'])
                    })
                {
                    return Err("Invalid STUN configuration".into());
                }
                let socket = self
                    .runtime
                    .block_on(UdpSocket::bind("0.0.0.0:0"))
                    .map_err(|error| {
                        format!("Unable to reserve Direct UDP socket ({:?})", error.kind())
                    })?;
                let route = Arc::new(Route {
                    session: session_id,
                    socket: Arc::new(socket),
                    cancelled: AtomicBool::new(false),
                    cancel: Notify::new(),
                    state: Mutex::new(State {
                        candidates: Vec::new(),
                        checking: false,
                        selected: None,
                        stop: None,
                        active: false,
                    }),
                });
                {
                    let mut routes = lock(&self.routes)?;
                    if routes.len() >= 4 || routes.contains_key(&attempt_id) {
                        return Err("P2P attempt capacity exceeded".into());
                    }
                    routes.insert(attempt_id, route.clone());
                }
                let result=self.runtime.block_on(async {
                    let (tx,mut rx)=mpsc::channel(32);
                    let drain=tokio::spawn(async move {while rx.recv().await.is_some(){}});
                    let config=GathererConfig{stun_servers,total_timeout:Duration::from_secs(3),port_mapping_enabled:false,ipv6_enabled:false,generation};
                    let gathered=tokio::select! {
                        _=route.cancel.notified()=>Err("Direct cancelled".to_owned()),
                        result=gather_candidates_on_socket(config,&route.socket,tx)=>result.map_err(|error|format!("Unable to gather Direct candidates ({})",gather_error_key(&error))),
                    };
                    drain.abort();gathered
                });
                match result {
                    Ok(result) if !route.cancelled.load(Ordering::Acquire) => {
                        let candidates = select_signaling_candidates(result.candidates);
                        if candidates.is_empty() {
                            self.close(attempt_id)?;
                            return Err("No Direct candidates available".into());
                        }
                        lock(&route.state)?.candidates = candidates.clone();
                        Ok(json!({"candidates":candidates,"stunSucceeded":result.stun_succeeded}))
                    }
                    Err(error) => {
                        self.close(attempt_id)?;
                        Err(error)
                    }
                    Ok(_) => {
                        self.close(attempt_id)?;
                        Err("Direct cancelled".into())
                    }
                }
            }
            Request::Check {
                attempt_id,
                device_id,
                peer_device_id,
                session_token,
                remote_candidates,
            } => {
                let credential = Zeroizing::new(session_token);
                if device_id == peer_device_id
                    || !(32..=256).contains(&credential.len())
                    || !credential.is_ascii()
                    || remote_candidates.is_empty()
                    || remote_candidates.len() > 16
                {
                    return Err("Invalid Direct connectivity request".into());
                }
                for c in &remote_candidates {
                    c.validate()
                        .map_err(|_| "Invalid remote Direct candidate")?;
                }
                let route = self.route(attempt_id)?;
                let local = {
                    let mut state = lock(&route.state)?;
                    if state.checking || state.active || state.selected.is_some() {
                        return Err("Direct check already started".into());
                    }
                    state.checking = true;
                    state.candidates.clone()
                };
                if route.cancelled.load(Ordering::Acquire) {
                    return Err("Direct cancelled".into());
                }
                let mut pairs = pairs(&local, &remote_candidates);
                if pairs.is_empty() {
                    return Err("No compatible Direct candidates".into());
                }
                let key = Zeroizing::new(probe_key(route.session, &credential));
                let selected=self.runtime.block_on(async{tokio::select! {
                    _=route.cancel.notified()=>Err("Direct cancelled".to_owned()),
                    result=check_connectivity_measured(&route.socket,&mut pairs,route.session,hash_device(device_id),hash_device(peer_device_id),&*key,false,Duration::from_secs(15))=>result.map_err(|_|"No verified Direct route".to_owned()),
                }})?;
                if route.cancelled.load(Ordering::Acquire) {
                    return Err("Direct cancelled".into());
                }
                let response = json!({"transport":"direct","probe":selected.probe,"remoteEndpoint":selected.pair.remote,"reason":selected.reason});
                lock(&route.state)?.selected = Some(standby::Config {
                    session: route.session,
                    local_device: hash_device(device_id),
                    peer_device: hash_device(peer_device_id),
                    key: *key,
                    controlling: false,
                    active: selected.pair.remote,
                    alternatives: remote_candidates
                        .iter()
                        .filter(|c| c.address.is_ipv4())
                        .map(P2pCandidate::endpoint)
                        .collect(),
                });
                Ok(response)
            }
            Request::Activate { attempt_id } => {
                let route = self.route(attempt_id)?;
                if route.cancelled.load(Ordering::Acquire) {
                    return Err("Direct cancelled".into());
                }
                self.runtime.block_on(async {
                    let config={let mut state=lock(&route.state)?;if state.active{return Err("Direct already active".into())}state.active=true;state.selected.take().ok_or("Direct was not nominated")?};
                    let local=UdpSocket::bind("127.0.0.1:0").await.map_err(|_|"Unable to bind Direct bridge")?;
                    let engine=StdUdp::bind("127.0.0.1:0").map_err(|_|"Unable to reserve native port")?;
                    let engine_address=engine.local_addr().map_err(|_|"Native port unavailable")?;
                    let endpoint=local.local_addr().map_err(|_|"Direct bridge unavailable")?;
                    let (stop,stopped)=oneshot::channel();lock(&route.state)?.stop=Some(stop);
                    let socket=route.socket.clone();
                    tokio::spawn(async move {let _=standby::run(socket,local,engine_address,config,stopped,|_|{}).await;});
                    // Only the loopback engine port is handed off. The public
                    // socket stays continuously bound through gather/check/media.
                    drop(engine);
                    Ok(json!({"endpoint":endpoint.to_string(),"localPort":engine_address.port(),"transport":"direct"}))
                })
            }
        }
    }
}

/// Run one JSON request. Close is nonblocking and may cancel a concurrent check.
///
/// # Safety
/// `request` must point to a live NUL-terminated UTF-8 string for this call.
/// Release the returned allocation once using `sanser_mobile_p2p_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sanser_mobile_p2p_request(request: *const c_char) -> *mut c_char {
    let answer = catch_unwind(AssertUnwindSafe(|| {
        if request.is_null() {
            return Err("Missing P2P request".to_owned());
        }
        // SAFETY: ownership and NUL termination are required by the C ABI.
        let raw = unsafe { CStr::from_ptr(request) }.to_bytes();
        if raw.len() > 65536 {
            return Err("P2P request too large".into());
        }
        let request = serde_json::from_slice(raw).map_err(|_| "Invalid P2P request".to_owned())?;
        manager().request(request)
    }))
    .unwrap_or_else(|_| Err("P2P operation failed".into()));
    let response = match answer {
        Ok(result) => json!({"ok":true,"result":result}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    CString::new(response.to_string())
        .expect("JSON escapes NUL")
        .into_raw()
}
/// Release a JSON response returned by this library.
///
/// # Safety
/// `pointer` must be null or a still-owned allocation returned by request above.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sanser_mobile_p2p_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        // SAFETY: the caller returns the allocation exactly once.
        drop(unsafe { CString::from_raw(pointer) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_manager() -> Arc<Manager> {
        Arc::new(Manager {
            runtime: Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap(),
            routes: Mutex::new(HashMap::new()),
        })
    }
    fn gather(m: &Manager, id: Uuid, session: Uuid) -> Vec<P2pCandidate> {
        let value = m
            .request(Request::Gather {
                attempt_id: id,
                session_id: session,
                generation: 1,
                stun_servers: vec![],
            })
            .unwrap();
        serde_json::from_value(value["candidates"].clone()).unwrap()
    }
    #[test]
    fn nominates_with_desktop_and_keeps_socket_for_bidirectional_media() {
        let m = test_manager();
        let attempt = Uuid::new_v4();
        let session = Uuid::new_v4();
        let mobile_id = Uuid::new_v4();
        let host_id = Uuid::new_v4();
        let token = "verified-mobile-desktop-test-credential";
        let local = gather(&m, attempt, session);
        let route = m.route(attempt).unwrap();
        let reserved = route.socket.local_addr().unwrap().port();
        let host = m.runtime.block_on(UdpSocket::bind("0.0.0.0:0")).unwrap();
        // A real interface address makes this exercise routing, not a synthetic
        // loopback candidate (loopback is deliberately invalid in signaling).
        let mut host_candidate = local
            .iter()
            .find(|c| c.candidate_type == sanser_p2p::CandidateType::Host)
            .unwrap()
            .clone();
        host_candidate.id = "desktop-host".into();
        host_candidate.port = host.local_addr().unwrap().port();
        let mut host_pairs = Vec::new();
        for candidate in &local {
            host_pairs.push(CandidatePair {
                pair_id: make_pair_id(&host_candidate.id, &candidate.id),
                local: host_candidate.endpoint(),
                remote: candidate.endpoint(),
                local_candidate_id: host_candidate.id.clone(),
                remote_candidate_id: candidate.id.clone(),
                priority: pair_priority(host_candidate.priority, candidate.priority),
                state: PairState::Waiting,
            });
        }
        let desktop = m.runtime.spawn(async move {
            let selected = check_connectivity_measured(
                &host,
                &mut host_pairs,
                session,
                hash_device(host_id),
                hash_device(mobile_id),
                &probe_key(session, token),
                true,
                Duration::from_secs(5),
            )
            .await
            .unwrap();
            (host, selected)
        });
        let selected = m
            .request(Request::Check {
                attempt_id: attempt,
                device_id: mobile_id,
                peer_device_id: host_id,
                session_token: token.into(),
                remote_candidates: vec![host_candidate.clone()],
            })
            .unwrap();
        assert_eq!(selected["transport"], "direct");
        let (host, desktop) = m.runtime.block_on(desktop).unwrap();
        assert_eq!(desktop.pair.remote.port(), reserved);
        assert_eq!(route.socket.local_addr().unwrap().port(), reserved);
        let bridge = m
            .request(Request::Activate {
                attempt_id: attempt,
            })
            .unwrap();
        let endpoint = bridge["endpoint"].as_str().unwrap();
        let engine = StdUdp::bind((
            std::net::Ipv4Addr::LOCALHOST,
            bridge["localPort"].as_u64().unwrap() as u16,
        ))
        .unwrap();
        engine
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        engine.send_to(b"mobile-input", endpoint).unwrap();
        m.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                let mut bytes = [0; 2048];
                loop {
                    let (n, source) = host.recv_from(&mut bytes).await.unwrap();
                    if &bytes[..n] == b"mobile-input" {
                        assert_eq!(source.port(), reserved);
                        host.send_to(b"desktop-media", source).await.unwrap();
                        break;
                    }
                }
            })
            .await
            .unwrap();
        });
        let mut bytes = [0; 2048];
        let (n, source) = engine.recv_from(&mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"desktop-media");
        assert_eq!(source, endpoint.parse().unwrap());
        m.close(attempt).unwrap();
        m.close(attempt).unwrap();
    }
    #[test]
    fn closing_an_inflight_check_cancels_without_waiting_for_timeout() {
        let m = test_manager();
        let attempt = Uuid::new_v4();
        let local = gather(&m, attempt, Uuid::new_v4());
        let mut remote = local[0].clone();
        remote.id = "unresponsive-desktop".into();
        let silent = StdUdp::bind("0.0.0.0:0").unwrap();
        remote.port = silent.local_addr().unwrap().port();
        let runner = m.clone();
        let check = std::thread::spawn(move || {
            runner.request(Request::Check {
                attempt_id: attempt,
                device_id: Uuid::new_v4(),
                peer_device_id: Uuid::new_v4(),
                session_token: "cancel-mobile-desktop-test-credential".into(),
                remote_candidates: vec![remote],
            })
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !lock(&m.route(attempt).unwrap().state).unwrap().checking {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let start = std::time::Instant::now();
        m.close(attempt).unwrap();
        assert!(check.join().unwrap().is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn rejects_bad_requests_and_close_is_idempotent() {
        let id = Uuid::new_v4();
        let m = manager();
        assert!(m.close(id).is_ok());
        assert!(m.close(id).is_ok());
        assert!(
            m.request(Request::Gather {
                attempt_id: id,
                session_id: Uuid::new_v4(),
                generation: 0,
                stun_servers: vec![]
            })
            .is_err()
        );
        assert!(m.request(Request::Activate { attempt_id: id }).is_err());
    }
    #[test]
    fn mobile_json_boundary_accepts_real_gather_request() {
        let attempt = Uuid::new_v4();
        let request = json!({"operation":"gather","attemptId":attempt.to_string(),"sessionId":Uuid::new_v4().to_string(),"generation":1793519996,"stunServers":[]});
        let value = ffi_json(request);
        let _ = ffi_json(json!({"operation":"close","attemptId":attempt.to_string()}));
        assert_eq!(value["ok"], true, "{value}");
        assert!(!value["result"]["candidates"].as_array().unwrap().is_empty());
    }
    fn ffi_json(request: Value) -> Value {
        let text = CString::new(request.to_string()).unwrap();
        // SAFETY: the request remains alive; return storage is freed exactly once.
        unsafe {
            let result = sanser_mobile_p2p_request(text.as_ptr());
            assert!(!result.is_null());
            let value = serde_json::from_slice(CStr::from_ptr(result).to_bytes()).unwrap();
            sanser_mobile_p2p_free(result);
            value
        }
    }
    #[test]
    fn native_boundary_returns_owned_json() {
        let invalid = CString::new("{}").unwrap();
        // SAFETY: request remains alive and the response is freed exactly once.
        unsafe {
            let result = sanser_mobile_p2p_request(invalid.as_ptr());
            assert!(!result.is_null());
            let json: Value = serde_json::from_slice(CStr::from_ptr(result).to_bytes()).unwrap();
            assert_eq!(json["ok"], false);
            sanser_mobile_p2p_free(result);
        }
    }
}
