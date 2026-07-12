//! Transport-independent network policy and bounded realtime buffers.

/// Opt-in P2P v2 primitives. The legacy route plan remains the default until
/// socket gathering and native-engine integration are complete.
#[cfg(feature = "p2p_v2")]
pub use sanser_p2p as p2p_v2;

mod discovery;
mod priority_queue;
mod retransmit;
mod route;

pub use discovery::{
    DiscoveryAnnouncement, DiscoveryCapabilities, DiscoveryError, DiscoveryKey,
    DiscoveryRateLimiter, DiscoveryReplayWindow, InterfaceBinding, PeerObservation, PeerRecord,
    PeerTable, SignedDiscovery, UdpDiscovery,
};
pub use priority_queue::{BoundedPriorityQueue, QueueCapacities, QueueFull};
pub use retransmit::{RetransmissionConfig, RetransmissionWindow, WindowConfigError};
pub use route::{RouteAttempt, RouteKind, RoutePlan};
