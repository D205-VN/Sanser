//! Optional systematic XOR FEC over opaque authenticated native video UDP.
//! Original datagrams are never delayed or rewritten. One parity protects
//! 8–16 datagrams; two erasures in the same block still require native NACK.
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

const MAGIC: &[u8; 4] = b"SFV1";
const HEADER: usize = 48;
const TAG: usize = 16;
const MAX_NATIVE: usize = 1300;
const CACHE: usize = 256;
const BLOCKS: usize = 32;
const REPAIR_AGE: Duration = Duration::from_millis(60);
const GROUP_AGE: Duration = Duration::from_millis(8);
const LOSS_GRACE: Duration = Duration::from_millis(80);
const FEEDBACK_AGE: Duration = Duration::from_secs(3);
const WINDOW: Duration = Duration::from_secs(1);

pub fn is_packet(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Video {
    pub family: u8,
    pub generation: u64,
    pub sequence: u64,
    pub repair: bool,
}
fn be64(b: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(b.try_into().ok()?))
}
fn le64(b: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(b.try_into().ok()?))
}
fn be32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.try_into().ok()?))
}
fn le32(b: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(b.try_into().ok()?))
}
fn le16(b: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes(b.try_into().ok()?))
}
fn be16(b: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes(b.try_into().ok()?))
}
/// Classification only. The native engine still authenticates the original
/// packet, including every reconstructed byte, before decrypting/decoding it.
pub fn video(bytes: &[u8]) -> Option<Video> {
    if bytes.len() > MAX_NATIVE {
        return None;
    }
    let meta = if bytes.len() >= 112 && &bytes[..4] == b"SNV2" {
        if bytes[4] != 2
            || bytes[5] != 80
            || bytes[6] != 3
            || bytes[7] != 5
            || be32(&bytes[28..32])? != 1
            || bytes.len() != 80 + be32(&bytes[56..60])? as usize
            || be16(&bytes[8..10])? & !63 != 0
        {
            return None;
        }
        Video {
            family: 1,
            generation: u64::from(be32(&bytes[60..64])?),
            sequence: be64(&bytes[32..40])?,
            repair: bytes[9] & 4 != 0,
        }
    } else if bytes.len() >= 82 && bytes[0] == 0 && &bytes[1..5] == b"SNU2" {
        let b = &bytes[1..];
        let count = le16(&b[6..8])?;
        let index = le16(&b[8..10])?;
        if le16(&b[4..6])? != 80
            || count == 0
            || count > 4096
            || index >= count
            || le16(&b[10..12])? & !7 != 0
            || bytes.len() != 81 + le32(&b[28..32])? as usize
            || le32(&b[28..32])? > 1152
        {
            return None;
        }
        Video {
            family: 2,
            generation: le64(&b[40..48])?,
            sequence: le64(&b[32..40])?,
            repair: b[10] & 4 != 0,
        }
    } else {
        return None;
    };
    (meta.sequence > 0 && meta.generation > 0).then_some(meta)
}

#[derive(Clone, Copy, Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub negotiated: bool,
    pub overhead_percent: u8,
    pub video_bytes: u64,
    pub parity_bytes: u64,
    pub parity_packets: u64,
    pub recovered: u64,
    pub expired: u64,
    pub invalid: u64,
    pub budget_skipped: u64,
    pub receiver_late: u64,
    pub congestion_pauses: u64,
}
#[derive(Clone, Copy, Default)]
struct Counts {
    received: u32,
    lost: u32,
    recovered: u32,
    late: u32,
    bytes: u64,
    drops: u32,
}
#[derive(Default)]
struct Sampler {
    highest: u64,
    seen: BTreeSet<u64>,
    gaps: BTreeMap<u64, Instant>,
    counts: Counts,
}
impl Sampler {
    fn note(&mut self, seq: u64, bytes: usize, recovered: bool, now: Instant) {
        if self.seen.contains(&seq) || (self.highest > 4096 && seq <= self.highest - 4096) {
            return;
        }
        if self.highest != 0 && seq > self.highest && seq - self.highest <= 4096 {
            for missing in self.highest + 1..seq {
                self.gaps.entry(missing).or_insert(now);
            }
        } else if seq.saturating_sub(self.highest) > 4096 {
            self.gaps.clear();
        }
        if seq < self.highest && self.gaps.remove(&seq).is_none() {
            self.counts.late = self.counts.late.saturating_add(1);
        }
        self.highest = self.highest.max(seq);
        self.seen.insert(seq);
        while self.seen.len() > 4096 {
            self.seen.pop_first();
        }
        self.gaps
            .retain(|&sequence, _| sequence.saturating_add(4096) >= self.highest);
        if recovered {
            self.counts.recovered = self.counts.recovered.saturating_add(1);
        } else {
            self.counts.received = self.counts.received.saturating_add(1);
            self.counts.bytes = self.counts.bytes.saturating_add(bytes as u64);
        }
    }
    fn mature(&mut self, now: Instant) {
        let old: Vec<_> = self
            .gaps
            .iter()
            .filter(|(_, at)| now.saturating_duration_since(**at) >= LOSS_GRACE)
            .map(|(&seq, _)| seq)
            .collect();
        for seq in old {
            self.gaps.remove(&seq);
            self.counts.lost = self.counts.lost.saturating_add(1);
        }
    }
}
struct Group {
    meta: Video,
    started: Instant,
    packets: Vec<(u64, Vec<u8>)>,
    longest: usize,
}
struct Parity {
    family: u8,
    generation: u64,
    entries: Vec<(u64, usize)>,
    bytes: Vec<u8>,
    received: Instant,
}
struct Cached {
    bytes: Vec<u8>,
    received: Instant,
}

