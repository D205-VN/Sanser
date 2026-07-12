use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use thiserror::Error;

const DEFAULT_CANDIDATE_CAPACITY: usize = 128;
const MAX_CANDIDATES: usize = 512;
const MAX_ID_BYTES: usize = 96;
const MAX_FOUNDATION_BYTES: usize = 64;

/// Origin of a direct UDP endpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CandidateType {
    Host,
    Ipv6Global,
    PortMapped,
    ServerReflexive,
    Manual,
}

/// Router mapping mechanism that created an endpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MappingProtocol {
    None,
    Pcp,
    NatPmp,
    Upnp,
    Manual,
}

/// P2P media transport. Phase one deliberately permits only SNV2 over UDP.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportProtocol {
    Udp,
}

/// One bounded, validated endpoint exchanged through signaling.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct P2pCandidate {
    pub id: String,
    #[serde(rename = "type")]
    pub candidate_type: CandidateType,
    pub address: IpAddr,
    pub port: u16,
    pub protocol: TransportProtocol,
    pub interface_index: Option<u32>,
    pub mapping_protocol: MappingProtocol,
    pub priority: u32,
    pub foundation: String,
}

impl P2pCandidate {
    /// Validates an endpoint received from a local gatherer or remote peer.
    ///
    /// # Errors
    ///
    /// Returns a [`CandidateError`] when metadata is not bounded, the endpoint
    /// cannot be routed safely, mapping/type fields disagree, or priority is
    /// outside the deterministic class assigned to the candidate type.
    pub fn validate(&self) -> Result<(), CandidateError> {
        validate_identifier(&self.id, MAX_ID_BYTES, CandidateError::InvalidId)?;
        validate_identifier(
            &self.foundation,
            MAX_FOUNDATION_BYTES,
            CandidateError::InvalidFoundation,
        )?;
        if self.port == 0 {
            return Err(CandidateError::InvalidPort);
        }
        if self.interface_index == Some(0) {
            return Err(CandidateError::InvalidInterfaceIndex);
        }
        validate_mapping(self.candidate_type, self.mapping_protocol)?;
        validate_address(self.candidate_type, self.address)?;

        let expected_class = type_preference(self.candidate_type, self.mapping_protocol)?;
        let actual_class = u8::try_from(self.priority >> 24)
            .map_err(|_| CandidateError::InvalidPriority(self.priority))?;
        let component = self.priority & 0xff;
        if self.priority == 0 || actual_class != expected_class || component != 255 {
            return Err(CandidateError::InvalidPriority(self.priority));
        }
        Ok(())
    }

    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        SocketAddr::new(self.address, self.port)
    }
}

/// Computes a deterministic ICE-style priority without depending on WebRTC.
///
/// Higher type and interface preferences win. Component `1` is encoded as the
/// final value `255`, because P2P v2 uses one multiplexed UDP component.
///
/// # Errors
///
/// Returns [`CandidateError::InvalidMapping`] when the mapping protocol does
/// not match the candidate type.
pub fn candidate_priority(
    candidate_type: CandidateType,
    mapping_protocol: MappingProtocol,
    interface_preference: u16,
) -> Result<u32, CandidateError> {
    let type_preference = type_preference(candidate_type, mapping_protocol)?;
    Ok((u32::from(type_preference) << 24) | (u32::from(interface_preference) << 8) | 255)
}

fn type_preference(
    candidate_type: CandidateType,
    mapping_protocol: MappingProtocol,
) -> Result<u8, CandidateError> {
    validate_mapping(candidate_type, mapping_protocol)?;
    Ok(match (candidate_type, mapping_protocol) {
        (CandidateType::Host, MappingProtocol::None) => 126,
        (CandidateType::Ipv6Global, MappingProtocol::None) => 120,
        (CandidateType::PortMapped, MappingProtocol::Pcp) => 110,
        (CandidateType::PortMapped, MappingProtocol::NatPmp) => 105,
        (CandidateType::PortMapped, MappingProtocol::Upnp) => 100,
        (CandidateType::ServerReflexive, MappingProtocol::None) => 90,
        (CandidateType::Manual, MappingProtocol::Manual) => 80,
        _ => return Err(CandidateError::InvalidMapping),
    })
}

