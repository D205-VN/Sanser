//! Stable, dependency-light domain primitives shared by every Sanser component.

mod identity;
mod media;
mod network;

pub use identity::{DeviceId, SessionId, StreamId};
pub use media::{AudioCodec, VideoCodec};
pub use network::{NetworkMode, QualityProfile, TransportKind};

/// Product name used by binaries, APIs and diagnostics.
pub const PRODUCT_NAME: &str = "Sanser";
/// One source of truth for the application release.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Version of both the API contract and native packet protocol.
pub const PROTOCOL_VERSION: u8 = 2;
/// Native streaming protocol marker.
pub const NATIVE_PROTOCOL_NAME: &str = "SNV2";
/// API namespace for this protocol generation.
pub const API_V2_PREFIX: &str = "/api/v2";

const _: () = assert!(PROTOCOL_VERSION == 2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_constants_are_consistent() {
        assert_eq!(VERSION, "2.0.1");
        assert_eq!(PROTOCOL_VERSION, 2);
        assert_eq!(NATIVE_PROTOCOL_NAME, "SNV2");
    }
}
