//! P2P transport primitives for Sanser's SNV2 media protocol.
//!
//! This crate implements ICE-like candidate gathering, STUN, UDP hole
//! punching and port mapping — all feeding into a direct UDP path that
//! carries SNV2 encrypted media. No full WebRTC stack is used.
//!
//! ## Phase roadmap
//!
//! - **Phase 1** (current): Types, validation, scoring, state machine.
//! - **Phase 2**: Single UDP socket multiplexing.
//! - **Phase 3**: STUN binding, interface enumeration, candidate gathering.
//! - **Phase 4**: UDP hole punching, connectivity checks.
//! - **Phase 5**: PCP / NAT-PMP / UPnP port mapping.
//! - **Phase 6**: X25519 key exchange, AEAD encryption.
//! - **Phase 7**: Adaptive bitrate, congestion control.
//! - **Phase 8**: Reconnect, route migration.

#![allow(clippy::all)]

mod candidate;
mod connectivity;
mod error;
mod gatherer;
mod interface;
mod keepalive;
pub mod mapping;
mod metrics;
mod migration;
mod nomination;
mod punch;
mod scoring;
mod security;
mod signaling;
mod state;
pub mod stun;

// ─── Re-exports ─────────────────────────────────────────────────────────────

pub use candidate::{
    CandidateError, CandidateInsert, CandidateSet, CandidateType, MappingProtocol, P2pCandidate,
    TransportProtocol, candidate_priority,
};
pub use connectivity::{
    CandidatePair, PROBE_COMMIT_BIT, PROBE_FINAL_BIT, PROBE_MAGIC, PROBE_MIN_SIZE,
    PROBE_NOMINATION_BIT, PROBE_RESPONSE_BIT, PROBE_TOTAL_SIZE, PROBE_VERSION, PUNCH_SCHEDULE,
    PairState, ProbeFields, PunchScheduleEntry, build_probe_packet, compute_pair_hash,
    make_pair_id, pair_priority, parse_probe_packet,
};
pub use error::P2pError;
pub use gatherer::{
    GathererConfig, GatheringEvent, GatheringResult, gather_candidates, gather_candidates_on_socket,
};
pub use interface::{
    InterfaceCost, InterfaceFilter, InterfaceKind, InterfaceRejectReason, NetworkInterface,
    enumerate_interfaces, filter_interface,
};
pub use keepalive::{KeepaliveConfig, KeepaliveState};
pub use metrics::{
    AdaptiveBitrateController, ConnectivityMetrics, FirewallState, GatheringMetrics,
    P2pDiagnostics, PathMetricsSnapshot, SessionCounters,
};
pub use migration::{
    MigrationController, MigrationReason, MigrationState, ReconnectAction, ReconnectSchedule,
};
pub use nomination::{Nomination, NominationPolicy};
pub use punch::{PunchAttempt, PunchState, check_connectivity};
pub use scoring::{
    P2pRouteKind, PathEvidence, PathMetrics, RouteScore, ScoringError, VerificationStatus,
    VerifiedPath, select_best_path,
};
pub use security::{
    AeadAlgorithm, DerivedKeys, HandshakeState, NonceComponents, RekeyTrigger, decrypt_payload,
    derive_keys, encrypt_payload, generate_x25519_keypair, public_key_from_base64,
    public_key_to_base64,
};
pub use signaling::{
    P2pCandidateBatch, P2pClosed, P2pFailed, P2pGatheringComplete, P2pHello, P2pNominate,
    P2pPathChanged, P2pPunchReady, P2pRole, P2pSelected, P2pSessionCredential, P2pSignal,
    SignalEnvelope,
};
pub use state::{P2pEvent, P2pFailure, P2pStage, P2pStateMachine, StateMachineError};
