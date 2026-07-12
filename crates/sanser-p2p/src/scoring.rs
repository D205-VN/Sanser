use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_RTT_MS: u32 = 60_000;
const MAX_JITTER_MS: u32 = 60_000;
const MAX_INTERFACE_COST: u16 = 1_000;
const MAX_PACKET_LOSS_BASIS_POINTS: u16 = 10_000;

/// Route mechanism used by an authenticated candidate pair.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum P2pRouteKind {
    LanIpv4,
    LanIpv6,
    PublicIpv6,
    PcpMapped,
    NatPmpMapped,
    UpnpMapped,
    StunHolePunch,
    ManualForward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationStatus {
    Pending,
    Satisfied,
}

/// Evidence that a candidate pair is safe to nominate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PathEvidence {
    pub authentication: VerificationStatus,
    pub outbound_probe: VerificationStatus,
    pub inbound_probe: VerificationStatus,
    pub endpoint_match: VerificationStatus,
    pub session_authorization: VerificationStatus,
}

impl PathEvidence {
    #[must_use]
    pub const fn satisfied() -> Self {
        Self {
            authentication: VerificationStatus::Satisfied,
            outbound_probe: VerificationStatus::Satisfied,
            inbound_probe: VerificationStatus::Satisfied,
            endpoint_match: VerificationStatus::Satisfied,
            session_authorization: VerificationStatus::Satisfied,
        }
    }

    fn first_pending(self) -> Option<VerificationRequirement> {
        [
            (self.authentication, VerificationRequirement::Authentication),
            (self.outbound_probe, VerificationRequirement::OutboundProbe),
            (self.inbound_probe, VerificationRequirement::InboundProbe),
            (self.endpoint_match, VerificationRequirement::EndpointMatch),
            (
                self.session_authorization,
                VerificationRequirement::SessionAuthorization,
            ),
        ]
        .into_iter()
        .find_map(|(status, requirement)| {
            (status == VerificationStatus::Pending).then_some(requirement)
        })
    }
}

/// Bounded measurements used for deterministic route selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PathMetrics {
    pub rtt_ms: u32,
    pub jitter_ms: u32,
    /// Packet loss in basis points: `100` is one percent.
    pub packet_loss_basis_points: u16,
    /// Higher values represent metered, virtual or otherwise costly links.
    pub interface_cost: u16,
    pub mapping_ttl_seconds: Option<u32>,
}

impl PathMetrics {
    fn validate(self, route: P2pRouteKind) -> Result<(), ScoringError> {
        if self.rtt_ms > MAX_RTT_MS {
            return Err(ScoringError::InvalidRtt(self.rtt_ms));
        }
        if self.jitter_ms > MAX_JITTER_MS {
            return Err(ScoringError::InvalidJitter(self.jitter_ms));
        }
        if self.packet_loss_basis_points > MAX_PACKET_LOSS_BASIS_POINTS {
            return Err(ScoringError::InvalidPacketLoss(
                self.packet_loss_basis_points,
            ));
        }
        if self.interface_cost > MAX_INTERFACE_COST {
            return Err(ScoringError::InvalidInterfaceCost(self.interface_cost));
        }
        let mapped = matches!(
            route,
            P2pRouteKind::PcpMapped | P2pRouteKind::NatPmpMapped | P2pRouteKind::UpnpMapped
        );
        match (mapped, self.mapping_ttl_seconds) {
            (true, None) => return Err(ScoringError::MissingMappingLease),
            (true, Some(0)) => return Err(ScoringError::ExpiredMappingLease),
            (false, Some(_)) => return Err(ScoringError::UnexpectedMappingLease),
            (true, Some(_)) | (false, None) => {}
        }
        Ok(())
    }
}

/// A candidate pair that passed every nomination requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPath {
    pair_id: String,
    route: P2pRouteKind,
    metrics: PathMetrics,
}

