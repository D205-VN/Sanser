//! In-session chat and annotation hub.
//!
//! Transport: the native engine's authenticated SNV2 user-data lane (`SCL1`
//! inside SUD1), which already exists on every 1:1 control session and on
//! each per-viewer peer of a sharing host. The host shell is the hub: it
//! validates, rate-limits and stamps the sender of every viewer message,
//! enforces the host's annotation policy and relays to the other viewers.
//! History lives only in this process's memory for the life of the native
//! session; nothing is persisted, logged or sent to the server.
use crate::error::DesktopError;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Mutex, mpsc},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Native payload bound (`collabMaxPayload` in native/desktop/collab.h).
pub const PAYLOAD_BYTES: usize = 4096;
/// Plain-text chat limit, measured in UTF-8 bytes.
pub const CHAT_BYTES: usize = 1024;
pub const NAME_CHARS: usize = 64;
/// Points per stroke message; strokes are split into short segments.
pub const STROKE_POINTS: usize = 32;
/// Normalized coordinates are fixed-point: 0 = left/top, 10000 = right/bottom.
pub const COORD_MAX: u16 = 10_000;
pub const COLORS: u8 = 6;
/// Stroke points each sender may send per second (burst equals one second).
pub const POINTS_PER_SECOND: f64 = 120.0;
/// Chat and clear messages: a burst of five, then one per second.
const CHAT_BURST: f64 = 5.0;
const CHAT_PER_SECOND: f64 = 1.0;
const CHAT_HISTORY: usize = 100;
const STROKE_HISTORY: usize = 400;
const MAX_PEERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Wire {
    Chat {
        id: String,
        #[serde(default)]
        name: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Stroke {
        id: String,
        #[serde(default)]
        name: String,
        color: u8,
        points: Vec<u16>,
        #[serde(default)]
        end: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Clear {
        #[serde(default)]
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Policy {
        annotations: bool,
    },
}

/// A message composed in this computer's UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Outgoing {
    Chat {
        name: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Stroke {
        name: String,
        stroke_id: String,
        color: u8,
        points: Vec<u16>,
        end: bool,
    },
    Clear {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub seq: u64,
    pub kind: &'static str,
    /// "self", "host", "-" (the single remote peer) or a sharing viewer id.
    pub from: String,
    pub name: String,
    pub own: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<bool>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub video_width: u32,
    pub video_height: u32,
    pub visible: bool,
    /// "pt" (macOS logical points) or "px" (Windows physical pixels).
    pub unit: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// The local native engine supports chat/annotation.
    pub available: bool,
    pub host: bool,
    /// Connected peers that negotiated chat/annotation.
    pub peers: usize,
    /// Connected peers whose engine does not (yet) support it.
    pub legacy_peers: usize,
    pub annotations: bool,
    pub cursor: u64,
    pub events: Vec<Event>,
}

struct Bucket {
    tokens: f64,
    burst: f64,
    rate: f64,
    at: Instant,
}
impl Bucket {
    fn new(burst: f64, rate: f64, now: Instant) -> Self {
        Self {
            tokens: burst,
            burst,
            rate,
            at: now,
        }
    }
    fn take(&mut self, cost: f64, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.at = now;
        if self.tokens + f64::EPSILON < cost {
            return false;
        }
        self.tokens -= cost;
        true
    }
}
struct Limits {
    chat: Bucket,
    points: Bucket,
}
impl Limits {
    fn new(now: Instant) -> Self {
        Self {
            chat: Bucket::new(CHAT_BURST, CHAT_PER_SECOND, now),
            points: Bucket::new(POINTS_PER_SECOND, POINTS_PER_SECOND, now),
        }
    }
    fn allow(&mut self, wire: &Wire, now: Instant) -> bool {
        match wire {
            Wire::Chat { .. } | Wire::Clear { .. } => self.chat.take(1.0, now),
            Wire::Stroke { points, .. } => self.points.take(
                f64::from(u32::try_from(points.len() / 2).unwrap_or(u32::MAX)),
                now,
            ),
            Wire::Policy { .. } => true,
        }
    }
}
struct Peer {
    capable: bool,
    limits: Limits,
}

struct State {
    host: bool,
    native: bool,
    annotations: bool,
    peers: HashMap<String, Peer>,
    own: Limits,
    seq: u64,
    chats: VecDeque<Event>,
    strokes: VecDeque<Event>,
    view: Option<View>,
    dropped: u64,
}
impl State {
    fn new() -> Self {
        Self {
            host: false,
            native: false,
            annotations: true,
            peers: HashMap::new(),
            own: Limits::new(Instant::now()),
            seq: 0,
            chats: VecDeque::new(),
            strokes: VecDeque::new(),
            view: None,
            dropped: 0,
        }
    }
}

pub struct Hub {
    state: Mutex<State>,
    pipe: Mutex<Option<mpsc::SyncSender<Vec<u8>>>>,
}
impl Default for Hub {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::new()),
            pipe: Mutex::new(None),
        }
    }
}

fn unavailable() -> DesktopError {
    DesktopError::Process("Chat is unavailable".into())
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| u64::try_from(value.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}
fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}
fn valid_source(id: &str) -> bool {
    id == "-"
        || ((1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'))
}
/// Display names are plain text: no control characters, bounded, trimmed.
pub fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}
/// Chat is plain text only. HTML is never rendered; this rejects control
/// characters (except newline/tab) and enforces the size bound.
pub fn valid_chat(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= CHAT_BYTES
        && text
            .chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\t')
}
pub fn valid_points(points: &[u16]) -> bool {
    !points.is_empty()
        && points.len().is_multiple_of(2)
        && points.len() <= STROKE_POINTS * 2
        && points.iter().all(|value| *value <= COORD_MAX)
}
fn valid(wire: &Wire) -> bool {
    match wire {
        Wire::Chat { id, text, .. } => valid_id(id) && valid_chat(text),
        Wire::Stroke {
            id, color, points, ..
        } => valid_id(id) && *color < COLORS && valid_points(points),
        Wire::Clear { .. } | Wire::Policy { .. } => true,
    }
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}
fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() > PAYLOAD_BYTES * 2 || !text.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}
fn random_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
/// `collab-send` stdin command for the native engine.
pub fn send_command(target: &str, wire: &Wire) -> Option<Vec<u8>> {
    if target != "*" && !valid_source(target) {
        return None;
    }
    let payload = serde_json::to_vec(wire).ok()?;
    (payload.len() <= PAYLOAD_BYTES)
        .then(|| format!("collab-send\t{target}\t{}\n", hex(&payload)).into_bytes())
}
fn with_from(wire: &Wire, source: &str) -> Wire {
    let mut wire = wire.clone();
    match &mut wire {
        Wire::Chat { from, name, .. }
        | Wire::Stroke { from, name, .. }
        | Wire::Clear { from, name } => {
            *from = Some(source.to_owned());
            *name = clean_name(name);
        }
        Wire::Policy { .. } => {}
    }
    wire
}

impl Hub {
    pub fn connect(&self, pipe: mpsc::SyncSender<Vec<u8>>) {
        if let Ok(mut current) = self.pipe.lock() {
            *current = Some(pipe);
        }
    }
    pub fn configure(&self, host: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.host = host;
        }
    }
    fn send(&self, target: &str, wire: &Wire) -> bool {
        let Some(command) = send_command(target, wire) else {
            return false;
        };
        self.pipe
            .lock()
            .ok()
            .and_then(|pipe| pipe.as_ref().map(|pipe| pipe.try_send(command).is_ok()))
            .unwrap_or(false)
    }
    fn record(state: &mut State, wire: &Wire, from: &str, own: bool) {
        state.seq += 1;
        let seq = state.seq;
        let at_ms = now_ms();
        let base = |kind, name: &str| Event {
            seq,
            kind,
            from: from.to_owned(),
            name: clean_name(name),
            own,
            text: None,
            stroke_id: None,
            color: None,
            points: None,
            end: None,
            at_ms,
        };
        match wire {
            Wire::Chat { name, text, .. } => {
                if state.chats.len() >= CHAT_HISTORY {
                    state.chats.pop_front();
                }
                state.chats.push_back(Event {
                    text: Some(text.clone()),
                    ..base("chat", name)
                });
            }
            Wire::Stroke {
                id,
                name,
                color,
                points,
                end,
                ..
            } => {
                if state.strokes.len() >= STROKE_HISTORY {
                    state.strokes.pop_front();
                }
                state.strokes.push_back(Event {
                    stroke_id: Some(id.clone()),
                    color: Some(*color),
                    points: Some(points.clone()),
                    end: Some(*end),
                    ..base("stroke", name)
                });
            }
            Wire::Clear { name, .. } => {
                if state.strokes.len() >= STROKE_HISTORY {
                    state.strokes.pop_front();
                }
                state.strokes.push_back(base("clear", name));
            }
            Wire::Policy { .. } => {}
        }
    }
    /// Native `SNUSER_COLLAB_CAP` record.
    pub fn observe_capability(&self, source: &str, available: bool) {
        if !valid_source(source) {
            return;
        }
        let policy = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.native = true;
            if !state.peers.contains_key(source) && state.peers.len() >= MAX_PEERS {
                return;
            }
            let peer = state
                .peers
                .entry(source.to_owned())
                .or_insert_with(|| Peer {
                    capable: false,
                    limits: Limits::new(Instant::now()),
                });
            let became = available && !peer.capable;
            peer.capable = available;
            (state.host && became).then_some(state.annotations)
        };
        // A newly capable viewer learns the host's current annotation policy.
        if let Some(annotations) = policy {
            self.send(source, &Wire::Policy { annotations });
        }
    }
    /// Native `SNUSER_COLLAB` record: an authenticated payload from `source`.
    pub fn observe_message(&self, source: &str, encoded: &str) {
        let Some(wire) = valid_source(source)
            .then(|| unhex(encoded))
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Wire>(&bytes).ok())
            .filter(valid)
        else {
            self.count_drop();
            return;
        };
        let now = Instant::now();
        let relay = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.host {
                let annotations = state.annotations;
                let Some(peer) = state.peers.get_mut(source).filter(|peer| peer.capable) else {
                    state.dropped += 1;
                    return;
                };
                let blocked = matches!(wire, Wire::Policy { .. })
                    || (!annotations && matches!(wire, Wire::Stroke { .. } | Wire::Clear { .. }));
                if blocked || !peer.limits.allow(&wire, now) {
                    state.dropped += 1;
                    return;
                }
                let stamped = with_from(&wire, source);
                Self::record(&mut state, &stamped, source, false);
                let targets = state
                    .peers
                    .iter()
                    .filter(|(id, peer)| id.as_str() != source && peer.capable)
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                Some((stamped, targets))
            } else {
                match &wire {
                    Wire::Policy { annotations } => state.annotations = *annotations,
                    Wire::Stroke { .. } | Wire::Clear { .. } if !state.annotations => {
                        state.dropped += 1;
                    }
                    Wire::Chat { from, .. }
                    | Wire::Stroke { from, .. }
                    | Wire::Clear { from, .. } => {
                        let from = from
                            .as_deref()
                            .filter(|value| valid_source(value) || *value == "host")
                            .unwrap_or("host")
                            .to_owned();
                        Self::record(&mut state, &wire, &from, false);
                    }
                }
                None
            }
        };
        if let Some((stamped, targets)) = relay {
            for target in targets {
                if !self.send(&target, &stamped) {
                    self.count_drop();
                }
            }
        }
    }
    /// Native `SNUSER_COLLAB_VIEW` record from a viewer engine.
    pub fn observe_view(&self, fields: &[&str]) {
        let [
            x,
            y,
            width,
            height,
            video_width,
            video_height,
            visible,
            unit,
        ] = fields
        else {
            return;
        };
        let (Ok(x), Ok(y), Ok(width), Ok(height), Ok(video_width), Ok(video_height)) = (
            x.parse::<i32>(),
            y.parse::<i32>(),
            width.parse::<u32>(),
            height.parse::<u32>(),
            video_width.parse::<u32>(),
            video_height.parse::<u32>(),
        ) else {
            return;
        };
        let unit = match *unit {
            "pt" => "pt",
            "px" => "px",
            _ => return,
        };
        if width > 16_384 || height > 16_384 || !matches!(*visible, "0" | "1") {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.view = Some(View {
                x,
                y,
                width,
                height,
                video_width: video_width.min(16_384),
                video_height: video_height.min(16_384),
                visible: *visible == "1",
                unit,
            });
        }
    }
    fn count_drop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.dropped += 1;
        }
    }
    /// Send a message composed locally. Returns its event sequence.
    pub fn post(&self, message: Outgoing) -> Result<u64, DesktopError> {
        let wire = match message {
            Outgoing::Chat { name, text } => Wire::Chat {
                id: random_id()[..16].to_owned(),
                name: clean_name(&name),
                text,
                from: None,
            },
            Outgoing::Stroke {
                name,
                stroke_id,
                color,
                points,
                end,
            } => Wire::Stroke {
                id: stroke_id,
                name: clean_name(&name),
                color,
                points,
                end,
                from: None,
            },
            Outgoing::Clear { name } => Wire::Clear {
                name: clean_name(&name),
                from: None,
            },
        };
        if !valid(&wire) {
            return Err(DesktopError::InvalidRequest(
                "Messages must be plain text up to 1 KB".into(),
            ));
        }
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if !state.peers.values().any(|peer| peer.capable) {
            return Err(DesktopError::Unavailable(
                "Chat needs an updated Sanser on the other computer".into(),
            ));
        }
        if matches!(wire, Wire::Stroke { .. } | Wire::Clear { .. }) && !state.annotations {
            return Err(DesktopError::Unavailable(
                "The host turned annotations off".into(),
            ));
        }
        if !state.own.allow(&wire, Instant::now()) {
            return Err(DesktopError::Process(
                "You're sending too quickly; wait a moment".into(),
            ));
        }
        let (target, sent) = if state.host {
            ("*", with_from(&wire, "host"))
        } else {
            ("-", wire.clone())
        };
        drop(state);
        if !self.send(target, &sent) {
            return Err(DesktopError::Process(
                "Chat is busy; try again in a moment".into(),
            ));
        }
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        Self::record(&mut state, &wire, "self", true);
        Ok(state.seq)
    }
    /// Host only: turn annotations on or off for the whole group.
    pub fn set_annotations(&self, enabled: bool) -> Result<(), DesktopError> {
        {
            let mut state = self.state.lock().map_err(|_| unavailable())?;
            if !state.host {
                return Err(DesktopError::InvalidRequest(
                    "Only the host can change annotations".into(),
                ));
            }
            state.annotations = enabled;
        }
        // Best effort: viewers that miss it learn the policy on reconnect, and
        // the host drops their strokes regardless.
        self.send(
            "*",
            &Wire::Policy {
                annotations: enabled,
            },
        );
        Ok(())
    }
    pub fn snapshot(&self, after: u64) -> Snapshot {
        let Ok(state) = self.state.lock() else {
            return Snapshot {
                available: false,
                host: false,
                peers: 0,
                legacy_peers: 0,
                annotations: false,
                cursor: 0,
                events: Vec::new(),
            };
        };
        let mut events = state
            .chats
            .iter()
            .chain(state.strokes.iter())
            .filter(|event| event.seq > after)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.seq);
        Snapshot {
            available: state.native,
            host: state.host,
            peers: state.peers.values().filter(|peer| peer.capable).count(),
            legacy_peers: state.peers.values().filter(|peer| !peer.capable).count(),
            annotations: state.annotations,
            cursor: state.seq,
            events,
        }
    }
    pub fn view(&self) -> Option<View> {
        self.state.lock().ok().and_then(|state| state.view)
    }
    pub fn forget_peer(&self, source: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.peers.remove(source);
        }
    }
    /// The native session ended: drop all history and capability state.
    pub fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            let host = state.host;
            *state = State::new();
            state.host = host;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(command: &[u8]) -> (String, Wire) {
        let text = std::str::from_utf8(command).unwrap_or_default().trim_end();
        let fields = text.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], "collab-send");
        let bytes = unhex(fields[2]).unwrap_or_default();
        (
            fields[1].to_owned(),
            serde_json::from_slice(&bytes).unwrap_or(Wire::Policy { annotations: false }),
        )
    }
    fn hub(host: bool) -> (Hub, mpsc::Receiver<Vec<u8>>) {
        let (sender, receiver) = mpsc::sync_channel(64);
        let hub = Hub::default();
        hub.connect(sender);
        hub.configure(host);
        (hub, receiver)
    }
    fn encoded(wire: &Wire) -> String {
        hex(&serde_json::to_vec(wire).unwrap_or_default())
    }
    fn chat(text: &str) -> Wire {
        Wire::Chat {
            id: "abc".into(),
            name: "Viewer".into(),
            text: text.into(),
            from: None,
        }
    }
    fn stroke(points: Vec<u16>) -> Wire {
        Wire::Stroke {
            id: "s1".into(),
            name: "Viewer".into(),
            color: 1,
            points,
            end: false,
            from: None,
        }
    }

    #[test]
    fn host_relays_viewer_chat_to_other_viewers_with_a_trusted_sender() {
        let (hub, commands) = hub(true);
        for viewer in ["a", "b", "c"] {
            hub.observe_capability(viewer, true);
        }
        // Each newly capable viewer receives the current policy.
        let policies = commands.try_iter().map(|c| decode(&c)).collect::<Vec<_>>();
        assert_eq!(policies.len(), 3);
        assert!(
            policies
                .iter()
                .all(|(_, wire)| *wire == Wire::Policy { annotations: true })
        );
        // A viewer cannot impersonate another sender.
        let forged = Wire::Chat {
            id: "abc".into(),
            name: "Viewer".into(),
            text: "hi <b>there</b>".into(),
            from: Some("host".into()),
        };
        hub.observe_message("a", &encoded(&forged));
        let mut relayed = commands.try_iter().map(|c| decode(&c)).collect::<Vec<_>>();
        relayed.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(relayed.len(), 2);
        assert_eq!(relayed[0].0, "b");
        assert_eq!(relayed[1].0, "c");
        assert!(
            matches!(&relayed[0].1, Wire::Chat { from: Some(from), text, .. } if from == "a" && text == "hi <b>there</b>")
        );
        let snapshot = hub.snapshot(0);
        assert_eq!(snapshot.peers, 3);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].from, "a");
        assert_eq!(snapshot.events[0].text.as_deref(), Some("hi <b>there</b>"));
        assert!(hub.snapshot(snapshot.cursor).events.is_empty());
    }

    #[test]
    fn invalid_oversized_or_unknown_messages_are_dropped() {
        let (hub, commands) = hub(true);
        hub.observe_capability("a", true);
        hub.observe_capability("b", true);
        let _ = commands.try_iter().count();
        for bad in [
            encoded(&chat(&"x".repeat(CHAT_BYTES + 1))),
            encoded(&chat("   ")),
            encoded(&chat("bell\u{7}")),
            encoded(&stroke(vec![1, 2, 3])),
            encoded(&stroke(vec![COORD_MAX + 1, 0])),
            encoded(&stroke(vec![1; STROKE_POINTS * 2 + 2])),
            encoded(&Wire::Policy { annotations: false }),
            hex(b"{\"type\":\"script\",\"src\":\"x\"}"),
            "zz".into(),
        ] {
            hub.observe_message("a", &bad);
        }
        // An unknown or not-yet-capable sender is ignored too.
        hub.observe_message("z", &encoded(&chat("hello")));
        assert_eq!(commands.try_iter().count(), 0);
        assert!(hub.snapshot(0).events.is_empty());
        assert!(hub.snapshot(0).annotations, "viewer changed host policy");
    }

    #[test]
    fn per_sender_rate_limits_bound_chat_and_stroke_points() {
        let (hub, commands) = hub(true);
        hub.observe_capability("a", true);
        hub.observe_capability("b", true);
        let _ = commands.try_iter().count();
        for _ in 0..20 {
            hub.observe_message("a", &encoded(&chat("spam")));
        }
        assert_eq!(commands.try_iter().count(), 5);
        // 32 points per message: four messages fit the 120-point second.
        for _ in 0..10 {
            hub.observe_message("a", &encoded(&stroke(vec![5000; STROKE_POINTS * 2])));
        }
        assert_eq!(commands.try_iter().count(), 3);
        // Another viewer has its own budget.
        hub.observe_message("b", &encoded(&chat("hello")));
        assert_eq!(commands.try_iter().count(), 1);
    }

    #[test]
    fn host_annotation_policy_is_enforced_and_announced() -> Result<(), DesktopError> {
        let (hub, commands) = hub(true);
        hub.observe_capability("a", true);
        hub.observe_capability("b", true);
        let _ = commands.try_iter().count();
        hub.set_annotations(false)?;
        let announced = commands.try_iter().map(|c| decode(&c)).collect::<Vec<_>>();
        assert_eq!(
            announced,
            vec![("*".into(), Wire::Policy { annotations: false })]
        );
        hub.observe_message("a", &encoded(&stroke(vec![1, 1])));
        hub.observe_message(
            "a",
            &encoded(&Wire::Clear {
                name: String::new(),
                from: None,
            }),
        );
        assert_eq!(commands.try_iter().count(), 0);
        // Chat still flows while annotations are off.
        hub.observe_message("a", &encoded(&chat("still here")));
        assert_eq!(commands.try_iter().count(), 1);
        assert!(
            hub.post(Outgoing::Clear {
                name: "Host".into()
            })
            .is_err()
        );
        // A viewer reconnecting learns the current policy.
        hub.observe_capability("a", false);
        hub.observe_capability("a", true);
        assert_eq!(
            commands.try_iter().map(|c| decode(&c)).collect::<Vec<_>>(),
            vec![("a".into(), Wire::Policy { annotations: false })]
        );
        Ok(())
    }

    #[test]
    fn viewer_follows_host_policy_and_posts_to_its_single_peer() -> Result<(), DesktopError> {
        let (hub, commands) = hub(false);
        assert!(
            hub.post(Outgoing::Chat {
                name: "Me".into(),
                text: "hi".into()
            })
            .is_err(),
            "posted to an older host"
        );
        hub.observe_capability("-", true);
        let seq = hub.post(Outgoing::Chat {
            name: "Me".into(),
            text: "hi".into(),
        })?;
        let (target, wire) = decode(&commands.try_recv().map_err(|_| unavailable())?);
        assert_eq!(target, "-");
        assert!(
            matches!(wire, Wire::Chat { from: None, .. }),
            "viewer must not claim a sender"
        );
        let own = hub.snapshot(0);
        assert_eq!(own.events.len(), 1);
        assert!(own.events[0].own && own.events[0].seq == seq);
        hub.observe_message("-", &encoded(&Wire::Policy { annotations: false }));
        assert!(!hub.snapshot(0).annotations);
        assert!(
            hub.post(Outgoing::Stroke {
                name: "Me".into(),
                stroke_id: "s".into(),
                color: 0,
                points: vec![1, 1],
                end: true
            })
            .is_err()
        );
        hub.observe_message("-", &encoded(&with_from(&stroke(vec![1, 1]), "b")));
        assert_eq!(
            hub.snapshot(seq).events.len(),
            0,
            "stroke shown while annotations are off"
        );
        hub.observe_message("-", &encoded(&Wire::Policy { annotations: true }));
        hub.observe_message("-", &encoded(&with_from(&stroke(vec![1, 1]), "b")));
        let events = hub.snapshot(seq).events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, "b");
        assert_eq!(events[0].kind, "stroke");
        hub.close();
        assert!(
            hub.snapshot(0).events.is_empty(),
            "history survived the session"
        );
        Ok(())
    }

    #[test]
    fn single_peer_host_records_without_relaying() -> Result<(), DesktopError> {
        let (hub, commands) = hub(true);
        hub.observe_capability("-", true);
        let _ = commands.try_iter().count();
        hub.observe_message("-", &encoded(&chat("from controller")));
        assert_eq!(commands.try_iter().count(), 0);
        hub.post(Outgoing::Chat {
            name: "Host".into(),
            text: "reply".into(),
        })?;
        let (target, wire) = decode(&commands.try_recv().map_err(|_| unavailable())?);
        assert_eq!(target, "*");
        assert!(matches!(wire, Wire::Chat { from: Some(from), .. } if from == "host"));
        Ok(())
    }

    #[test]
    fn view_records_are_strict() {
        let hub = Hub::default();
        hub.observe_view(&["10", "-20", "1280", "720", "1920", "1080", "1", "pt"]);
        assert_eq!(
            hub.view(),
            Some(View {
                x: 10,
                y: -20,
                width: 1280,
                height: 720,
                video_width: 1920,
                video_height: 1080,
                visible: true,
                unit: "pt"
            })
        );
        hub.observe_view(&["1", "2", "3", "4", "5", "6", "1", "in"]);
        hub.observe_view(&["1", "2", "99999", "4", "5", "6", "1", "px"]);
        assert_eq!(hub.view().map(|view| view.x), Some(10));
    }

    #[test]
    fn names_are_plain_bounded_text() {
        assert_eq!(clean_name("  Ann\u{0}e\n "), "Anne");
        assert_eq!(clean_name(&"n".repeat(200)).chars().count(), NAME_CHARS);
        assert!(send_command("bad target", &chat("x")).is_none());
        assert!(send_command("*", &chat("x")).is_some());
    }
}