pub struct Fec {
    send_key: [u8; 32],
    receive_key: [u8; 32],
    epoch: u64,
    peer_epoch: Option<u64>,
    sequence: u64,
    replay: BTreeSet<u64>,
    highest: u64,
    hello_at: Instant,
    window_at: Instant,
    peer_seen: Option<Instant>,
    peer_confirmed: bool,
    hello_ack: bool,
    mtu: usize,
    wire_overhead: usize,
    group: Option<Group>,
    cache: BTreeMap<u64, Cached>,
    order: VecDeque<u64>,
    pending: BTreeMap<u64, Parity>,
    rx_generation: Option<(u8, u64)>,
    retired: VecDeque<(u8, u64)>,
    tx_generation: Option<(u8, u64)>,
    sampler: Sampler,
    feedback_at: Option<Instant>,
    loss_windows: u8,
    healthy_windows: u8,
    percent: u8,
    credit: u64,
    paused_until: Instant,
    stats: Stats,
}
impl Drop for Fec {
    fn drop(&mut self) {
        self.send_key.fill(0);
        self.receive_key.fill(0);
    }
}
impl Fec {
    /// Device hashes and session must have been authorized by signaling. The
    /// base key never appears in diagnostics or the wire header.
    pub fn new(
        session: [u8; 16],
        local: u64,
        peer: u64,
        key: [u8; 32],
        epoch: u64,
        mtu: usize,
        wire_overhead: usize,
        now: Instant,
    ) -> Self {
        let derive = |device: u64| {
            let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("fixed HMAC key");
            mac.update(b"sanser-fec-direction-v1\0");
            mac.update(&session);
            mac.update(&device.to_be_bytes());
            mac.finalize().into_bytes().into()
        };
        Self {
            send_key: derive(local),
            receive_key: derive(peer),
            epoch: epoch.max(1),
            peer_epoch: None,
            sequence: 0,
            replay: BTreeSet::new(),
            highest: 0,
            hello_at: now,
            window_at: now,
            peer_seen: None,
            peer_confirmed: false,
            hello_ack: false,
            mtu: mtu.clamp(512, 1400),
            wire_overhead,
            group: None,
            cache: BTreeMap::new(),
            order: VecDeque::new(),
            pending: BTreeMap::new(),
            rx_generation: None,
            retired: VecDeque::new(),
            tx_generation: None,
            sampler: Sampler::default(),
            feedback_at: None,
            loss_windows: 0,
            healthy_windows: 0,
            percent: 0,
            credit: 0,
            paused_until: now,
            stats: Stats::default(),
        }
    }
    pub fn stats(&self, now: Instant) -> Stats {
        Stats {
            negotiated: self.peer_confirmed
                && self
                    .peer_seen
                    .is_some_and(|at| now.saturating_duration_since(at) <= FEEDBACK_AGE),
            overhead_percent: if self.enabled(now) { self.percent } else { 0 },
            ..self.stats
        }
    }
    fn enabled(&self, now: Instant) -> bool {
        self.peer_confirmed
            && self
                .peer_seen
                .is_some_and(|at| now.saturating_duration_since(at) <= FEEDBACK_AGE)
            && self
                .feedback_at
                .is_some_and(|at| now.saturating_duration_since(at) <= FEEDBACK_AGE)
            && now >= self.paused_until
            && self.percent > 0
    }
    pub fn next_tick(&self, now: Instant) -> Duration {
        if self.hello_ack {
            return Duration::ZERO;
        }
        let mut due = (self.hello_at + WINDOW).min(self.window_at + WINDOW);
        if let Some(group) = &self.group {
            due = due.min(group.started + GROUP_AGE);
        }
        if let Some(at) = self.pending.values().map(|p| p.received + REPAIR_AGE).min() {
            due = due.min(at);
        }
        due.saturating_duration_since(now)
    }
    fn envelope(
        &mut self,
        kind: u8,
        family: u8,
        count: u8,
        generation: u64,
        base: u64,
        length: u16,
        body: &[u8],
    ) -> Option<Vec<u8>> {
        self.sequence = self.sequence.checked_add(1)?;
        let mut b = vec![0; HEADER];
        b[..4].copy_from_slice(MAGIC);
        b[4] = 1;
        b[5] = kind;
        b[6] = family;
        b[7] = count;
        b[8..16].copy_from_slice(&self.epoch.to_be_bytes());
        b[16..24].copy_from_slice(&self.sequence.to_be_bytes());
        b[24..32].copy_from_slice(&generation.to_be_bytes());
        b[32..40].copy_from_slice(&base.to_be_bytes());
        b[40..42].copy_from_slice(&length.to_be_bytes());
        b.extend_from_slice(body);
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.send_key).ok()?;
        mac.update(&b);
        b.extend_from_slice(&mac.finalize().into_bytes()[..TAG]);
        (b.len() <= self.mtu).then_some(b)
    }
    fn authenticate(&mut self, b: &[u8]) -> bool {
        if b.len() < HEADER + TAG
            || b.len() > self.mtu
            || &b[..4] != MAGIC
            || b[4] != 1
            || b[42..48] != [0; 6]
        {
            return false;
        }
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&self.receive_key) else {
            return false;
        };
        mac.update(&b[..b.len() - TAG]);
        if mac.verify_truncated_left(&b[b.len() - TAG..]).is_err() {
            return false;
        }
        let Some(epoch) = be64(&b[8..16]) else {
            return false;
        };
        let Some(seq) = be64(&b[16..24]) else {
            return false;
        };
        if epoch == 0
            || seq == 0
            || self.peer_epoch.is_some_and(|p| p != epoch)
            || (self.highest >= 512 && seq <= self.highest - 512)
            || self.replay.contains(&seq)
        {
            return false;
        }
        if self.peer_epoch.is_none() {
            if b[5] != 1 {
                return false;
            }
            self.peer_epoch = Some(epoch);
        }
        self.highest = self.highest.max(seq);
        self.replay.insert(seq);
        self.replay
            .retain(|&s| s.saturating_add(512) > self.highest);
        true
    }
    /// These records are consumed by the UDP bridge, never sent to native.
    /// All other datagrams continue unchanged to native immediately.
    pub fn receive(&mut self, b: &[u8], now: Instant) -> (bool, Vec<Vec<u8>>) {
        // Input/audio/control never pay cache pruning or XOR recovery work.
        if !is_packet(b) && video(b).is_none() {
            return (false, Vec::new());
        }
        self.prune(now);
        if b.starts_with(MAGIC) {
            if !self.authenticate(b) {
                self.stats.invalid += 1;
                return (true, Vec::new());
            }
            self.peer_seen = Some(now);
            let family = b[6];
            let count = b[7] as usize;
            let generation = be64(&b[24..32]).unwrap_or(0);
            let base = be64(&b[32..40]).unwrap_or(0);
            let size = be16(&b[40..42]).unwrap_or(0) as usize;
            match b[5] {
                1 if b.len() == HEADER + TAG
                    && family == 0
                    && count == 0
                    && size == 0
                    && base == 0 =>
                {
                    let was_confirmed = self.peer_confirmed;
                    self.peer_confirmed |= generation == self.epoch;
                    if !was_confirmed {
                        self.hello_ack = true;
                    }
                }
                2 if b.len() == HEADER + 32 + TAG
                    && count == 0
                    && size == 0
                    && self.tx_generation == Some((family, generation)) =>
                {
                    self.feedback(&b[HEADER..HEADER + 32], now);
                }
                3 if (8..=16).contains(&count)
                    && (1..=MAX_NATIVE).contains(&size)
                    && b.len() == HEADER + count * 4 + size + TAG
                    && generation > 0
                    && matches!(family, 1 | 2) =>
                {
                    if self.rx_generation != Some((family, generation)) {
                        return (true, Vec::new());
                    }
                    let mut entries = Vec::with_capacity(count);
                    let mut ids = BTreeSet::new();
                    for chunk in b[HEADER..HEADER + count * 4].chunks_exact(4) {
                        let Some(seq) = base.checked_add(u64::from(be16(&chunk[..2]).unwrap_or(0)))
                        else {
                            self.stats.invalid += 1;
                            return (true, Vec::new());
                        };
                        let len = be16(&chunk[2..]).unwrap_or(0) as usize;
                        if seq == 0 || len == 0 || len > size || !ids.insert(seq) {
                            self.stats.invalid += 1;
                            return (true, Vec::new());
                        }
                        entries.push((seq, len));
                    }
                    let id = be64(&b[16..24]).unwrap_or(0);
                    if self.pending.len() >= BLOCKS {
                        self.pending.pop_first();
                        self.stats.expired += 1;
                    }
                    self.pending.insert(
                        id,
                        Parity {
                            family,
                            generation,
                            entries,
                            bytes: b[HEADER + count * 4..b.len() - TAG].to_vec(),
                            received: now,
                        },
                    );
                }
                _ => self.stats.invalid += 1,
            }
            return (true, self.recover(now));
        }
        if let Some(meta) = video(b) {
            self.incoming(meta, b, false, now);
        }
        (false, self.recover(now))
    }
    fn incoming(&mut self, meta: Video, b: &[u8], recovered: bool, now: Instant) {
        let generation = (meta.family, meta.generation);
        if self.rx_generation != Some(generation) {
            if self.retired.contains(&generation) {
                return;
            }
            if let Some(old) = self.rx_generation.replace(generation) {
                self.retired.push_back(old);
                while self.retired.len() > 8 {
                    self.retired.pop_front();
                }
            }
            self.cache.clear();
            self.order.clear();
            self.pending.clear();
            self.sampler = Sampler::default();
        }
        self.sampler.note(meta.sequence, b.len(), recovered, now);
        if self.peer_epoch.is_some() && !self.cache.contains_key(&meta.sequence) {
            self.cache.insert(
                meta.sequence,
                Cached {
                    bytes: b.to_vec(),
                    received: now,
                },
            );
            self.order.push_back(meta.sequence);
            while self.cache.len() > CACHE {
                if let Some(seq) = self.order.pop_front() {
                    self.cache.remove(&seq);
                }
            }
        }
    }
    fn prune(&mut self, now: Instant) {
        while self.order.front().is_some_and(|seq| {
            self.cache
                .get(seq)
                .is_none_or(|c| now.saturating_duration_since(c.received) >= REPAIR_AGE)
        }) {
            if let Some(seq) = self.order.pop_front() {
                self.cache.remove(&seq);
            }
        }
        self.pending.retain(|_, p| {
            let fresh = now.saturating_duration_since(p.received) < REPAIR_AGE;
            if !fresh {
                self.stats.expired += 1;
            }
            fresh
        });
        self.sampler.mature(now);
    }
    fn recover(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut done = Vec::new();
        let mut ready = Vec::new();
        for (&id, p) in &self.pending {
            let missing: Vec<_> = p
                .entries
                .iter()
                .filter(|(seq, _)| !self.cache.contains_key(seq))
                .collect();
            if missing.is_empty() {
                done.push(id);
                continue;
            }
            if missing.len() != 1 || ready.len() >= 4 {
                continue;
            }
            let (seq, len) = *missing[0];
            let mut restored = p.bytes.clone();
            let mut valid = true;
            for (sequence, length) in &p.entries {
                if *sequence == seq {
                    continue;
                }
                let Some(cached) = self.cache.get(sequence) else {
                    valid = false;
                    break;
                };
                if cached.bytes.len() != *length {
                    valid = false;
                    break;
                }
                for (target, byte) in restored.iter_mut().zip(&cached.bytes) {
                    *target ^= *byte;
                }
            }
            restored.truncate(len);
            if valid
                && video(&restored).is_some_and(|v| {
                    v.family == p.family
                        && v.generation == p.generation
                        && v.sequence == seq
                        && !v.repair
                })
            {
                ready.push(restored);
            } else {
                self.stats.invalid += 1;
            }
            done.push(id);
        }
        for id in done {
            self.pending.remove(&id);
        }
        for b in &ready {
            if let Some(meta) = video(b) {
                self.incoming(meta, b, true, now);
                self.stats.recovered += 1;
            }
        }
        ready
    }
    pub fn outgoing(&mut self, b: &[u8], now: Instant) -> Option<Vec<u8>> {
        let meta = video(b)?;
        if meta.repair {
            return None;
        }
        self.stats.video_bytes += b.len() as u64;
        let generation = (meta.family, meta.generation);
        if self.tx_generation != Some(generation) {
            self.tx_generation = Some(generation);
            self.group = None;
            self.feedback_at = None;
            self.percent = 0;
            self.credit = 0;
            self.loss_windows = 0;
            self.healthy_windows = 0;
        }
        if !self.enabled(now) {
            self.group = None;
            self.credit = 0;
            return None;
        }
        self.credit = (self.credit
            + ((b.len() + self.wire_overhead) as u64 * u64::from(self.percent)))
        .min(((self.mtu + self.wire_overhead) * 2 * 100) as u64);
        if self.group.as_ref().is_some_and(|g| {
            now.saturating_duration_since(g.started) >= GROUP_AGE
                || meta.sequence < g.packets[0].0
                || meta.sequence - g.packets[0].0 > u64::from(u16::MAX)
        }) {
            self.group = None;
        }
        let group = self.group.get_or_insert_with(|| Group {
            meta,
            started: now,
            packets: Vec::new(),
            longest: 0,
        });
        if group.packets.iter().any(|(seq, _)| *seq == meta.sequence) {
            return None;
        }
        group.longest = group.longest.max(b.len());
        group.packets.push((meta.sequence, b.to_vec()));
        let target = if self.percent >= 15 { 8 } else { 16 };
        let fit = self.mtu.saturating_sub(HEADER + TAG + group.longest) / 4;
        if group.packets.len() >= target.min(fit).clamp(8, 16) {
            self.parity(now)
        } else {
            None
        }
    }
    fn parity(&mut self, now: Instant) -> Option<Vec<u8>> {
        let group = self.group.take()?;
        if !self.enabled(now)
            || group.packets.len() < 8
            || now.saturating_duration_since(group.started) > Duration::from_millis(20)
        {
            return None;
        }
        let base = group.packets[0].0;
        let mut body = Vec::new();
        let mut parity = vec![0; group.longest];
        for (sequence, b) in &group.packets {
            body.extend_from_slice(
                &u16::try_from(sequence.checked_sub(base)?)
                    .ok()?
                    .to_be_bytes(),
            );
            body.extend_from_slice(&u16::try_from(b.len()).ok()?.to_be_bytes());
            for (target, byte) in parity.iter_mut().zip(b) {
                *target ^= *byte;
            }
        }
        body.extend(parity);
        let wire_bytes = HEADER + body.len() + TAG + self.wire_overhead;
        if HEADER + body.len() + TAG > self.mtu || wire_bytes as u64 * 100 > self.credit {
            self.stats.budget_skipped += 1;
            return None;
        }
        self.credit -= wire_bytes as u64 * 100;
        let packet = self.envelope(
            3,
            group.meta.family,
            group.packets.len() as u8,
            group.meta.generation,
            base,
            group.longest as u16,
            &body,
        )?;
        self.stats.parity_packets += 1;
        self.stats.parity_bytes += packet.len() as u64;
        Some(packet)
    }
    fn feedback(&mut self, b: &[u8], now: Instant) {
        let received = u64::from(be32(&b[..4]).unwrap_or(0));
        let lost = u64::from(be32(&b[4..8]).unwrap_or(0));
        let recovered = u64::from(be32(&b[8..12]).unwrap_or(0));
        let late = u64::from(be32(&b[12..16]).unwrap_or(0));
        let drops = be32(&b[28..32]).unwrap_or(0);
        let total = received + lost + recovered;
        if total < 64
            || total > 1_000_000
            || late > total
            || be32(&b[24..28]).is_none_or(|ms| !(500..=3000).contains(&ms))
        {
            return;
        }
        self.feedback_at = Some(now);
        let loss = (lost + recovered) as f64 / total as f64;
        if drops > 0 || loss >= 0.12 || late as f64 / total as f64 >= 0.03 {
            self.congested(now);
            return;
        }
        if loss >= 0.005 {
            self.loss_windows = self.loss_windows.saturating_add(1);
            self.healthy_windows = 0;
            if self.loss_windows >= 2 {
                self.percent = if loss >= 0.03 {
                    15
                } else if loss >= 0.01 {
                    12
                } else {
                    8
                };
            }
        } else {
            self.loss_windows = 0;
            self.healthy_windows = self.healthy_windows.saturating_add(1);
            if self.healthy_windows >= 3 {
                self.percent = 0;
                self.group = None;
                self.credit = 0;
            }
        }
    }
    /// Local overload or a fresh RTT inflation signal stops parity immediately.
    pub fn congested(&mut self, now: Instant) {
        if now >= self.paused_until {
            self.stats.congestion_pauses += 1;
        }
        self.paused_until = now + Duration::from_secs(5);
        self.percent = 0;
        self.credit = 0;
        self.group = None;
        self.loss_windows = 0;
    }
    pub fn path_changed(&mut self, now: Instant) {
        self.group = None;
        self.credit = 0;
        self.percent = 0;
        self.feedback_at = None;
        self.loss_windows = 0;
        self.healthy_windows = 0;
        self.paused_until = now + FEEDBACK_AGE;
        self.sampler.counts = Counts::default();
    }
    pub fn local_drop(&mut self, now: Instant) {
        self.sampler.counts.drops = self.sampler.counts.drops.saturating_add(1);
        self.congested(now);
    }
    pub fn tick(&mut self, now: Instant) -> Vec<Vec<u8>> {
        self.prune(now);
        let mut out = Vec::new();
        if now.saturating_duration_since(self.hello_at) >= WINDOW || self.hello_ack {
            self.hello_at = now;
            self.hello_ack = false;
            if let Some(b) = self.envelope(1, 0, 0, self.peer_epoch.unwrap_or(0), 0, 0, &[]) {
                out.push(b);
            }
        }
        if now.saturating_duration_since(self.window_at) >= WINDOW {
            let ms = now
                .saturating_duration_since(self.window_at)
                .as_millis()
                .min(u128::from(u32::MAX)) as u32;
            self.window_at = now;
            let c = std::mem::take(&mut self.sampler.counts);
            self.stats.receiver_late += u64::from(c.late);
            if let Some((family, generation)) = self.rx_generation
                && self.peer_confirmed
            {
                let mut body = Vec::new();
                for value in [c.received, c.lost, c.recovered, c.late] {
                    body.extend_from_slice(&value.to_be_bytes());
                }
                body.extend_from_slice(&c.bytes.to_be_bytes());
                body.extend_from_slice(&ms.to_be_bytes());
                body.extend_from_slice(&c.drops.to_be_bytes());
                if let Some(b) = self.envelope(2, family, 0, generation, 0, 0, &body) {
                    out.push(b);
                }
            }
        }
        if self
            .group
            .as_ref()
            .is_some_and(|g| now.saturating_duration_since(g.started) >= GROUP_AGE)
            && let Some(b) = self.parity(now)
        {
            out.push(b);
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn pair(now: Instant, mtu: usize, overhead: usize) -> (Fec, Fec) {
        (
            Fec::new([1; 16], 11, 22, [5; 32], 100, mtu, overhead, now),
            Fec::new([1; 16], 22, 11, [5; 32], 200, mtu, overhead, now),
        )
    }
    pub(crate) fn native(seq: u64, generation: u32, size: usize) -> Vec<u8> {
        let mut b = vec![0; size];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = ((i as u64 * 19 + seq * 31) % 251) as u8;
        }
        b[..4].copy_from_slice(b"SNV2");
        b[4] = 2;
        b[5] = 80;
        b[6] = 3;
        b[7] = 5;
        b[8..12].fill(0);
        b[28..32].copy_from_slice(&1_u32.to_be_bytes());
        b[32..40].copy_from_slice(&seq.to_be_bytes());
        b[56..60].copy_from_slice(&((size - 80) as u32).to_be_bytes());
        b[60..64].copy_from_slice(&generation.to_be_bytes());
        b
    }
    fn legacy(seq: u64) -> Vec<u8> {
        let mut b = vec![0; 1233];
        b[1..5].copy_from_slice(b"SNU2");
        b[5..7].copy_from_slice(&80_u16.to_le_bytes());
        b[7..9].copy_from_slice(&16_u16.to_le_bytes());
        b[29..33].copy_from_slice(&1152_u32.to_le_bytes());
        b[33..41].copy_from_slice(&seq.to_le_bytes());
        b[41..49].copy_from_slice(&7_u64.to_le_bytes());
        for (i, byte) in b[81..].iter_mut().enumerate() {
            *byte = ((i as u64 + seq) % 251) as u8;
        }
        b
    }
    fn handshake(a: &mut Fec, b: &mut Fec, at: Instant) {
        for _ in 0..5 {
            for packet in a.tick(at) {
                assert!(b.receive(&packet, at).0);
            }
            for packet in b.tick(at) {
                assert!(a.receive(&packet, at).0);
            }
        }
        assert!(a.peer_confirmed && b.peer_confirmed);
        assert!(
            a.tick(at).is_empty() && b.tick(at).is_empty(),
            "capability acknowledgement must not ping-pong indefinitely"
        );
    }
    fn feedback_packet(
        b: &mut Fec,
        meta: Video,
        received: u32,
        lost: u32,
        late: u32,
        drops: u32,
    ) -> Vec<u8> {
        let mut data = Vec::new();
        for value in [received, lost, 0, late] {
            data.extend_from_slice(&value.to_be_bytes());
        }
        data.extend_from_slice(&200000_u64.to_be_bytes());
        data.extend_from_slice(&1000_u32.to_be_bytes());
        data.extend_from_slice(&drops.to_be_bytes());
        b.envelope(2, meta.family, 0, meta.generation, 0, 0, &data)
            .unwrap()
    }
    fn warm(a: &mut Fec, b: &mut Fec, seed: &[u8], at: Instant) {
        handshake(a, b, at);
        a.outgoing(seed, at);
        b.receive(seed, at);
        for _ in 0..2 {
            let packet = feedback_packet(b, video(seed).unwrap(), 97, 3, 0, 0);
            a.receive(&packet, at);
        }
        assert_eq!(a.stats(at).overhead_percent, 15);
    }
    #[test]
    fn restores_every_single_erasure_for_snv2_and_authenticated_legacy_without_changing_bytes() {
        for old in [false, true] {
            for missing in 0..8 {
                let now = Instant::now();
                let at = now + Duration::from_secs(1);
                let (mut a, mut b) = pair(now, 1400, 28);
                let packets: Vec<_> = (2..10)
                    .map(|s| if old { legacy(s) } else { native(s, 7, 1120) })
                    .collect();
                warm(
                    &mut a,
                    &mut b,
                    &if old { legacy(1) } else { native(1, 7, 1120) },
                    at,
                );
                let mut parity = None;
                for (i, packet) in packets.iter().enumerate() {
                    parity = a.outgoing(packet, at).or(parity);
                    if i != missing {
                        assert!(b.receive(packet, at).1.is_empty());
                    }
                }
                let parity = parity.expect("8 full packets should fund one bounded parity");
                assert!(parity.len() <= 1400);
                let (used, recovered) = b.receive(&parity, at);
                assert!(used);
                assert_eq!(recovered, vec![packets[missing].clone()]);
                assert_eq!(b.stats(at).recovered, 1);
                assert!(
                    b.receive(&parity, at).1.is_empty(),
                    "parity replay duplicated video"
                );
            }
        }
    }
    #[test]
    fn handles_variable_lengths_reordering_and_two_losses_without_inventing_data() {
        let now = Instant::now();
        let at = now + WINDOW;
        let (mut a, mut b) = pair(now, 1400, 28);
        warm(&mut a, &mut b, &native(1, 7, 1120), at);
        // Accumulate credit from full-sized traffic, preserving the 15% cap.
        for seq in 2..10 {
            a.outgoing(&native(seq, 7, 1120), at);
        }
        let packets: Vec<_> = (10..18)
            .map(|seq| native(seq, 7, if seq == 12 { 128 } else { 1120 }))
            .collect();
        let mut parity = None;
        for packet in &packets {
            parity = a.outgoing(packet, at).or(parity);
        }
        let parity = parity.unwrap();
        let (_, recovered) = b.receive(&parity, at);
        assert!(recovered.is_empty());
        for &i in &[7, 5, 4, 3, 1, 0] {
            assert!(b.receive(&packets[i], at).1.is_empty());
        }
        assert_eq!(
            b.stats(at).recovered,
            0,
            "two erasures exceed XOR capability"
        );
        let (_, recovered) = b.receive(&packets[6], at + Duration::from_millis(5));
        assert_eq!(recovered, vec![packets[2].clone()]);
    }
    #[test]
    fn expired_missing_groups_generation_changes_and_old_epochs_never_replay_video() {
        let now = Instant::now();
        let at = now + WINDOW;
        let (mut a, mut b) = pair(now, 1400, 28);
        warm(&mut a, &mut b, &native(1, 7, 1120), at);
        let mut parity = None;
        for seq in 2..10 {
            let packet = native(seq, 7, 1120);
            parity = a.outgoing(&packet, at).or(parity);
            if seq > 3 {
                b.receive(&packet, at);
            }
        }
        let parity = parity.unwrap();
        assert!(b.receive(&parity, at).1.is_empty());
        b.tick(at + REPAIR_AGE);
        assert!(b.pending.is_empty());
        assert_eq!(b.stats(at).expired, 1);
        b.receive(&native(1, 8, 1120), at + REPAIR_AGE);
        assert_eq!(b.rx_generation, Some((1, 8)));
        b.receive(&native(2, 7, 1120), at + REPAIR_AGE);
        assert_eq!(b.rx_generation, Some((1, 8)));
        assert!(b.cache.keys().all(|seq| *seq == 1));
        let mut retired = Fec::new([1; 16], 11, 22, [5; 32], 999, 1400, 28, now);
        let packet = retired.envelope(1, 0, 0, 0, 0, 0, &[]).unwrap();
        assert!(b.receive(&packet, at).1.is_empty());
        assert_eq!(b.peer_epoch, Some(100));
    }
    #[test]
    fn old_peers_audio_input_repair_and_small_groups_stay_unchanged() {
        let now = Instant::now();
        let mut a = pair(now, 1400, 28).0;
        for seq in 1..100 {
            assert!(a.outgoing(&native(seq, 7, 1120), now).is_none());
        }
        assert!(!a.stats(now).negotiated);
        assert_eq!(a.stats(now).parity_packets, 0);
        assert_eq!(
            a.receive(b"keyboard/audio/control", now),
            (false, Vec::new())
        );
        let (mut a, mut b) = pair(now, 1400, 28);
        let at = now + WINDOW;
        warm(&mut a, &mut b, &native(1, 7, 1120), at);
        let mut repair = native(2, 7, 1120);
        repair[9] = 4;
        assert!(a.outgoing(&repair, at).is_none());
        assert!(a.group.is_none());
        for seq in 3..8 {
            assert!(a.outgoing(&native(seq, 7, 1120), at).is_none());
        }
        assert!(a.tick(at + GROUP_AGE).iter().all(|packet| packet[5] != 3));
        assert!(
            a.group.is_none(),
            "partial blocks must not wait for later frames"
        );
    }
    #[test]
    fn adaptive_policy_requires_sustained_feedback_and_stops_on_congestion_stale_feedback_and_path_change()
     {
        let now = Instant::now();
        let at = now + WINDOW;
        let (mut a, mut b) = pair(now, 1400, 28);
        handshake(&mut a, &mut b, at);
        let seed = native(1, 7, 1120);
        a.outgoing(&seed, at);
        let meta = video(&seed).unwrap();
        let p = feedback_packet(&mut b, meta, 98, 2, 0, 0);
        a.receive(&p, at);
        assert_eq!(a.stats(at).overhead_percent, 0);
        let p = feedback_packet(&mut b, meta, 98, 2, 0, 0);
        a.receive(&p, at);
        assert_eq!(a.stats(at).overhead_percent, 12);
        let p = feedback_packet(&mut b, meta, 97, 3, 0, 1);
        a.receive(&p, at);
        assert_eq!(a.stats(at).overhead_percent, 0);
        assert!(a.group.is_none());
        let later = at + Duration::from_secs(6);
        for _ in 0..2 {
            let p = feedback_packet(&mut b, meta, 97, 3, 0, 0);
            a.receive(&p, later);
        }
        assert_eq!(a.stats(later).overhead_percent, 15);
        assert_eq!(
            a.stats(later + Duration::from_secs(4)).overhead_percent,
            0,
            "stale feedback must not keep redundancy active"
        );
        a.path_changed(later);
        assert_eq!(a.stats(later).overhead_percent, 0);
        assert!(a.feedback_at.is_none());
        for _ in 0..2 {
            let p = feedback_packet(&mut b, meta, 97, 3, 0, 0);
            a.receive(&p, later + Duration::from_secs(4));
        }
        for _ in 0..3 {
            let p = feedback_packet(&mut b, meta, 100, 0, 0, 0);
            a.receive(&p, later + Duration::from_secs(4));
        }
        assert_eq!(a.stats(later + Duration::from_secs(4)).overhead_percent, 0);
    }
    #[test]
    fn parity_spending_including_headers_stays_below_cap_and_relay_mtu() {
        for old in [true, false] {
            let now = Instant::now();
            let at = now + WINDOW;
            let (mut a, mut b) = pair(now, 1348, 152);
            warm(
                &mut a,
                &mut b,
                &if old { legacy(1) } else { native(1, 7, 1120) },
                at,
            );
            let mut original_wire = 0;
            let mut parity_wire = 0;
            let mut parity_count = 0;
            for seq in 2..1002 {
                let packet = if old {
                    legacy(seq)
                } else {
                    native(seq, 7, 1120)
                };
                original_wire += packet.len() + 152;
                if let Some(parity) = a.outgoing(&packet, at) {
                    assert!(
                        parity.len() + 124 <= 1472,
                        "relay envelope caused an oversized UDP packet"
                    );
                    parity_count += 1;
                    parity_wire += parity.len() + 152;
                }
            }
            assert!(parity_count > 0);
            assert!(
                parity_wire * 100 <= original_wire * 15,
                "parity exceeded hard wire budget"
            );
        }
    }
    #[test]
    fn malformed_unauthenticated_wrong_direction_and_replayed_fec_are_consumed_without_output() {
        let now = Instant::now();
        let at = now + WINDOW;
        let (mut a, mut b) = pair(now, 1400, 28);
        let hello = a.tick(at).pop().unwrap();
        let mut changed = hello.clone();
        changed[32] ^= 1;
        assert_eq!(b.receive(&changed, at), (true, Vec::new()));
        assert!(b.peer_epoch.is_none());
        assert_eq!(a.receive(&hello, at), (true, Vec::new()));
        assert!(a.peer_epoch.is_none());
        b.receive(&hello, at);
        let before = b.highest;
        b.receive(&hello, at);
        assert_eq!(before, b.highest);
        for size in 0..1500 {
            let mut bytes = vec![255; size];
            if size >= 4 {
                bytes[..4].copy_from_slice(MAGIC);
            }
            let (_, packets) = b.receive(&bytes, at);
            assert!(packets.is_empty());
        }
        assert!(b.cache.len() <= CACHE && b.pending.len() <= BLOCKS && b.replay.len() <= 512);
    }
    #[test]
    fn authenticated_malformed_blocks_and_large_cache_loads_remain_bounded() {
        let now = Instant::now();
        let at = now + WINDOW;
        let (mut a, mut b) = pair(now, 1400, 28);
        warm(&mut a, &mut b, &native(1, 7, 1120), at);
        for seq in 2..2000 {
            b.receive(&native(seq, 7, 1120), at);
        }
        assert!(b.cache.len() <= CACHE && b.order.len() <= CACHE);
        let mut body = Vec::new();
        for delta in 0_u16..8 {
            body.extend_from_slice(&delta.to_be_bytes());
            body.extend_from_slice(&1120_u16.to_be_bytes());
        }
        body.resize(32 + 1120, 0);
        for base in (10000..11000).step_by(8) {
            let packet = a.envelope(3, 1, 8, 7, base, 1120, &body).unwrap();
            assert!(b.receive(&packet, at).1.is_empty());
        }
        assert_eq!(b.pending.len(), BLOCKS);
        assert!(b.stats(at).expired > 0);
        let mut duplicate = body.clone();
        duplicate[4..6].copy_from_slice(&0_u16.to_be_bytes());
        let malformed = a.envelope(3, 1, 8, 7, 20000, 1120, &duplicate).unwrap();
        let old = b.stats(at).invalid;
        assert!(b.receive(&malformed, at).1.is_empty());
        assert!(b.stats(at).invalid > old);
    }
    #[test]
    fn every_adaptive_tier_obeys_its_wire_budget_and_feedback_replay_cannot_activate_fec() {
        for (received, lost, percent) in [(199, 1, 8), (198, 2, 12), (97, 3, 15)] {
            let now = Instant::now();
            let at = now + WINDOW;
            let (mut a, mut b) = pair(now, 1348, 152);
            handshake(&mut a, &mut b, at);
            let seed = legacy(1);
            a.outgoing(&seed, at);
            b.receive(&seed, at);
            let message = feedback_packet(&mut b, video(&seed).unwrap(), received, lost, 0, 0);
            a.receive(&message, at);
            a.receive(&message, at);
            assert_eq!(a.stats(at).overhead_percent, 0);
            let message = feedback_packet(&mut b, video(&seed).unwrap(), received, lost, 0, 0);
            a.receive(&message, at);
            assert_eq!(a.stats(at).overhead_percent, percent);
            let mut data = 0;
            let mut parity = 0;
            for seq in 2..500 {
                let packet = legacy(seq);
                data += packet.len() + 152;
                if let Some(packet) = a.outgoing(&packet, at) {
                    parity += packet.len() + 152;
                }
            }
            assert!(parity > 0);
            assert!(parity * 100 <= data * usize::from(percent));
        }
    }
    #[test]
    fn receiver_loss_samples_allow_reorder_before_reporting_loss_and_bound_sequence_jumps() {
        let now = Instant::now();
        let mut s = Sampler::default();
        s.note(1, 100, false, now);
        s.note(3, 100, false, now);
        s.note(2, 100, false, now + Duration::from_millis(10));
        s.mature(now + LOSS_GRACE);
        assert_eq!(s.counts.lost, 0);
        s.note(5, 100, false, now + LOSS_GRACE);
        s.mature(now + LOSS_GRACE * 2);
        assert_eq!(s.counts.lost, 1);
        s.note(u64::MAX - 1, 100, false, now + LOSS_GRACE * 2);
        assert!(s.gaps.len() <= 4096 && s.seen.len() <= 4096);
    }
}
