//! Port mapping abstractions (PCP, NAT-PMP, UPnP IGD).
//!
//! Phase 1 defines the common trait and types. Actual protocol
//! implementations are added in Phase 5.

pub mod nat_pmp;
pub mod pcp;
pub mod upnp;

use serde::Serialize;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// A successfully created port mapping.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    /// Protocol that created this mapping.
    pub protocol: MappingProtocolKind,
    /// External (public) address and port.
    pub external: SocketAddr,
    /// Internal (local) address and port.
    pub internal: SocketAddr,
    /// Time-to-live before the mapping expires.
    pub ttl: Duration,
    /// Whether this mapping needs periodic renewal.
    pub needs_renewal: bool,
}

/// Which port mapping protocol was used.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MappingProtocolKind {
    Pcp,
    NatPmp,
    Upnp,
}

/// Error from port mapping operations.
#[derive(Clone, Debug)]
pub enum MappingError {
    /// The router does not support this protocol.
    NotSupported,
    /// The router refused the mapping request.
    Refused { reason: String },
    /// The request timed out.
    Timeout,
    /// The mapping has expired and could not be renewed.
    Expired,
    /// A network error occurred.
    NetworkError { reason: String },
}

impl std::fmt::Display for MappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupported => write!(f, "port mapping protocol not supported by router"),
            Self::Refused { reason } => write!(f, "router refused mapping: {reason}"),
            Self::Timeout => write!(f, "port mapping request timed out"),
            Self::Expired => write!(f, "port mapping expired"),
            Self::NetworkError { reason } => write!(f, "port mapping network error: {reason}"),
        }
    }
}

impl std::error::Error for MappingError {}

/// Configuration for port mapping.
#[derive(Clone, Debug)]
pub struct MappingConfig {
    /// Desired external port (0 = let router choose).
    pub requested_port: u16,
    /// Requested lifetime in seconds.
    pub lifetime_seconds: u32,
    /// Renewal interval as a fraction of lifetime (e.g. 0.5 = renew at half).
    pub renewal_fraction: f64,
    /// Gateway address (auto-detected if None).
    pub gateway: Option<IpAddr>,
}

impl Default for MappingConfig {
    fn default() -> Self {
        Self {
            requested_port: 0,
            lifetime_seconds: 7200,
            renewal_fraction: 0.5,
            gateway: None,
        }
    }
}

/// Common trait for port mapping protocols.
#[async_trait::async_trait]
pub trait PortMapper {
    async fn try_map_port(
        &self,
        config: &MappingConfig,
        internal_port: u16,
    ) -> Result<PortMapping, MappingError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mapping_config_defaults() {
        let config = MappingConfig::default();
        assert_eq!(config.requested_port, 0);
        assert_eq!(config.lifetime_seconds, 7200);
        assert_eq!(config.gateway, None);
    }
}