fn validate_mapping(
    candidate_type: CandidateType,
    mapping_protocol: MappingProtocol,
) -> Result<(), CandidateError> {
    let valid = matches!(
        (candidate_type, mapping_protocol),
        (
            CandidateType::Host | CandidateType::Ipv6Global | CandidateType::ServerReflexive,
            MappingProtocol::None
        ) | (
            CandidateType::PortMapped,
            MappingProtocol::Pcp | MappingProtocol::NatPmp | MappingProtocol::Upnp
        ) | (CandidateType::Manual, MappingProtocol::Manual)
    );
    valid.then_some(()).ok_or(CandidateError::InvalidMapping)
}

fn validate_identifier(
    value: &str,
    max_bytes: usize,
    error: CandidateError,
) -> Result<(), CandidateError> {
    let valid = !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    valid.then_some(()).ok_or(error)
}

fn validate_address(candidate_type: CandidateType, address: IpAddr) -> Result<(), CandidateError> {
    let valid_unicast = match address {
        IpAddr::V4(ipv4) => valid_ipv4(ipv4),
        IpAddr::V6(ipv6) => valid_ipv6(ipv6),
    };
    if !valid_unicast {
        return Err(CandidateError::InvalidAddress(address));
    }

    match candidate_type {
        CandidateType::Ipv6Global => match address {
            IpAddr::V6(ipv6) if public_ipv6(ipv6) => Ok(()),
            _ => Err(CandidateError::AddressTypeMismatch),
        },
        CandidateType::PortMapped | CandidateType::ServerReflexive => {
            let public = match address {
                IpAddr::V4(ipv4) => public_ipv4(ipv4),
                IpAddr::V6(ipv6) => public_ipv6(ipv6),
            };
            public
                .then_some(())
                .ok_or(CandidateError::AddressTypeMismatch)
        }
        CandidateType::Host | CandidateType::Manual => Ok(()),
    }
}

fn valid_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_multicast()
        && !address.is_link_local()
        && !address.is_documentation()
        && octets != [255, 255, 255, 255]
        && octets[0] != 0
        && octets[0] < 240
}

fn public_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    if !valid_ipv4(address) || address.is_private() {
        return false;
    }
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return false;
    }
    !(octets[0] == 198 && matches!(octets[1], 18 | 19))
}

fn valid_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_multicast()
        && !address.is_unicast_link_local()
        && !is_ipv4_embedded(address)
        && !is_documentation_ipv6(address)
        && segments[0] & 0xffc0 != 0xfec0
}

fn public_ipv6(address: Ipv6Addr) -> bool {
    valid_ipv6(address) && !address.is_unique_local()
}

fn is_ipv4_embedded(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    segments[..6] == [0, 0, 0, 0, 0, 0]
        || (segments[..5] == [0, 0, 0, 0, 0] && segments[5] == u16::MAX)
}

fn is_documentation_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateInsert {
    Added,
    Duplicate,
    Replaced { removed_id: String },
}

/// A bounded candidate collection with endpoint and identifier deduplication.
#[derive(Clone, Debug)]
pub struct CandidateSet {
    candidates: Vec<P2pCandidate>,
    id_to_index: HashMap<String, usize>,
    capacity: usize,
}

impl CandidateSet {
    /// Creates an empty candidate set.
    ///
    /// # Errors
    ///
    /// Returns [`CandidateError::InvalidCapacity`] when `capacity` is outside
    /// the hard bound `1..=512`.
    pub fn new(capacity: usize) -> Result<Self, CandidateError> {
        if capacity == 0 || capacity > MAX_CANDIDATES {
            return Err(CandidateError::InvalidCapacity(capacity));
        }
        Ok(Self {
            candidates: Vec::with_capacity(capacity.min(DEFAULT_CANDIDATE_CAPACITY)),
            id_to_index: HashMap::with_capacity(capacity.min(DEFAULT_CANDIDATE_CAPACITY)),
            capacity,
        })
    }