impl VerifiedPath {
    /// Constructs a scoreable path only after all connectivity requirements pass.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe pair identifier, a pending verification
    /// requirement, out-of-range metrics, or a mapped route without lease data.
    pub fn new(
        pair_id: impl Into<String>,
        route: P2pRouteKind,
        metrics: PathMetrics,
        evidence: PathEvidence,
    ) -> Result<Self, ScoringError> {
        let pair_id = pair_id.into();
        if pair_id.is_empty()
            || pair_id.len() > 128
            || !pair_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(ScoringError::InvalidPairId);
        }
        if let Some(requirement) = evidence.first_pending() {
            return Err(ScoringError::Unverified(requirement));
        }
        metrics.validate(route)?;
        Ok(Self {
            pair_id,
            route,
            metrics,
        })
    }

    #[must_use]
    pub fn pair_id(&self) -> &str {
        &self.pair_id
    }

    #[must_use]
    pub const fn route(&self) -> P2pRouteKind {
        self.route
    }

    #[must_use]
    pub const fn metrics(&self) -> PathMetrics {
        self.metrics
    }

    #[must_use]
    pub fn score(&self) -> RouteScore {
        let base = match self.route {
            P2pRouteKind::LanIpv4 => 1_000_000_i64,
            P2pRouteKind::LanIpv6 => 950_000,
            P2pRouteKind::PublicIpv6 => 900_000,
            P2pRouteKind::PcpMapped => 850_000,
            P2pRouteKind::NatPmpMapped => 800_000,
            P2pRouteKind::UpnpMapped => 750_000,
            P2pRouteKind::StunHolePunch => 700_000,
            P2pRouteKind::ManualForward => 650_000,
        };
        let mapping_penalty = self
            .metrics
            .mapping_ttl_seconds
            .map_or(0_i64, |ttl| i64::from(120_u32.saturating_sub(ttl)) * 100);
        let penalty = i64::from(self.metrics.rtt_ms) * 100
            + i64::from(self.metrics.jitter_ms) * 75
            + i64::from(self.metrics.packet_loss_basis_points) * 20
            + i64::from(self.metrics.interface_cost) * 50
            + mapping_penalty;
        RouteScore(u32::try_from(base.saturating_sub(penalty).max(0)).unwrap_or(0))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RouteScore(u32);

impl RouteScore {
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Selects the highest scoring verified path with deterministic tie-breaking.
#[must_use]
pub fn select_best_path(paths: &[VerifiedPath]) -> Option<&VerifiedPath> {
    paths.iter().max_by(|left, right| {
        left.score()
            .cmp(&right.score())
            .then_with(|| right.pair_id.cmp(&left.pair_id))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationRequirement {
    Authentication,
    OutboundProbe,
    InboundProbe,
    EndpointMatch,
    SessionAuthorization,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ScoringError {
    #[error("candidate pair id is empty, oversized, or contains unsupported characters")]
    InvalidPairId,
    #[error("candidate pair has not satisfied {0:?}")]
    Unverified(VerificationRequirement),
    #[error("RTT {0}ms exceeds the supported bound")]
    InvalidRtt(u32),
    #[error("jitter {0}ms exceeds the supported bound")]
    InvalidJitter(u32),
    #[error("packet loss {0} basis points exceeds 100 percent")]
    InvalidPacketLoss(u16),
    #[error("interface cost {0} exceeds the supported bound")]
    InvalidInterfaceCost(u16),
    #[error("a mapped route must include its remaining lease duration")]
    MissingMappingLease,
    #[error("a mapped route lease must not already be expired")]
    ExpiredMappingLease,
    #[error("an unmapped route must not advertise mapping lease data")]
    UnexpectedMappingLease,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(rtt_ms: u32) -> PathMetrics {
        PathMetrics {
            rtt_ms,
            jitter_ms: 2,
            packet_loss_basis_points: 10,
            interface_cost: 0,
            mapping_ttl_seconds: None,
        }
    }

    #[test]
    fn refuses_to_score_an_unverified_path() {
        let evidence = PathEvidence {
            authentication: VerificationStatus::Satisfied,
            outbound_probe: VerificationStatus::Satisfied,
            inbound_probe: VerificationStatus::Pending,
            endpoint_match: VerificationStatus::Satisfied,
            session_authorization: VerificationStatus::Satisfied,
        };
        assert_eq!(
            VerifiedPath::new("pair-1", P2pRouteKind::LanIpv4, metrics(5), evidence),
            Err(ScoringError::Unverified(
                VerificationRequirement::InboundProbe
            ))
        );
    }

    #[test]
    fn stable_direct_route_beats_a_slightly_faster_hole_punch() {
        let lan = VerifiedPath::new(
            "lan",
            P2pRouteKind::LanIpv4,
            metrics(20),
            PathEvidence::satisfied(),
        )
        .unwrap_or_else(|error| panic!("LAN path failed: {error}"));
        let stun = VerifiedPath::new(
            "stun",
            P2pRouteKind::StunHolePunch,
            metrics(10),
            PathEvidence::satisfied(),
        )
        .unwrap_or_else(|error| panic!("STUN path failed: {error}"));
        assert!(lan.score() > stun.score());
        assert_eq!(
            select_best_path(&[stun, lan]).map(VerifiedPath::pair_id),
            Some("lan")
        );
    }

    #[test]
    fn quality_penalties_change_selection_within_the_same_route_type() {
        let slower = VerifiedPath::new(
            "slow",
            P2pRouteKind::PublicIpv6,
            metrics(40),
            PathEvidence::satisfied(),
        )
        .unwrap_or_else(|error| panic!("slow path failed: {error}"));
        let faster = VerifiedPath::new(
            "fast",
            P2pRouteKind::PublicIpv6,
            metrics(20),
            PathEvidence::satisfied(),
        )
        .unwrap_or_else(|error| panic!("fast path failed: {error}"));
        assert_eq!(
            select_best_path(&[slower, faster]).map(VerifiedPath::pair_id),
            Some("fast")
        );
    }

    #[test]
    fn mapped_routes_require_lease_data() {
        assert_eq!(
            VerifiedPath::new(
                "pcp",
                P2pRouteKind::PcpMapped,
                metrics(10),
                PathEvidence::satisfied(),
            ),
            Err(ScoringError::MissingMappingLease)
        );
    }

    #[test]
    fn mapping_lease_metadata_must_match_the_route() {
        let mut expired = metrics(10);
        expired.mapping_ttl_seconds = Some(0);
        assert_eq!(
            VerifiedPath::new(
                "pcp",
                P2pRouteKind::PcpMapped,
                expired,
                PathEvidence::satisfied(),
            ),
            Err(ScoringError::ExpiredMappingLease)
        );

        let mut unexpected = metrics(10);
        unexpected.mapping_ttl_seconds = Some(300);
        assert_eq!(
            VerifiedPath::new(
                "lan",
                P2pRouteKind::LanIpv4,
                unexpected,
                PathEvidence::satisfied(),
            ),
            Err(ScoringError::UnexpectedMappingLease)
        );
    }
}
