use hmac::{Hmac, Mac};
use sanser_core::{DeviceId, PROTOCOL_VERSION, VideoCodec};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    net::{SocketAddr, UdpSocket},
};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

type HmacSha256 = Hmac<Sha256>;
const DISCOVERY_MAGIC: [u8; 4] = *b"SND2";
const DISCOVERY_PREFIX_LEN: usize = 8;
const DISCOVERY_TAG_LEN: usize = 16;
const MAX_DISCOVERY_PAYLOAD: usize = 1_200;
const MAX_INTERFACES: usize = 32;
const MAX_REPLAY_NONCES: usize = 4_096;
const MAX_PEERS: usize = 1_024;
const MAX_TTL_MS: u64 = 120_000;
const FUTURE_SKEW_MS: u64 = 5_000;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct DiscoveryKey([u8; 32]);

impl DiscoveryKey {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for DiscoveryKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DiscoveryKey([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryCapabilities {
    pub codecs: Vec<VideoCodec>,
    pub native_snv2: bool,
    pub webrtc: bool,
    pub audio: bool,
    pub gamepad: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryAnnouncement {
    pub device_id: DeviceId,
    pub timestamp_ms: u64,
    pub nonce: [u8; 16],
    pub service_port: u16,
    pub display_name: String,
    pub platform: String,
    pub capabilities: DiscoveryCapabilities,
}

impl DiscoveryAnnouncement {
    pub fn validate(&self, now_ms: u64, ttl_ms: u64) -> Result<(), DiscoveryError> {
        if ttl_ms == 0 || ttl_ms > MAX_TTL_MS {
            return Err(DiscoveryError::InvalidTtl(ttl_ms));
        }
        if self.nonce.iter().all(|byte| *byte == 0) {
            return Err(DiscoveryError::EmptyNonce);
        }
        if self.service_port == 0 {
            return Err(DiscoveryError::InvalidPort);
        }
        validate_metadata(&self.display_name, 64)?;
        validate_metadata(&self.platform, 32)?;
        if self.capabilities.codecs.is_empty() || self.capabilities.codecs.len() > 4 {
            return Err(DiscoveryError::InvalidCapabilities);
        }
        if self
            .capabilities
            .codecs
            .iter()
            .any(|codec| *codec == VideoCodec::Auto)
            || self
                .capabilities
                .codecs
                .windows(2)
                .any(|window| window[0] == window[1])
        {
            return Err(DiscoveryError::InvalidCapabilities);
        }
        if self.timestamp_ms > now_ms.saturating_add(FUTURE_SKEW_MS) {
            return Err(DiscoveryError::FromFuture);
        }
        if now_ms.saturating_sub(self.timestamp_ms) > ttl_ms {
            return Err(DiscoveryError::Expired);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedDiscovery {
    pub announcement: DiscoveryAnnouncement,
}

impl SignedDiscovery {
    pub fn encode(&self, key: &DiscoveryKey) -> Result<Vec<u8>, DiscoveryError> {
        let payload =
            serde_json::to_vec(&self.announcement).map_err(|_| DiscoveryError::Serialization)?;
        if payload.len() > MAX_DISCOVERY_PAYLOAD {
            return Err(DiscoveryError::PayloadTooLarge(payload.len()));
        }
        let payload_len = u16::try_from(payload.len())
            .map_err(|_| DiscoveryError::PayloadTooLarge(payload.len()))?;
        let mut datagram =
            Vec::with_capacity(DISCOVERY_PREFIX_LEN + payload.len() + DISCOVERY_TAG_LEN);
        datagram.extend_from_slice(&DISCOVERY_MAGIC);
        datagram.push(PROTOCOL_VERSION);
        datagram.push(0);
        datagram.extend_from_slice(&payload_len.to_be_bytes());
        datagram.extend_from_slice(&payload);
        let tag = discovery_tag(key, &datagram)?;
        datagram.extend_from_slice(&tag);
        Ok(datagram)
    }

    pub fn decode(
        datagram: &[u8],
        key: &DiscoveryKey,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Self, DiscoveryError> {
        if datagram.len() < DISCOVERY_PREFIX_LEN + DISCOVERY_TAG_LEN {
            return Err(DiscoveryError::Truncated);
        }
        if datagram[0..4] != DISCOVERY_MAGIC {
            return Err(DiscoveryError::InvalidMagic);
        }
        if datagram[4] != PROTOCOL_VERSION {
            return Err(DiscoveryError::UnsupportedVersion(datagram[4]));
        }
        if datagram[5] != 0 {
            return Err(DiscoveryError::ReservedBitsSet);
        }
        let payload_len = usize::from(u16::from_be_bytes([datagram[6], datagram[7]]));
        if payload_len > MAX_DISCOVERY_PAYLOAD {
            return Err(DiscoveryError::PayloadTooLarge(payload_len));
        }
        let signed_len = DISCOVERY_PREFIX_LEN
            .checked_add(payload_len)
            .ok_or(DiscoveryError::LengthMismatch)?;
        let expected_len = signed_len
            .checked_add(DISCOVERY_TAG_LEN)
            .ok_or(DiscoveryError::LengthMismatch)?;
        if datagram.len() != expected_len {
            return Err(DiscoveryError::LengthMismatch);
        }
        let mut verifier =
            HmacSha256::new_from_slice(&key.0).map_err(|_| DiscoveryError::AuthenticationSetup)?;
        verifier.update(&datagram[..signed_len]);
        verifier
            .verify_truncated_left(&datagram[signed_len..])
            .map_err(|_| DiscoveryError::InvalidSignature)?;
        let announcement: DiscoveryAnnouncement =
            serde_json::from_slice(&datagram[DISCOVERY_PREFIX_LEN..signed_len])
                .map_err(|_| DiscoveryError::Serialization)?;
        announcement.validate(now_ms, ttl_ms)?;
        Ok(Self { announcement })
    }
}

fn discovery_tag(
    key: &DiscoveryKey,
    authenticated: &[u8],
) -> Result<[u8; DISCOVERY_TAG_LEN], DiscoveryError> {
    let mut mac =
        HmacSha256::new_from_slice(&key.0).map_err(|_| DiscoveryError::AuthenticationSetup)?;
    mac.update(authenticated);
    let digest = mac.finalize().into_bytes();
    let mut tag = [0_u8; DISCOVERY_TAG_LEN];
    tag.copy_from_slice(&digest[..DISCOVERY_TAG_LEN]);
    Ok(tag)
}

fn validate_metadata(value: &str, max_bytes: usize) -> Result<(), DiscoveryError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(DiscoveryError::InvalidMetadata);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct DiscoveryReplayWindow {
    entries: HashMap<[u8; 16], u64>,
    insertion_order: VecDeque<[u8; 16]>,
    capacity: usize,
}

impl DiscoveryReplayWindow {
    pub fn new(capacity: usize) -> Result<Self, DiscoveryError> {
        if capacity == 0 || capacity > MAX_REPLAY_NONCES {
            return Err(DiscoveryError::ReplayCapacity(capacity));
        }
        Ok(Self {
            entries: HashMap::with_capacity(capacity.min(256)),
            insertion_order: VecDeque::with_capacity(capacity.min(256)),
            capacity,
        })
    }

    pub fn observe(
        &mut self,
        nonce: [u8; 16],
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<(), DiscoveryError> {
        self.expire(now_ms);
        if self.entries.contains_key(&nonce) {
            return Err(DiscoveryError::Replay);
        }
        while self.entries.len() >= self.capacity {
            let Some(stale) = self.insertion_order.pop_front() else {
                break;
            };
            self.entries.remove(&stale);
        }
        self.entries.insert(nonce, now_ms.saturating_add(ttl_ms));
        self.insertion_order.push_back(nonce);
        Ok(())
    }

    pub fn expire(&mut self, now_ms: u64) {
        self.entries.retain(|_, expires_at| *expires_at >= now_ms);
        self.insertion_order
            .retain(|nonce| self.entries.contains_key(nonce));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerRecord {
    pub announcement: DiscoveryAnnouncement,
    pub endpoint: SocketAddr,
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerObservation {
    Added,
    Updated,
    Evicted(DeviceId),
}

#[derive(Clone, Debug)]
pub struct PeerTable {
    peers: BTreeMap<DeviceId, PeerRecord>,
    capacity: usize,
}

impl PeerTable {
    pub fn new(capacity: usize) -> Result<Self, DiscoveryError> {
        if capacity == 0 || capacity > MAX_PEERS {
            return Err(DiscoveryError::PeerCapacity(capacity));
        }
        Ok(Self {
            peers: BTreeMap::new(),
            capacity,
        })
    }

    /// Records a verified peer. Discovery deliberately provides no connect API;
    /// session authorization remains a separate explicit operation.
    pub fn observe(
        &mut self,
        announcement: DiscoveryAnnouncement,
        endpoint: SocketAddr,
        ttl_ms: u64,
    ) -> PeerObservation {
        let device_id = announcement.device_id;
        let record = PeerRecord {
            expires_at_ms: announcement.timestamp_ms.saturating_add(ttl_ms),
            announcement,
            endpoint,
        };
        if self.peers.insert(device_id, record).is_some() {
            return PeerObservation::Updated;
        }
        if self.peers.len() <= self.capacity {
            return PeerObservation::Added;
        }
        let evicted = self
            .peers
            .iter()
            .min_by_key(|(id, peer)| (peer.expires_at_ms, **id))
            .map(|(id, _)| *id);
        if let Some(evicted) = evicted {
            self.peers.remove(&evicted);
            PeerObservation::Evicted(evicted)
        } else {
            PeerObservation::Added
        }
    }

    pub fn expire(&mut self, now_ms: u64) -> Vec<DeviceId> {
        let expired: Vec<_> = self
            .peers
            .iter()
            .filter_map(|(id, peer)| (peer.expires_at_ms < now_ms).then_some(*id))
            .collect();
        for id in &expired {
            self.peers.remove(id);
        }
        expired
    }

    #[must_use]
    pub fn peers(&self) -> impl ExactSizeIterator<Item = &PeerRecord> {
        self.peers.values()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryRateLimiter {
    minimum_interval_ms: u64,
    last_sent_ms: Option<u64>,
}

impl DiscoveryRateLimiter {
    pub fn new(minimum_interval_ms: u64) -> Result<Self, DiscoveryError> {
        if !(1_000..=60_000).contains(&minimum_interval_ms) {
            return Err(DiscoveryError::RateInterval(minimum_interval_ms));
        }
        Ok(Self {
            minimum_interval_ms,
            last_sent_ms: None,
        })
    }

    pub fn allow(&mut self, now_ms: u64) -> bool {
        let allowed = self
            .last_sent_ms
            .is_none_or(|last| now_ms.saturating_sub(last) >= self.minimum_interval_ms);
        if allowed {
            self.last_sent_ms = Some(now_ms);
        }
        allowed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceBinding {
    pub bind_address: SocketAddr,
    pub broadcast_target: SocketAddr,
}

#[derive(Debug)]
pub struct UdpDiscovery {
    sockets: Vec<(UdpSocket, SocketAddr)>,
}

impl UdpDiscovery {
    pub fn bind(interfaces: &[InterfaceBinding]) -> Result<Self, DiscoveryError> {
        if interfaces.is_empty() || interfaces.len() > MAX_INTERFACES {
            return Err(DiscoveryError::InterfaceCount(interfaces.len()));
        }
        let mut sockets = Vec::with_capacity(interfaces.len());
        for interface in interfaces {
            let socket = UdpSocket::bind(interface.bind_address)?;
            socket.set_broadcast(true)?;
            socket.set_nonblocking(true)?;
            sockets.push((socket, interface.broadcast_target));
        }
        Ok(Self { sockets })
    }

    pub fn broadcast(&self, datagram: &[u8]) -> Result<usize, DiscoveryError> {
        if datagram.len() > DISCOVERY_PREFIX_LEN + MAX_DISCOVERY_PAYLOAD + DISCOVERY_TAG_LEN {
            return Err(DiscoveryError::PayloadTooLarge(datagram.len()));
        }
        let mut sent = 0;
        for (socket, target) in &self.sockets {
            sent += socket.send_to(datagram, target)?;
        }
        Ok(sent)
    }

    #[must_use]
    pub fn sockets(&self) -> impl ExactSizeIterator<Item = &UdpSocket> {
        self.sockets.iter().map(|(socket, _)| socket)
    }
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("discovery datagram is truncated")]
    Truncated,
    #[error("invalid discovery magic")]
    InvalidMagic,
    #[error("unsupported discovery protocol version {0}")]
    UnsupportedVersion(u8),
    #[error("discovery reserved bits are set")]
    ReservedBitsSet,
    #[error("discovery datagram length is invalid")]
    LengthMismatch,
    #[error("discovery payload size {0} exceeds the limit")]
    PayloadTooLarge(usize),
    #[error("discovery serialization failed")]
    Serialization,
    #[error("discovery authentication setup failed")]
    AuthenticationSetup,
    #[error("discovery signature is invalid")]
    InvalidSignature,
    #[error("discovery announcement has expired")]
    Expired,
    #[error("discovery announcement timestamp is in the future")]
    FromFuture,
    #[error("discovery nonce is empty")]
    EmptyNonce,
    #[error("discovery nonce is a replay")]
    Replay,
    #[error("discovery service port is invalid")]
    InvalidPort,
    #[error("discovery metadata is invalid")]
    InvalidMetadata,
    #[error("discovery capabilities are invalid")]
    InvalidCapabilities,
    #[error("discovery TTL {0}ms is outside 1..=120000ms")]
    InvalidTtl(u64),
    #[error("discovery replay capacity {0} is outside 1..=4096")]
    ReplayCapacity(usize),
    #[error("discovery peer capacity {0} is outside 1..=1024")]
    PeerCapacity(usize),
    #[error("discovery interval {0}ms is outside 1000..=60000ms")]
    RateInterval(u64),
    #[error("discovery interface count {0} is outside 1..=32")]
    InterfaceCount(usize),
    #[error("discovery UDP operation failed")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announcement(now_ms: u64) -> DiscoveryAnnouncement {
        DiscoveryAnnouncement {
            device_id: DeviceId::new(),
            timestamp_ms: now_ms,
            nonce: [1; 16],
            service_port: 5_174,
            display_name: "Living Room".to_owned(),
            platform: "windows".to_owned(),
            capabilities: DiscoveryCapabilities {
                codecs: vec![VideoCodec::H264, VideoCodec::Hevc],
                native_snv2: true,
                webrtc: true,
                audio: true,
                gamepad: true,
            },
        }
    }

    #[test]
    fn signature_replay_and_expiry_are_enforced() {
        let key = DiscoveryKey::new([7; 32]);
        let signed = SignedDiscovery {
            announcement: announcement(1_000),
        };
        let encoded = signed
            .encode(&key)
            .unwrap_or_else(|error| panic!("{error}"));
        let decoded = SignedDiscovery::decode(&encoded, &key, 1_010, 5_000)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut replay = DiscoveryReplayWindow::new(8).unwrap_or_else(|error| panic!("{error}"));
        assert!(
            replay
                .observe(decoded.announcement.nonce, 1_010, 5_000)
                .is_ok()
        );
        assert!(matches!(
            replay.observe(decoded.announcement.nonce, 1_011, 5_000),
            Err(DiscoveryError::Replay)
        ));

        let mut tampered = encoded;
        tampered[DISCOVERY_PREFIX_LEN] ^= 1;
        assert!(matches!(
            SignedDiscovery::decode(&tampered, &key, 1_010, 5_000),
            Err(DiscoveryError::InvalidSignature)
        ));

        let expired = SignedDiscovery {
            announcement: announcement(1),
        }
        .encode(&key)
        .unwrap_or_else(|error| panic!("{error}"));
        assert!(matches!(
            SignedDiscovery::decode(&expired, &key, 10_000, 5_000),
            Err(DiscoveryError::Expired)
        ));
    }

    #[test]
    fn address_ranges_are_not_assigned_special_vpn_meaning() {
        let endpoint: SocketAddr = "100.64.0.1:5174"
            .parse()
            .unwrap_or_else(|error| panic!("address parse failed: {error}"));
        let mut peers = PeerTable::new(2).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            peers.observe(announcement(1_000), endpoint, 5_000),
            PeerObservation::Added
        );
        assert_eq!(
            peers.peers().next().map(|peer| peer.endpoint),
            Some(endpoint)
        );
    }
}