    /// Inserts a validated candidate, suppressing duplicate endpoints.
    ///
    /// When two candidates describe the same UDP endpoint, the deterministic
    /// higher-priority candidate is retained. Equal-priority ties use the
    /// lexicographically smaller identifier so arrival order cannot change the
    /// result.
    ///
    /// # Errors
    ///
    /// Returns a validation error, [`CandidateError::IdCollision`] for an ID
    /// reused with different data, or [`CandidateError::CapacityExceeded`] for
    /// a distinct candidate beyond the configured bound.
    pub fn insert(&mut self, candidate: P2pCandidate) -> Result<CandidateInsert, CandidateError> {
        candidate.validate()?;
        if let Some(index) = self.id_to_index.get(&candidate.id).copied() {
            return if self.candidates[index] == candidate {
                Ok(CandidateInsert::Duplicate)
            } else {
                Err(CandidateError::IdCollision(candidate.id))
            };
        }

        if let Some(index) = self
            .candidates
            .iter()
            .position(|existing| existing.endpoint() == candidate.endpoint())
        {
            let existing = &self.candidates[index];
            let incoming_wins = match candidate.priority.cmp(&existing.priority) {
                Ordering::Greater => true,
                Ordering::Equal => candidate.id < existing.id,
                Ordering::Less => false,
            };
            if !incoming_wins {
                return Ok(CandidateInsert::Duplicate);
            }
            let removed_id = existing.id.clone();
            self.id_to_index.remove(&removed_id);
            self.id_to_index.insert(candidate.id.clone(), index);
            self.candidates[index] = candidate;
            return Ok(CandidateInsert::Replaced { removed_id });
        }

        if self.candidates.len() == self.capacity {
            return Err(CandidateError::CapacityExceeded(self.capacity));
        }
        let index = self.candidates.len();
        self.id_to_index.insert(candidate.id.clone(), index);
        self.candidates.push(candidate);
        Ok(CandidateInsert::Added)
    }

    #[must_use]
    pub fn ordered_by_priority(&self) -> Vec<&P2pCandidate> {
        let mut ordered: Vec<_> = self.candidates.iter().collect();
        ordered.sort_unstable_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        ordered
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

impl Default for CandidateSet {
    fn default() -> Self {
        Self {
            candidates: Vec::with_capacity(DEFAULT_CANDIDATE_CAPACITY),
            id_to_index: HashMap::with_capacity(DEFAULT_CANDIDATE_CAPACITY),
            capacity: DEFAULT_CANDIDATE_CAPACITY,
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CandidateError {
    #[error("candidate id is empty, oversized, or contains unsupported characters")]
    InvalidId,
    #[error("candidate foundation is empty, oversized, or contains unsupported characters")]
    InvalidFoundation,
    #[error("candidate port must be non-zero")]
    InvalidPort,
    #[error("candidate interface index must be non-zero when present")]
    InvalidInterfaceIndex,
    #[error("candidate mapping protocol does not match its type")]
    InvalidMapping,
    #[error("candidate address {0} is not a usable unicast address")]
    InvalidAddress(IpAddr),
    #[error("candidate address family or scope does not match its type")]
    AddressTypeMismatch,
    #[error("candidate priority {0} is outside its deterministic type class")]
    InvalidPriority(u32),
    #[error("candidate capacity {0} is outside 1..={MAX_CANDIDATES}")]
    InvalidCapacity(usize),
    #[error("candidate capacity {0} has been reached")]
    CapacityExceeded(usize),
    #[error("candidate id {0} was reused for a different endpoint")]
    IdCollision(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        id: &str,
        candidate_type: CandidateType,
        address: IpAddr,
        port: u16,
        mapping_protocol: MappingProtocol,
        interface_preference: u16,
    ) -> P2pCandidate {
        let priority = candidate_priority(candidate_type, mapping_protocol, interface_preference)
            .unwrap_or_else(|error| panic!("priority setup failed: {error}"));
        P2pCandidate {
            id: id.to_owned(),
            candidate_type,
            address,
            port,
            protocol: TransportProtocol::Udp,
            interface_index: Some(1),
            mapping_protocol,
            priority,
            foundation: "foundation-1".to_owned(),
        }
    }

    #[test]
    fn rejects_endpoints_that_must_never_be_signaled() {
        let cases = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 1, 2)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            "fe80::1"
                .parse::<IpAddr>()
                .unwrap_or_else(|error| panic!("test address failed: {error}")),
            "::ffff:8.8.8.8"
                .parse::<IpAddr>()
                .unwrap_or_else(|error| panic!("test address failed: {error}")),
            "fec0::1"
                .parse::<IpAddr>()
                .unwrap_or_else(|error| panic!("test address failed: {error}")),
        ];
        for address in cases {
            let invalid = candidate(
                "candidate-1",
                CandidateType::Host,
                address,
                50_000,
                MappingProtocol::None,
                1,
            );
            assert!(invalid.validate().is_err(), "accepted {address}");
        }
    }

    #[test]
    fn enforces_candidate_type_and_mapping_coherence() {
        let private_srflx = candidate(
            "srflx-1",
            CandidateType::ServerReflexive,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
            50_000,
            MappingProtocol::None,
            1,
        );
        assert_eq!(
            private_srflx.validate(),
            Err(CandidateError::AddressTypeMismatch)
        );
        assert_eq!(
            candidate_priority(CandidateType::PortMapped, MappingProtocol::None, 1),
            Err(CandidateError::InvalidMapping)
        );
    }

    #[test]
    fn candidate_json_uses_the_bounded_p2p_schema() {
        let original = candidate(
            "host-1",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
            50_000,
            MappingProtocol::None,
            42,
        );
        let json = serde_json::to_string(&original)
            .unwrap_or_else(|error| panic!("candidate serialization failed: {error}"));
        assert!(json.contains("\"type\":\"host\""));
        assert!(json.contains("\"mappingProtocol\":\"none\""));
        let decoded: P2pCandidate = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("candidate deserialization failed: {error}"));
        assert_eq!(decoded, original);
        assert_eq!(decoded.validate(), Ok(()));
    }

    #[test]
    fn set_is_bounded_and_deduplicates_by_id_and_endpoint() {
        let mut set = CandidateSet::new(1)
            .unwrap_or_else(|error| panic!("candidate set setup failed: {error}"));
        let low = candidate(
            "host-z",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
            50_000,
            MappingProtocol::None,
            1,
        );
        let high = candidate(
            "host-a",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
            50_000,
            MappingProtocol::None,
            2,
        );
        assert_eq!(set.insert(low.clone()), Ok(CandidateInsert::Added));
        assert_eq!(set.insert(low), Ok(CandidateInsert::Duplicate));
        assert_eq!(
            set.insert(high),
            Ok(CandidateInsert::Replaced {
                removed_id: "host-z".to_owned()
            })
        );

        let distinct = candidate(
            "host-2",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 3)),
            50_001,
            MappingProtocol::None,
            3,
        );
        assert_eq!(
            set.insert(distinct),
            Err(CandidateError::CapacityExceeded(1))
        );
    }

