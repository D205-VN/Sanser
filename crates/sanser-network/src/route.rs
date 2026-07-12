use sanser_core::{NetworkMode, TransportKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
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
pub struct RouteAttempt {
    pub route: RouteKind,
    pub transport: TransportKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutePlan(Vec<RouteAttempt>);

impl RoutePlan {
    #[must_use]
    pub fn build(mode: NetworkMode, snv2_available: bool) -> Self {
        let mut attempts = Vec::with_capacity(8);
        if !snv2_available {
            return Self(attempts);
        }

        match mode {
            NetworkMode::Auto => {
                attempts.push(RouteAttempt {
                    route: RouteKind::LanIpv4,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::LanIpv6,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::PublicIpv6,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::PcpMapped,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::NatPmpMapped,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::UpnpMapped,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::StunHolePunch,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::ManualForward,
                    transport: TransportKind::Snv2Udp,
                });
            }
            NetworkMode::DirectOnly => {
                attempts.push(RouteAttempt {
                    route: RouteKind::LanIpv4,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::LanIpv6,
                    transport: TransportKind::Snv2Udp,
                });
                attempts.push(RouteAttempt {
                    route: RouteKind::PublicIpv6,
                    transport: TransportKind::Snv2Udp,
                });
            }
            NetworkMode::Manual => {
                attempts.push(RouteAttempt {
                    route: RouteKind::ManualForward,
                    transport: TransportKind::Snv2Udp,
                });
            }
        }
        Self(attempts)
    }

    #[must_use]
    pub fn attempts(&self) -> &[RouteAttempt] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_uses_required_fallback_order() {
        let plan = RoutePlan::build(NetworkMode::Auto, true);
        let routes: Vec<_> = plan
            .attempts()
            .iter()
            .map(|attempt| attempt.route)
            .collect();
        assert_eq!(
            routes,
            vec![
                RouteKind::LanIpv4,
                RouteKind::LanIpv6,
                RouteKind::PublicIpv6,
                RouteKind::PcpMapped,
                RouteKind::NatPmpMapped,
                RouteKind::UpnpMapped,
                RouteKind::StunHolePunch,
                RouteKind::ManualForward,
            ]
        );
    }

    #[test]
    fn direct_only_never_uses_hole_punch_or_mappings() {
        let direct = RoutePlan::build(NetworkMode::DirectOnly, true);
        let routes: Vec<_> = direct
            .attempts()
            .iter()
            .map(|attempt| attempt.route)
            .collect();
        assert_eq!(
            routes,
            vec![
                RouteKind::LanIpv4,
                RouteKind::LanIpv6,
                RouteKind::PublicIpv6,
            ]
        );
    }

    #[test]
    fn manual_only_uses_manual_forward() {
        let manual = RoutePlan::build(NetworkMode::Manual, true);
        let routes: Vec<_> = manual
            .attempts()
            .iter()
            .map(|attempt| attempt.route)
            .collect();
        assert_eq!(routes, vec![RouteKind::ManualForward]);
    }
}
