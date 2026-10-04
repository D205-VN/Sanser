//! Bound signaling without losing entire ways of reaching the peer.

use crate::P2pCandidate;
use std::collections::HashSet;

/// Select up to 16 validated candidates, retaining one endpoint of each
/// address-family/origin class before filling remaining slots by priority.
/// A machine with many host interfaces must retain both STUN and router-mapped
/// endpoints; neither guarantees that the other will work through a given NAT.
#[must_use]
pub fn select_signaling_candidates(mut candidates: Vec<P2pCandidate>) -> Vec<P2pCandidate> {
    candidates.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    let mut endpoints = HashSet::new();
    candidates.retain(|candidate| endpoints.insert(candidate.endpoint()));
    let mut classes = HashSet::new();
    let mut selected = Vec::new();
    let mut deferred = Vec::new();
    for candidate in candidates {
        if classes.insert((candidate.address.is_ipv4(), candidate.candidate_type)) {
            selected.push(candidate);
        } else {
            deferred.push(candidate);
        }
    }
    // There are five origin classes and two address families, at most ten
    // reserved entries. Preserve all remaining priority slots up to the cap.
    selected.extend(
        deferred
            .into_iter()
            .take(16_usize.saturating_sub(selected.len())),
    );
    selected.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CandidateType, MappingProtocol, TransportProtocol, candidate_priority};

    fn candidate(id: usize, kind: CandidateType, address: &str) -> P2pCandidate {
        let mapping = match kind {
            CandidateType::PortMapped => MappingProtocol::Upnp,
            CandidateType::Manual => MappingProtocol::Manual,
            _ => MappingProtocol::None,
        };
        P2pCandidate {
            id: format!("candidate-{id}"),
            candidate_type: kind,
            address: address.parse().unwrap(),
            port: 5000 + id as u16,
            protocol: TransportProtocol::Udp,
            interface_index: Some(1),
            mapping_protocol: mapping,
            priority: candidate_priority(kind, mapping, u16::MAX).unwrap(),
            foundation: format!("foundation-{id}"),
        }
    }

    #[test]
    fn many_interfaces_retain_lan_stun_router_and_manual_routes() {
        let mut candidates: Vec<_> = (0..24)
            .map(|id| candidate(id, CandidateType::Host, "192.168.1.10"))
            .collect();
        candidates.push(candidate(30, CandidateType::ServerReflexive, "8.8.8.8"));
        candidates.push(candidate(31, CandidateType::PortMapped, "8.8.8.8"));
        candidates.push(candidate(32, CandidateType::Manual, "8.8.8.8"));
        candidates.push(candidate(33, CandidateType::Ipv6Global, "2606:4700::1111"));
        for candidate in &candidates {
            candidate.validate().unwrap();
        }
        let selected = select_signaling_candidates(candidates.clone());
        assert_eq!(selected.len(), 16);
        for kind in [
            CandidateType::Host,
            CandidateType::ServerReflexive,
            CandidateType::PortMapped,
            CandidateType::Manual,
            CandidateType::Ipv6Global,
        ] {
            assert!(
                selected
                    .iter()
                    .any(|candidate| candidate.candidate_type == kind)
            );
        }
        candidates.reverse();
        assert_eq!(selected, select_signaling_candidates(candidates));
    }

    #[test]
    fn duplicate_endpoints_cannot_consume_the_budget() {
        let host = candidate(1, CandidateType::Host, "192.168.1.10");
        let mut duplicate = host.clone();
        duplicate.id = "duplicate".into();
        assert_eq!(
            select_signaling_candidates(vec![host.clone(), duplicate]),
            vec![host]
        );
        assert!(select_signaling_candidates(Vec::new()).is_empty());
    }
}