    #[test]
    fn id_reuse_with_different_data_is_rejected() {
        let mut set = CandidateSet::default();
        let first = candidate(
            "host-1",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)),
            50_000,
            MappingProtocol::None,
            1,
        );
        let second = candidate(
            "host-1",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3)),
            50_001,
            MappingProtocol::None,
            1,
        );
        assert_eq!(set.insert(first), Ok(CandidateInsert::Added));
        assert_eq!(
            set.insert(second),
            Err(CandidateError::IdCollision("host-1".to_owned()))
        );
    }

    #[test]
    fn priority_order_matches_the_p2p_route_policy() {
        let host = candidate_priority(CandidateType::Host, MappingProtocol::None, 0)
            .unwrap_or_else(|error| panic!("host priority failed: {error}"));
        let ipv6 = candidate_priority(CandidateType::Ipv6Global, MappingProtocol::None, 0)
            .unwrap_or_else(|error| panic!("IPv6 priority failed: {error}"));
        let pcp = candidate_priority(CandidateType::PortMapped, MappingProtocol::Pcp, 0)
            .unwrap_or_else(|error| panic!("PCP priority failed: {error}"));
        let stun = candidate_priority(CandidateType::ServerReflexive, MappingProtocol::None, 0)
            .unwrap_or_else(|error| panic!("STUN priority failed: {error}"));
        let manual = candidate_priority(CandidateType::Manual, MappingProtocol::Manual, 0)
            .unwrap_or_else(|error| panic!("manual priority failed: {error}"));
        assert!(host > ipv6 && ipv6 > pcp && pcp > stun && stun > manual);
    }

    #[test]
    fn candidate_priority_requires_the_single_udp_component_marker() {
        let mut invalid = candidate(
            "host-1",
            CandidateType::Host,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)),
            50_000,
            MappingProtocol::None,
            1,
        );
        invalid.priority &= !0xff;
        invalid.priority |= 1;
        assert!(matches!(
            invalid.validate(),
            Err(CandidateError::InvalidPriority(_))
        ));
    }
}
