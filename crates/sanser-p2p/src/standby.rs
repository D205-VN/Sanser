//! Authenticated, continuously measured alternatives on the nominated UDP socket.
//! Opaque native datagrams are forwarded without changing media keys/generation.
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use serde::Serialize;
use sha2::Sha256;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, sync::oneshot};

const MAGIC: &[u8; 4] = b"SRP1";
const SIZE: usize = 96;
const MAX_PATHS: usize = 20;
const FRESH: Duration = Duration::from_secs(8);
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const CHALLENGE_TTL: Duration = Duration::from_secs(3);
const COOLDOWN: Duration = Duration::from_secs(15);

pub struct Config {
    pub session: uuid::Uuid,
    pub local_device: u64,
    pub peer_device: u64,
    pub key: [u8; 32],
    pub controlling: bool,
    pub active: SocketAddr,
    pub alternatives: Vec<SocketAddr>,
}
impl Drop for Config {
    fn drop(&mut self) {
        self.key.fill(0);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathSnapshot {
    pub endpoint: SocketAddr,
    pub samples: usize,
    pub observations: usize,
    pub median_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loss_percent: f64,
    pub last_response_age_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub active_endpoint: SocketAddr,
    pub switches: u64,
    pub rollbacks: u64,
    pub pending: bool,
    pub paths: Vec<PathSnapshot>,
    pub local_send_drops: u64,
    pub fec: crate::fec::Stats,
    pub router: RouterStats,
}

/// Cumulative socket-boundary counters, not proof of native authentication,
/// decoding or delivery beyond a successful local UDP send call.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterStats {
    pub external_received: u64,
    pub external_probes: u64,
    pub peer_payloads: u64,
    pub external_rejected: u64,
    pub native_received: u64,
    pub native_rejected: u64,
    pub to_native: u64,
    pub to_peer: u64,
    pub to_native_drops: u64,
    pub to_peer_drops: u64,
    pub external_receive_errors: u64,
    pub native_receive_errors: u64,
    /// 0 running, 1 owner stopped/dropped, 2 external receive fault, 3 local receive fault.
    pub stop_reason: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Ping = 1,
    Pong = 2,
    Switch = 3,
    Switched = 4,
}
#[derive(Clone, Copy)]
struct Message {
    kind: Kind,
    id: u64,
    token: u64,
    serial: u64,
}
fn encode(config: &Config, m: Message) -> [u8; SIZE] {
    let mut b = [0; SIZE];
    b[..4].copy_from_slice(MAGIC);
    b[4] = 1;
    b[5] = m.kind as u8;
    b[8..24].copy_from_slice(config.session.as_bytes());
    b[24..32].copy_from_slice(&config.local_device.to_be_bytes());
    b[32..40].copy_from_slice(&m.id.to_be_bytes());
    b[40..48].copy_from_slice(&m.token.to_be_bytes());
    b[48..56].copy_from_slice(&m.serial.to_be_bytes());
    let mut mac = Hmac::<Sha256>::new_from_slice(&config.key).expect("fixed HMAC key");
    mac.update(&b[..64]);
    b[64..].copy_from_slice(&mac.finalize().into_bytes());
    b
}
fn decode(config: &Config, b: &[u8]) -> Option<Message> {
    if b.len() != SIZE
        || &b[..4] != MAGIC
        || b[4] != 1
        || b[6..8] != [0; 2]
        || b[56..64] != [0; 8]
        || &b[8..24] != config.session.as_bytes()
        || b[24..32] != config.peer_device.to_be_bytes()
    {
        return None;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(&config.key).ok()?;
    mac.update(&b[..64]);
    mac.verify_slice(&b[64..]).ok()?;
    let kind = match b[5] {
        1 => Kind::Ping,
        2 => Kind::Pong,
        3 => Kind::Switch,
        4 => Kind::Switched,
        _ => return None,
    };
    Some(Message {
        kind,
        id: u64::from_be_bytes(b[32..40].try_into().ok()?),
        token: u64::from_be_bytes(b[40..48].try_into().ok()?),
        serial: u64::from_be_bytes(b[48..56].try_into().ok()?),
    })
}

#[derive(Default)]
struct Path {
    history: VecDeque<(Instant, Option<f64>)>,
    last_response: Option<Instant>,
    challenge: Option<(Message, Instant)>,
    issued: VecDeque<(Message, Instant)>,
}
impl Path {
    fn sample(&mut self, value: Option<f64>, now: Instant) {
        if value.is_some() {
            self.last_response = Some(now);
        }
        self.history.push_back((now, value));
        while self.history.len() > 8 {
            self.history.pop_front();
        }
    }
    fn snapshot(&self, endpoint: SocketAddr, now: Instant) -> PathSnapshot {
        let history: Vec<_> = self
            .history
            .iter()
            .filter(|(at, _)| now.saturating_duration_since(*at) <= FRESH)
            .collect();
        let mut rtts: Vec<_> = history.iter().filter_map(|(_, rtt)| *rtt).collect();
        rtts.sort_by(f64::total_cmp);
        let midpoint = median(&rtts);
        let mut deviations: Vec<_> = rtts
            .iter()
            .map(|rtt| (rtt - midpoint.unwrap_or_default()).abs())
            .collect();
        deviations.sort_by(f64::total_cmp);
        PathSnapshot {
            endpoint,
            samples: rtts.len(),
            observations: history.len(),
            median_ms: midpoint,
            jitter_ms: median(&deviations),
            loss_percent: if history.is_empty() {
                100.0
            } else {
                (history.len() - rtts.len()) as f64 * 100.0 / history.len() as f64
            },
            last_response_age_ms: self.last_response.map(|at| {
                now.saturating_duration_since(at)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64
            }),
        }
    }
    fn verified(&self, now: Instant) -> bool {
        self.last_response
            .is_some_and(|at| now.saturating_duration_since(at) <= FRESH)
    }
}
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some((values[(values.len() - 1) / 2] + values[values.len() / 2]) / 2.0)
    }
}
fn score(s: &PathSnapshot) -> Option<f64> {
    Some(s.median_ms? + 2.0 * s.jitter_ms? + s.loss_percent)
}

