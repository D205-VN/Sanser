use sanser_core::{NetworkMode, TransportKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteKind {
    LanDiscovery,
    DirectPrivateIp,
    IceHost,
    StunServerReflexive,
    TurnUdp,
    TurnTcp,
    TurnTls,
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
        let mut attempts = Vec::with_capacity(7);
        match mode {
            NetworkMode::Auto => {
                if snv2_available {
                    attempts.push(RouteAttempt {
                        route: RouteKind::LanDiscovery,
                        transport: TransportKind::Snv2,
                    });
                    attempts.push(RouteAttempt {
                        route: RouteKind::DirectPrivateIp,
                        transport: TransportKind::Snv2,
                    });
                }
                add_direct_webrtc(&mut attempts);
                add_relay(&mut attempts);
            }
            NetworkMode::Direct => {
                if snv2_available {
                    attempts.push(RouteAttempt {
                        route: RouteKind::LanDiscovery,
                        transport: TransportKind::Snv2,
                    });
                    attempts.push(RouteAttempt {
                        route: RouteKind::DirectPrivateIp,
                        transport: TransportKind::Snv2,
                    });
                }
                add_direct_webrtc(&mut attempts);
            }
            NetworkMode::Relay => add_relay(&mut attempts),
        }
        Self(attempts)
    }

    #[must_use]
    pub fn attempts(&self) -> &[RouteAttempt] {
        &self.0
    }
}

fn add_direct_webrtc(attempts: &mut Vec<RouteAttempt>) {
    attempts.push(RouteAttempt {
        route: RouteKind::IceHost,
        transport: TransportKind::WebRtcDirect,
    });
    attempts.push(RouteAttempt {
        route: RouteKind::StunServerReflexive,
        transport: TransportKind::WebRtcDirect,
    });
}

fn add_relay(attempts: &mut Vec<RouteAttempt>) {
    attempts.push(RouteAttempt {
        route: RouteKind::TurnUdp,
        transport: TransportKind::WebRtcRelayUdp,
    });
    attempts.push(RouteAttempt {
        route: RouteKind::TurnTcp,
        transport: TransportKind::WebRtcRelayTcp,
    });
    attempts.push(RouteAttempt {
        route: RouteKind::TurnTls,
        transport: TransportKind::WebRtcRelayTls,
    });
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
                RouteKind::LanDiscovery,
                RouteKind::DirectPrivateIp,
                RouteKind::IceHost,
                RouteKind::StunServerReflexive,
                RouteKind::TurnUdp,
                RouteKind::TurnTcp,
                RouteKind::TurnTls,
            ]
        );
    }

    #[test]
    fn direct_never_uses_turn_and_relay_never_uses_snv2() {
        let direct = RoutePlan::build(NetworkMode::Direct, true);
        assert!(direct.attempts().iter().all(|attempt| !matches!(
            attempt.route,
            RouteKind::TurnUdp | RouteKind::TurnTcp | RouteKind::TurnTls
        )));
        let relay = RoutePlan::build(NetworkMode::Relay, true);
        assert!(
            relay
                .attempts()
                .iter()
                .all(|attempt| attempt.transport != TransportKind::Snv2)
        );
    }
}