struct PendingSwitch {
    address: SocketAddr,
    message: Message,
    began: Instant,
    sent: Option<Instant>,
}
struct Monitor {
    config: Config,
    paths: Vec<(SocketAddr, Path)>,
    outstanding: HashMap<u64, (SocketAddr, Instant)>,
    pending: Option<PendingSwitch>,
    accepted: Option<(SocketAddr, Message)>,
    next_serial: u64,
    switches: u64,
    rollbacks: u64,
    previous: Option<SocketAddr>,
    began: Instant,
    last_switch: Option<Instant>,
    vote: Option<(SocketAddr, Instant, u8)>,
    cursor: usize,
    drops: u64,
}
impl Monitor {
    fn new(config: Config, now: Instant) -> Self {
        let mut addresses = vec![config.active];
        for &address in &config.alternatives {
            if address.is_ipv4()
                && address.port() != 0
                && !addresses.contains(&address)
                && addresses.len() < 17
            {
                addresses.push(address);
            }
        }
        Self {
            config,
            paths: addresses
                .into_iter()
                .map(|address| (address, Path::default()))
                .collect(),
            outstanding: HashMap::new(),
            pending: None,
            accepted: None,
            next_serial: 0,
            switches: 0,
            rollbacks: 0,
            previous: None,
            began: now,
            last_switch: None,
            vote: None,
            cursor: 0,
            drops: 0,
        }
    }
    fn snapshot(&self, now: Instant) -> Snapshot {
        Snapshot {
            active_endpoint: self.config.active,
            switches: self.switches,
            rollbacks: self.rollbacks,
            pending: self.pending.is_some(),
            paths: self
                .paths
                .iter()
                .map(|(a, p)| p.snapshot(*a, now))
                .collect(),
            local_send_drops: self.drops,
            fec: crate::fec::Stats::default(),
            router: RouterStats::default(),
        }
    }
    fn transmit(&mut self, socket: &UdpSocket, address: SocketAddr, message: Message) {
        if socket
            .try_send_to(&encode(&self.config, message), address)
            .is_err()
        {
            self.drops += 1;
        }
    }
    fn allowed_source(&self, source: SocketAddr) -> bool {
        self.paths
            .iter()
            .any(|(address, _)| address.ip() == source.ip())
            || self
                .config
                .alternatives
                .iter()
                .any(|a| a.ip() == source.ip())
    }
    fn path(&mut self, source: SocketAddr) -> Option<&mut Path> {
        if !self.paths.iter().any(|(a, _)| *a == source) && self.paths.len() < MAX_PATHS {
            self.paths.push((source, Path::default()));
        }
        self.paths
            .iter_mut()
            .find(|(a, _)| *a == source)
            .map(|(_, p)| p)
    }
    fn receive(
        &mut self,
        socket: &UdpSocket,
        bytes: &[u8],
        source: SocketAddr,
        now: Instant,
    ) -> bool {
        if !bytes.starts_with(MAGIC) {
            return source == self.config.active
                || self
                    .paths
                    .iter()
                    .any(|(a, p)| *a == source && p.verified(now));
        }
        let Some(message) = decode(&self.config, bytes) else {
            return false;
        };
        if !self.allowed_source(source) {
            return false;
        }
        match message.kind {
            Kind::Ping => {
                let Some(path) = self.path(source) else {
                    return false;
                };
                if path.issued.back().is_some_and(|(_, at)| {
                    now.saturating_duration_since(*at) < Duration::from_millis(200)
                }) {
                    return false;
                }
                let reply = Message {
                    kind: Kind::Pong,
                    id: message.id,
                    token: OsRng.next_u64(),
                    serial: 0,
                };
                path.issued
                    .retain(|(_, at)| now.saturating_duration_since(*at) <= CHALLENGE_TTL);
                path.issued.push_back((reply, now));
                while path.issued.len() > 8 {
                    path.issued.pop_front();
                }
                self.transmit(socket, source, reply);
            }
            Kind::Pong => {
                let Some(&(address, sent)) = self.outstanding.get(&message.id) else {
                    return false;
                };
                if address != source || now.saturating_duration_since(sent) > PROBE_TIMEOUT {
                    return false;
                }
                self.outstanding.remove(&message.id);
                if let Some(path) = self.path(source) {
                    path.sample(
                        Some(now.saturating_duration_since(sent).as_secs_f64() * 1000.0),
                        now,
                    );
                    path.challenge = Some((message, now));
                }
            }
            Kind::Switch if !self.config.controlling => {
                if self.config.active == source
                    && self.accepted.is_some_and(|(address, m)| {
                        address == source
                            && m.id == message.id
                            && m.token == message.token
                            && m.serial == message.serial
                    })
                {
                    self.transmit(
                        socket,
                        source,
                        Message {
                            kind: Kind::Switched,
                            ..message
                        },
                    );
                    return false;
                }
                let serial = self.accepted.map_or(0, |(_, m)| m.serial);
                let valid = self.path(source).is_some_and(|path| {
                    path.verified(now)
                        && path.issued.iter().any(|(challenge, at)| {
                            challenge.id == message.id
                                && challenge.token == message.token
                                && now.saturating_duration_since(*at) <= CHALLENGE_TTL
                        })
                });
                if valid && message.serial > serial {
                    self.activate(source, now);
                    self.accepted = Some((source, message));
                    self.transmit(
                        socket,
                        source,
                        Message {
                            kind: Kind::Switched,
                            ..message
                        },
                    );
                }
            }
            Kind::Switched if self.config.controlling => {
                if self.pending.as_ref().is_some_and(|p| {
                    p.address == source
                        && p.message.id == message.id
                        && p.message.token == message.token
                        && p.message.serial == message.serial
                        && now.saturating_duration_since(p.began) < Duration::from_secs(2)
                }) {
                    self.activate(source, now);
                    self.pending = None;
                }
            }
            _ => {}
        }
        false
    }
    fn activate(&mut self, address: SocketAddr, now: Instant) {
        if self.config.active != address {
            self.previous = Some(self.config.active);
            self.config.active = address;
            self.switches += 1;
            self.last_switch = Some(now);
            self.vote = None;
        }
    }
    fn consider(&mut self, now: Instant) {
        if !self.config.controlling
            || self.pending.is_some()
            || now.saturating_duration_since(self.began) < Duration::from_secs(3)
        {
            return;
        }
        let snapshots = self.snapshot(now);
        let Some(active) = snapshots
            .paths
            .iter()
            .find(|p| p.endpoint == self.config.active)
        else {
            return;
        };
        let active_lost = active.observations >= 3
            && active.last_response_age_ms.is_none_or(|age| age > 3000)
            && active.loss_percent >= 50.0;
        if !active_lost
            && self
                .last_switch
                .is_some_and(|at| now.saturating_duration_since(at) < COOLDOWN)
        {
            return;
        }
        let active_score = score(active);
        let eligible: Vec<_> = snapshots
            .paths
            .iter()
            .filter(|p| {
                p.endpoint != self.config.active
                    && p.samples >= 3
                    && p.loss_percent <= 20.0
                    && p.last_response_age_ms
                        .is_some_and(|age| age <= self.response_fresh_ms())
            })
            .filter(|p| {
                active_lost
                    || (active.samples >= 3
                        && active_score.is_some_and(|old| {
                            score(p).is_some_and(|new| old - new >= 15.0_f64.max(old * 0.25))
                        }))
            })
            .collect();
        // Finish validating a still-qualified path instead of resetting votes
        // whenever two fast alternatives exchange places by a fraction of a ms.
        let best = self
            .vote
            .and_then(|(address, _, _)| eligible.iter().copied().find(|p| p.endpoint == address))
            .or_else(|| {
                eligible.into_iter().min_by(|a, b| {
                    score(a)
                        .unwrap_or(f64::MAX)
                        .total_cmp(&score(b).unwrap_or(f64::MAX))
                })
            });
        let Some(best) = best else {
            self.vote = None;
            return;
        };
        let Some(path) = self
            .paths
            .iter()
            .find(|(a, _)| *a == best.endpoint)
            .map(|(_, p)| p)
        else {
            return;
        };
        let Some((challenge, at)) = path
            .challenge
            .filter(|(_, at)| now.saturating_duration_since(*at) <= CHALLENGE_TTL)
        else {
            return;
        };
        let count = match self.vote {
            Some((address, last, count)) if address == best.endpoint => {
                if at > last {
                    count.saturating_add(1)
                } else {
                    count
                }
            }
            _ => 1,
        };
        self.vote = Some((best.endpoint, at, count));
        if count >= 3 {
            self.next_serial += 1;
            self.pending = Some(PendingSwitch {
                address: best.endpoint,
                message: Message {
                    kind: Kind::Switch,
                    serial: self.next_serial,
                    ..challenge
                },
                began: now,
                sent: None,
            });
        }
    }
    fn response_fresh_ms(&self) -> u64 {
        // A full round can take 2.5s with twenty paths. Evidence must not
        // expire halfway through its own bounded probe scheduling cycle.
        self.paths.len().div_ceil(4) as u64 * 500 + 1500
    }
    fn rollback_if_lost(&mut self, now: Instant) {
        if self.config.controlling {
            return;
        }
        let snapshot = self.snapshot(now);
        let Some(active) = snapshot
            .paths
            .iter()
            .find(|p| p.endpoint == self.config.active)
        else {
            return;
        };
        if active.observations < 3
            || active.loss_percent < 50.0
            || active.last_response_age_ms.is_some_and(|age| age <= 3000)
        {
            return;
        }
        if let Some(previous) = self
            .previous
            .and_then(|address| snapshot.paths.iter().find(|p| p.endpoint == address))
            && previous.samples >= 3
            && previous.loss_percent <= 20.0
            && previous
                .last_response_age_ms
                .is_some_and(|age| age <= self.response_fresh_ms())
        {
            self.activate(previous.endpoint, now);
            self.rollbacks += 1;
        }
    }
    fn tick(&mut self, socket: &UdpSocket, now: Instant) {
        let expired: Vec<_> = self
            .outstanding
            .iter()
            .filter(|(_, (_, sent))| now.saturating_duration_since(*sent) >= PROBE_TIMEOUT)
            .map(|(&id, &(a, _))| (id, a))
            .collect();
        for (id, address) in expired {
            self.outstanding.remove(&id);
            if let Some(path) = self.path(address) {
                path.sample(None, now);
            }
        }
        // Four requests per 500ms tick, <=20 endpoints including learned NAT ports.
        for _ in 0..4.min(self.paths.len()) {
            let address = self.paths[self.cursor % self.paths.len()].0;
            self.cursor += 1;
            if self.outstanding.len() >= 64 {
                break;
            }
            let id = OsRng.next_u64();
            self.outstanding.insert(id, (address, now));
            self.transmit(
                socket,
                address,
                Message {
                    kind: Kind::Ping,
                    id,
                    token: 0,
                    serial: 0,
                },
            );
        }
        self.rollback_if_lost(now);
        self.consider(now);
        if self
            .pending
            .as_ref()
            .is_some_and(|p| now.saturating_duration_since(p.began) >= Duration::from_secs(2))
        {
            self.pending = None;
            self.vote = None;
            self.last_switch = Some(now);
        }
        let send = self.pending.as_mut().and_then(|p| {
            if p.sent
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_millis(250))
            {
                p.sent = Some(now);
                Some((p.address, p.message))
            } else {
                None
            }
        });
        if let Some((address, message)) = send {
            self.transmit(socket, address, message);
        }
    }
}

/// A Rust UDP router, not a decoder/encoder or a UI media callback. Both native
/// protocols retain their own authentication, replay, priority and repair rules.
/// The peer must support SRP1 before an alternative can ever be activated.
pub async fn run(
    external: Arc<UdpSocket>,
    local: UdpSocket,
    engine: SocketAddr,
    config: Config,
    mut stop: oneshot::Receiver<()>,
    observe: impl Fn(Snapshot) + Send + 'static,
) -> std::io::Result<()> {
    let mut fec = crate::fec::Fec::new(
        *config.session.as_bytes(),
        config.local_device,
        config.peer_device,
        config.key,
        OsRng.next_u64(),
        1400,
        28,
        Instant::now(),
    );
    let mut monitor = Monitor::new(config, Instant::now());
    let mut incoming = vec![0; 65536];
    let mut outgoing = vec![0; 65536];
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut report = Instant::now();
    let mut fec_route = monitor.config.active;
    let mut baseline_rtt: Option<f64> = None;
    let mut router = RouterStats::default();
    observe(monitor.snapshot(Instant::now()));
    let result = loop {
        tokio::select! {
            _=&mut stop=>{router.stop_reason=1;break Ok(());},
            _=tick.tick()=> {
                let now=Instant::now();monitor.tick(&external,now);
                if fec_route!=monitor.config.active {fec.path_changed(now);fec_route=monitor.config.active;baseline_rtt=None;}
                let mut snapshot=monitor.snapshot(now);
                if let Some(rtt)=snapshot.paths.iter().find(|p|p.endpoint==snapshot.active_endpoint && p.samples>=3).and_then(|p|p.median_ms) {
                    let minimum=baseline_rtt.get_or_insert(rtt);*minimum=minimum.min(rtt);
                    if rtt>*minimum+20.0_f64.max(*minimum*0.5) {fec.congested(now);}
                }
                if now.duration_since(report)>=Duration::from_secs(2) {snapshot.fec=fec.stats(now);snapshot.router=router;observe(snapshot);report=now;}
            },
            _=tokio::time::sleep(fec.next_tick(Instant::now()))=> {
                let now=Instant::now();
                for packet in fec.tick(now) {if external.try_send_to(&packet,monitor.config.active).is_err() {monitor.drops+=1;fec.local_drop(now);}}
            },
            result=external.recv_from(&mut incoming)=> {
                let (n,source)=match result {
                    Ok(value)=>value,
                    Err(error)=>{
                        router.external_receive_errors+=1;
                        if matches!(error.kind(),std::io::ErrorKind::ConnectionRefused|std::io::ErrorKind::ConnectionReset) {continue;}
                        router.stop_reason=2;break Err(error);
                    }
                };
                let now=Instant::now();
                router.external_received+=1;
                let probe=incoming[..n].starts_with(MAGIC);
                if probe {router.external_probes+=1;}
                if monitor.receive(&external,&incoming[..n],source,now) {
                    // Originals reach native before optional recovery work.
                    if !crate::fec::is_packet(&incoming[..n]) {
                        router.peer_payloads+=1;
                        if local.try_send_to(&incoming[..n],engine).is_err() {router.to_native_drops+=1;monitor.drops+=1;fec.local_drop(now);}
                        else {router.to_native+=1;}
                    }
                    let (_,recovered)=fec.receive(&incoming[..n],now);
                    for packet in recovered {if local.try_send_to(&packet,engine).is_err() {router.to_native_drops+=1;monitor.drops+=1;fec.local_drop(now);}else {router.to_native+=1;}}
                }else if !probe {router.external_rejected+=1;}
            },
            result=local.recv_from(&mut outgoing)=> {
                let (n,source)=match result {
                    Ok(value)=>value,
                    Err(error)=>{
                        router.native_receive_errors+=1;
                        if matches!(error.kind(),std::io::ErrorKind::ConnectionRefused|std::io::ErrorKind::ConnectionReset) {continue;}
                        router.stop_reason=3;break Err(error);
                    }
                };
                if source!=engine {router.native_rejected+=1;continue;}
                router.native_received+=1;
                let now=Instant::now();
                if external.try_send_to(&outgoing[..n],monitor.config.active).is_err() {router.to_peer_drops+=1;monitor.drops+=1;fec.local_drop(now);}
                else {
                    router.to_peer+=1;
                    if let Some(parity)=fec.outgoing(&outgoing[..n],now) && external.try_send_to(&parity,monitor.config.active).is_err() {monitor.drops+=1;fec.local_drop(now);}
                }
            },
        }
    };
    // Retain a final sample even if startup failed before the periodic report.
    let now = Instant::now();
    let mut snapshot = monitor.snapshot(now);
    snapshot.fec = fec.stats(now);
    snapshot.router = router;
    observe(snapshot);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn config(active: SocketAddr, alternative: SocketAddr, controlling: bool) -> Config {
        Config {
            session: uuid::Uuid::from_u128(42),
            local_device: if controlling { 1 } else { 2 },
            peer_device: if controlling { 2 } else { 1 },
            key: [7; 32],
            controlling,
            active,
            alternatives: vec![alternative],
        }
    }
    fn add_samples(m: &mut Monitor, address: SocketAddr, rtts: &[f64], now: Instant) {
        for &rtt in rtts {
            m.path(address).unwrap().sample(Some(rtt), now);
        }
        m.path(address).unwrap().challenge = Some((
            Message {
                kind: Kind::Pong,
                id: 5,
                token: 9,
                serial: 0,
            },
            now,
        ));
    }
    #[test]
    fn authenticated_wire_rejects_wrong_peer_session_key_and_tampering() {
        let address = "127.0.0.1:12345".parse().unwrap();
        let left = config(address, address, true);
        let mut right = config(address, address, false);
        let message = Message {
            kind: Kind::Ping,
            id: 777,
            token: 0,
            serial: 0,
        };
        let packet = encode(&left, message);
        assert_eq!(decode(&right, &packet).unwrap().id, 777);
        assert!(decode(&left, &packet).is_none());
        right.key = [8; 32];
        assert!(decode(&right, &packet).is_none());
        right.key = left.key;
        right.session = uuid::Uuid::new_v4();
        assert!(decode(&right, &packet).is_none());
        right.session = left.session;
        let mut packet = packet;
        packet[48] ^= 1;
        assert!(decode(&right, &packet).is_none());
    }
    #[test]
    fn sustained_measured_improvement_not_a_single_fast_sample_triggers_switch() {
        let slow = "127.0.0.1:50000".parse().unwrap();
        let fast = "127.0.0.1:50001".parse().unwrap();
        let began = Instant::now();
        let now = began + Duration::from_secs(4);
        let mut monitor = Monitor::new(config(slow, fast, true), began);
        add_samples(
            &mut monitor,
            slow,
            &[198.0, 204.0, 199.0, 202.0, 201.0],
            now,
        );
        add_samples(&mut monitor, fast, &[13.0], now);
        monitor.consider(now);
        assert!(monitor.pending.is_none());
        add_samples(&mut monitor, fast, &[12.0, 14.0], now);
        monitor.consider(now);
        monitor.consider(now);
        assert!(
            monitor.pending.is_none(),
            "repeated evaluation must not count the same probe twice"
        );
        for step in 1..=2 {
            let at = now + Duration::from_millis(step * 500);
            add_samples(&mut monitor, fast, &[13.0], at);
            monitor.consider(at);
        }
        assert_eq!(monitor.pending.as_ref().unwrap().address, fast);
        assert_eq!(
            monitor.config.active, slow,
            "do not switch until authenticated peer ACK"
        );
        monitor.activate(fast, now);
        monitor.pending = None;
        add_samples(&mut monitor, fast, &[200.0; 8], now);
        add_samples(&mut monitor, slow, &[1.0; 8], now);
        monitor.consider(now + Duration::from_secs(1));
        assert!(
            monitor.pending.is_none(),
            "cooldown prevents route flapping"
        );
    }
    #[test]
    fn stale_lossy_and_insignificantly_better_routes_do_not_replace_working_path() {
        let a = "127.0.0.1:50000".parse().unwrap();
        let b = "127.0.0.1:50001".parse().unwrap();
        let began = Instant::now();
        let now = began + Duration::from_secs(4);
        let mut monitor = Monitor::new(config(a, b, true), began);
        add_samples(&mut monitor, a, &[20.0; 5], now);
        add_samples(&mut monitor, b, &[15.0; 5], now);
        for step in 0..5 {
            let at = now + Duration::from_millis(step * 500);
            add_samples(&mut monitor, b, &[15.0], at);
            monitor.consider(at);
        }
        assert!(monitor.pending.is_none());
        add_samples(&mut monitor, a, &[200.0; 8], now);
        add_samples(&mut monitor, b, &[10.0; 4], now);
        for _ in 0..4 {
            monitor.path(b).unwrap().sample(None, now);
        }
        monitor.consider(now);
        assert!(
            monitor.pending.is_none(),
            "low RTT is insufficient when half the probes are lost"
        );
        monitor.consider(now + Duration::from_secs(9));
        assert!(
            monitor.pending.is_none(),
            "expired samples cannot authorize migration"
        );
        assert_eq!(monitor.config.active, a);
    }
    #[tokio::test]
    async fn switching_requires_fresh_return_path_challenge_and_matching_ack() {
        let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let active = "127.0.0.1:50000".parse().unwrap();
        let other = "127.0.0.1:50001".parse().unwrap();
        let now = Instant::now();
        let mut follower = Monitor::new(config(active, other, false), now);
        let controller = config(active, other, true);
        let switch = Message {
            kind: Kind::Switch,
            id: 11,
            token: 12345,
            serial: 1,
        };
        let bytes = encode(&controller, switch);
        follower.receive(&local, &bytes, other, now);
        assert_eq!(follower.config.active, active);
        follower.path(other).unwrap().sample(Some(10.0), now);
        follower.path(other).unwrap().issued.push_back((
            Message {
                kind: Kind::Pong,
                ..switch
            },
            now,
        ));
        follower.receive(&local, &bytes, other, now + Duration::from_secs(4));
        assert_eq!(follower.config.active, active, "old challenge replay");
        follower.path(other).unwrap().issued.push_back((
            Message {
                kind: Kind::Pong,
                ..switch
            },
            now + Duration::from_secs(4),
        ));
        follower.receive(&local, &bytes, other, now + Duration::from_secs(4));
        assert_eq!(follower.config.active, other);
        let switches = follower.switches;
        follower.receive(&local, &bytes, other, now + Duration::from_secs(4));
        assert_eq!(
            follower.switches, switches,
            "duplicate request is idempotent"
        );
        let mut leader = Monitor::new(controller, now);
        leader.pending = Some(PendingSwitch {
            address: other,
            message: switch,
            began: now,
            sent: Some(now),
        });
        let reply = encode(
            &follower.config,
            Message {
                kind: Kind::Switched,
                token: 999,
                ..switch
            },
        );
        leader.receive(&local, &reply, other, now);
        assert_eq!(leader.config.active, active);
        let reply = encode(
            &follower.config,
            Message {
                kind: Kind::Switched,
                ..switch
            },
        );
        leader.receive(&local, &reply, other, now);
        assert_eq!(leader.config.active, other);
        assert!(leader.pending.is_none());
    }
    #[test]
    fn many_candidates_do_not_expire_votes_between_bounded_probe_rounds() {
        let slow = "127.0.0.1:50000".parse().unwrap();
        let fast = "127.0.0.1:50001".parse().unwrap();
        let began = Instant::now();
        let now = began + Duration::from_secs(4);
        let mut monitor = Monitor::new(config(slow, fast, true), began);
        for port in 50002..50020 {
            monitor.path(SocketAddr::from(([127, 0, 0, 1], port)));
        }
        assert_eq!(monitor.paths.len(), 20);
        add_samples(&mut monitor, slow, &[200.0; 3], now);
        add_samples(&mut monitor, fast, &[12.0; 3], now);
        monitor.consider(now);
        for step in 1..=10 {
            let at = now + Duration::from_millis(step * 500);
            if step % 5 == 0 {
                add_samples(&mut monitor, fast, &[12.0], at);
                add_samples(&mut monitor, slow, &[200.0], at);
            }
            monitor.consider(at);
        }
        assert_eq!(
            monitor.pending.as_ref().unwrap().address,
            fast,
            "probe cadence must not erase all nomination progress"
        );
    }
    #[test]
    fn follower_rolls_back_only_to_a_fresh_verified_previous_path() {
        let slow = "127.0.0.1:50000".parse().unwrap();
        let fast = "127.0.0.1:50001".parse().unwrap();
        let began = Instant::now();
        let now = began + Duration::from_secs(4);
        let mut monitor = Monitor::new(config(slow, fast, false), began);
        monitor.activate(fast, began);
        add_samples(&mut monitor, slow, &[200.0; 5], now);
        for _ in 0..5 {
            monitor.path(fast).unwrap().sample(None, now);
        }
        monitor.rollback_if_lost(now);
        assert_eq!(monitor.config.active, slow);
        assert_eq!(monitor.rollbacks, 1);
        monitor.activate(fast, now);
        monitor.rollback_if_lost(now + Duration::from_secs(9));
        assert_eq!(
            monitor.config.active, fast,
            "stale previous path must not authorize failback"
        );
    }
    #[tokio::test]
    async fn stale_or_duplicate_pongs_do_not_improve_a_path_and_lost_switch_ack_keeps_old_route() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let active = "127.0.0.1:50000".parse().unwrap();
        let other = "127.0.0.1:50001".parse().unwrap();
        let now = Instant::now();
        let mut monitor = Monitor::new(config(active, other, true), now);
        let peer = config(active, other, false);
        let message = Message {
            kind: Kind::Pong,
            id: 11,
            token: 9,
            serial: 0,
        };
        monitor.outstanding.insert(11, (other, now));
        let pong = encode(&peer, message);
        monitor.receive(&socket, &pong, active, now + Duration::from_millis(10));
        assert!(monitor.path(other).unwrap().history.is_empty());
        monitor.receive(&socket, &pong, other, now + Duration::from_millis(10));
        monitor.receive(&socket, &pong, other, now + Duration::from_millis(11));
        assert_eq!(monitor.path(other).unwrap().history.len(), 1);
        monitor.pending = Some(PendingSwitch {
            address: other,
            message: Message {
                kind: Kind::Switch,
                serial: 1,
                ..message
            },
            began: now,
            sent: Some(now),
        });
        monitor.tick(&socket, now + Duration::from_secs(3));
        assert!(monitor.pending.is_none());
        assert_eq!(monitor.config.active, active);
    }
    #[tokio::test]
    async fn bridge_preserves_source_port_and_delivers_media_input_audio_during_live_switch() {
        let a = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address_a = a.local_addr().unwrap();
        let address_b = b.local_addr().unwrap();
        let slow_a = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let slow_b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let slow_address_a = slow_a.local_addr().unwrap();
        let slow_address_b = slow_b.local_addr().unwrap();
        // A delayed synthetic network path. Two port pairs reach the SAME peer.
        // No capture, playback, device input or external server is involved.
        let mut proxies = Vec::new();
        for (read, write, target, expected) in [
            (
                Arc::clone(&slow_a),
                Arc::clone(&slow_b),
                address_b,
                address_a,
            ),
            (
                Arc::clone(&slow_b),
                Arc::clone(&slow_a),
                address_a,
                address_b,
            ),
        ] {
            proxies.push(tokio::spawn(async move {
                let mut buffer = [0; 65536];
                let mut jobs = tokio::task::JoinSet::new();
                loop {
                    let (n, from) = read.recv_from(&mut buffer).await.unwrap();
                    assert_eq!(from, expected);
                    let bytes = buffer[..n].to_vec();
                    let write = Arc::clone(&write);
                    jobs.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let _ = write.send_to(&bytes, target).await;
                    });
                    while jobs.try_join_next().is_some() {}
                }
            }));
        }
        let engine_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let engine_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy_a = local_a.local_addr().unwrap();
        let proxy_b = local_b.local_addr().unwrap();
        let (stop_a, stopped_a) = oneshot::channel();
        let (stop_b, stopped_b) = oneshot::channel();
        let snapshots = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = Arc::clone(&snapshots);
        let bridge_a = tokio::spawn(run(
            Arc::clone(&a),
            local_a,
            engine_a.local_addr().unwrap(),
            config(slow_address_a, address_b, true),
            stopped_a,
            move |s| observed.lock().unwrap().push(s),
        ));
        let bridge_b = tokio::spawn(run(
            Arc::clone(&b),
            local_b,
            engine_b.local_addr().unwrap(),
            config(slow_address_b, address_a, false),
            stopped_b,
            |_| {},
        ));
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if snapshots
                .lock()
                .unwrap()
                .last()
                .is_some_and(|s| s.active_endpoint == address_b && s.switches == 1)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "faster path was never activated: {:?}",
                snapshots.lock().unwrap().last()
            );
            // Native media is already flowing while the alternative is measured
            // and nominated. Neither native window/socket is restarted.
            engine_a.send_to(b"stream-active", proxy_a).await.unwrap();
            let mut live = [0; 128];
            let (n, _) =
                tokio::time::timeout(Duration::from_secs(1), engine_b.recv_from(&mut live))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&live[..n], b"stream-active");
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert_eq!(a.local_addr().unwrap(), address_a);
        assert_eq!(b.local_addr().unwrap(), address_b);
        for payload in [
            b"authenticated-native-video".as_slice(),
            b"key-up",
            b"opus-audio",
        ] {
            engine_a.send_to(payload, proxy_a).await.unwrap();
            let mut packet = [0; 128];
            let (n, _) =
                tokio::time::timeout(Duration::from_millis(80), engine_b.recv_from(&mut packet))
                    .await
                    .expect("media still followed the 200ms path")
                    .unwrap();
            assert_eq!(&packet[..n], payload);
            engine_b.send_to(payload, proxy_b).await.unwrap();
            let (n, _) =
                tokio::time::timeout(Duration::from_millis(80), engine_a.recv_from(&mut packet))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&packet[..n], payload);
        }
        stop_a.send(()).unwrap();
        stop_b.send(()).unwrap();
        bridge_a.await.unwrap().unwrap();
        bridge_b.await.unwrap().unwrap();
        for proxy in proxies {
            proxy.abort();
        }
    }
    #[tokio::test]
    async fn adaptive_fec_recovers_real_udp_losses_while_control_and_audio_remain_unchanged() {
        let a = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address_a = a.local_addr().unwrap();
        let address_b = b.local_addr().unwrap();
        let proxy_a = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let proxy_b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let remote_a = proxy_a.local_addr().unwrap();
        let remote_b = proxy_b.local_addr().unwrap();
        let lost = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
        let mut proxies = Vec::new();
        for (read, write, target, drop_video) in [
            (Arc::clone(&proxy_a), Arc::clone(&proxy_b), address_b, true),
            (Arc::clone(&proxy_b), Arc::clone(&proxy_a), address_a, false),
        ] {
            let lost = Arc::clone(&lost);
            proxies.push(tokio::spawn(async move {
                let mut buffer = [0; 65536];
                loop {
                    let (n, _) = read.recv_from(&mut buffer).await.unwrap();
                    if drop_video
                        && let Some(meta) = crate::fec::video(&buffer[..n])
                        && meta.sequence % 16 == 7
                    {
                        lost.lock().unwrap().insert(meta.sequence);
                        continue;
                    }
                    write.send_to(&buffer[..n], target).await.unwrap();
                }
            }));
        }
        let engine_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let engine_b = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let local_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let near_a = local_a.local_addr().unwrap();
        let near_b = local_b.local_addr().unwrap();
        let (stop_a, stopped_a) = oneshot::channel();
        let (stop_b, stopped_b) = oneshot::channel();
        let mut ca = config(remote_a, remote_a, true);
        ca.alternatives.clear();
        let mut cb = config(remote_b, remote_b, false);
        cb.alternatives.clear();
        let stats = Arc::new(std::sync::Mutex::new(crate::fec::Stats::default()));
        let observed = Arc::clone(&stats);
        let bridge_a = tokio::spawn(run(
            a,
            local_a,
            engine_a.local_addr().unwrap(),
            ca,
            stopped_a,
            |_| {},
        ));
        let bridge_b = tokio::spawn(run(
            b,
            local_b,
            engine_b.local_addr().unwrap(),
            cb,
            stopped_b,
            move |s| *observed.lock().unwrap() = s.fec,
        ));
        let received = Arc::new(std::sync::Mutex::new(BTreeSet::new()));
        let received_frames = Arc::clone(&received);
        let viewer = Arc::clone(&engine_b);
        let viewer_task = tokio::spawn(async move {
            let mut packet = [0; 2048];
            loop {
                let (n, _) = viewer.recv_from(&mut packet).await.unwrap();
                let meta = crate::fec::video(&packet[..n])
                    .expect("FEC/probe records must never reach native");
                assert_eq!(
                    &packet[..n],
                    crate::fec::tests::native(meta.sequence, 7, 1120)
                );
                received_frames.lock().unwrap().insert(meta.sequence);
            }
        });
        let deadline = Instant::now() + Duration::from_millis(4500);
        let mut seq = 1;
        while Instant::now() < deadline {
            for _ in 0..16 {
                engine_a
                    .send_to(&crate::fec::tests::native(seq, 7, 1120), near_a)
                    .await
                    .unwrap();
                seq += 1;
            }
            for payload in [b"key-up".as_slice(), b"opus-audio"] {
                engine_b.send_to(payload, near_b).await.unwrap();
                let mut buffer = [0; 64];
                let (n, _) = tokio::time::timeout(
                    Duration::from_millis(500),
                    engine_a.recv_from(&mut buffer),
                )
                .await
                .expect("FEC blocked priority input/audio")
                .unwrap();
                assert_eq!(&buffer[..n], payload);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let end = Instant::now() + Duration::from_secs(2);
        while stats.lock().unwrap().recovered == 0 && Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let final_stats = *stats.lock().unwrap();
        assert!(
            final_stats.recovered > 0,
            "FEC never recovered a dropped original: {:?}",
            final_stats
        );
        assert!(
            lost.lock()
                .unwrap()
                .iter()
                .any(|seq| received.lock().unwrap().contains(seq)),
            "recovery must include a datagram actually discarded by the network fixture"
        );
        assert!(!viewer_task.is_finished());
        viewer_task.abort();
        stop_a.send(()).unwrap();
        stop_b.send(()).unwrap();
        bridge_a.await.unwrap().unwrap();
        bridge_b.await.unwrap().unwrap();
        for proxy in proxies {
            proxy.abort();
        }
    }
    #[tokio::test]
    async fn legacy_peer_without_monitor_support_keeps_original_media_and_cannot_switch() {
        let external = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let alternate = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let engine = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy = local.local_addr().unwrap();
        let address = external.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let samples = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = Arc::clone(&samples);
        let bridge = tokio::spawn(run(
            external,
            local,
            engine.local_addr().unwrap(),
            config(
                peer.local_addr().unwrap(),
                alternate.local_addr().unwrap(),
                true,
            ),
            stopped,
            move |snapshot| observed.lock().unwrap().push(snapshot.router),
        ));
        peer.send_to(b"native-frame", address).await.unwrap();
        let mut buffer = [0; 128];
        let (n, _) = tokio::time::timeout(Duration::from_secs(1), engine.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buffer[..n], b"native-frame");
        alternate
            .send_to(b"unverified-frame", address)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), engine.recv_from(&mut buffer))
                .await
                .is_err()
        );
        engine.send_to(b"native-input", proxy).await.unwrap();
        loop {
            let (n, from) =
                tokio::time::timeout(Duration::from_secs(1), peer.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            if buffer[..n].starts_with(MAGIC) || crate::fec::is_packet(&buffer[..n]) {
                continue;
            }
            assert_eq!(&buffer[..n], b"native-input");
            assert_eq!(from, address);
            break;
        }
        stop.send(()).unwrap();
        bridge.await.unwrap().unwrap();
        let samples = samples.lock().unwrap();
        assert_eq!(samples.first().unwrap().stop_reason, 0);
        let final_sample = samples.last().unwrap();
        assert_eq!(final_sample.stop_reason, 1);
        assert_eq!(final_sample.peer_payloads, 1);
        assert_eq!(final_sample.to_native, 1);
        assert_eq!(final_sample.native_received, 1);
        assert_eq!(final_sample.to_peer, 1);
        assert_eq!(final_sample.to_native_drops + final_sample.to_peer_drops, 0);
    }

    #[tokio::test]
    async fn router_distinguishes_probes_rejected_sources_and_missing_peer_payload() {
        let external = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address = external.local_addr().unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let engine = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let local = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let proxy = local.local_addr().unwrap();
        let peer_address = peer.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let (reports, mut samples) = tokio::sync::mpsc::unbounded_channel();
        let bridge = tokio::spawn(run(
            external,
            local,
            engine.local_addr().unwrap(),
            config(peer_address, peer_address, true),
            stopped,
            move |snapshot| {
                let _ = reports.send(snapshot.router);
            },
        ));
        let remote = config(address, address, false);
        let ping = encode(
            &remote,
            Message {
                kind: Kind::Ping,
                id: 7,
                token: 0,
                serial: 0,
            },
        );
        peer.send_to(&ping, address).await.unwrap();
        stranger.send_to(b"unverified-peer", address).await.unwrap();
        stranger.send_to(b"unverified-engine", proxy).await.unwrap();
        engine
            .send_to(b"control-without-peer-media", proxy)
            .await
            .unwrap();
        let sample = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let sample = samples.recv().await.unwrap();
                if sample.external_received >= 2
                    && sample.native_received == 1
                    && sample.native_rejected == 1
                {
                    break sample;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(sample.external_probes, 1);
        assert_eq!(sample.external_rejected, 1);
        assert_eq!(sample.peer_payloads, 0);
        assert_eq!(sample.to_native, 0);
        assert_eq!(sample.to_peer, 1);
        assert_eq!(sample.to_peer_drops, 0);
        // Router accounting never grants the foreign local sender access.
        let mut buffer = [0; 128];
        assert!(engine.try_recv_from(&mut buffer).is_err());
        stop.send(()).unwrap();
        bridge.await.unwrap().unwrap();
        let mut final_sample = None;
        while let Ok(sample) = samples.try_recv() {
            final_sample = Some(sample);
        }
        assert_eq!(final_sample.unwrap().stop_reason, 1);
    }
}
