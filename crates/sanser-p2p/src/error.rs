//! Unified P2P error types with user-facing message mapping.

use thiserror::Error;

/// Top-level error returned by P2P operations.
#[derive(Clone, Debug, Error)]
pub enum P2pError {
    #[error("no local network interface is usable for P2P")]
    NoLocalInterface,

    #[error("STUN binding request failed: {reason}")]
    StunFailed { reason: String },

    #[error("STUN response timed out after {timeout_ms} ms")]
    StunTimeout { timeout_ms: u64 },

    #[error("port mapping failed: {reason}")]
    PortMappingFailed { reason: String },

    #[error("no candidate was gathered")]
    NoCandidateGathered,

    #[error("candidate exchange timed out")]
    CandidateExchangeTimeout,

    #[error("UDP hole punch failed after {attempts} attempts")]
    PunchFailed { attempts: u32 },

    #[error("connectivity check failed for pair {pair_id}")]
    ConnectivityCheckFailed { pair_id: String },

    #[error("authentication handshake failed: {reason}")]
    AuthenticationFailed { reason: String },

    #[error("session expired or revoked")]
    SessionExpired,

    #[error("peer is offline")]
    PeerOffline,

    #[error("firewall is blocking inbound UDP")]
    FirewallBlocked,

    #[error("network interface changed during session")]
    NetworkChanged,

    #[error("no direct route could be established")]
    NoDirectRoute,

    #[error("invalid probe packet: {reason}")]
    InvalidProbe { reason: String },

    #[error("socket bind failed: {reason}")]
    SocketBindFailed { reason: String },

    #[error("signaling error: {reason}")]
    SignalingError { reason: String },

    #[error("internal P2P error: {reason}")]
    Internal { reason: String },
}

impl P2pError {
    /// Returns a short, localisation-ready key suitable for UI display.
    #[must_use]
    pub fn ui_key(&self) -> &'static str {
        match self {
            Self::NoLocalInterface => "p2p_no_interface",
            Self::StunFailed { .. } | Self::StunTimeout { .. } => "p2p_stun_failed",
            Self::PortMappingFailed { .. } => "p2p_port_mapping_failed",
            Self::NoCandidateGathered => "p2p_no_candidate",
            Self::CandidateExchangeTimeout => "p2p_exchange_timeout",
            Self::PunchFailed { .. } => "p2p_punch_failed",
            Self::ConnectivityCheckFailed { .. } => "p2p_check_failed",
            Self::AuthenticationFailed { .. } => "p2p_auth_failed",
            Self::SessionExpired => "p2p_session_expired",
            Self::PeerOffline => "p2p_peer_offline",
            Self::FirewallBlocked => "p2p_firewall",
            Self::NetworkChanged => "p2p_network_changed",
            Self::NoDirectRoute => "p2p_no_route",
            Self::InvalidProbe { .. } => "p2p_invalid_probe",
            Self::SocketBindFailed { .. } => "p2p_socket_failed",
            Self::SignalingError { .. } => "p2p_signaling_error",
            Self::Internal { .. } => "p2p_internal_error",
        }
    }

    /// Returns a user-friendly Vietnamese description for the error.
    #[must_use]
    pub fn user_message_vi(&self) -> &'static str {
        match self {
            Self::NoLocalInterface => "Không tìm thấy giao diện mạng khả dụng.",
            Self::StunFailed { .. } | Self::StunTimeout { .. } => {
                "Không thể xác định địa chỉ IP công cộng."
            }
            Self::PortMappingFailed { .. } => "Không thể yêu cầu router mở cổng.",
            Self::NoCandidateGathered => "Không thu thập được địa chỉ kết nối nào.",
            Self::CandidateExchangeTimeout => "Hết thời gian trao đổi thông tin kết nối.",
            Self::PunchFailed { .. } => "Không thể xuyên NAT để kết nối trực tiếp.",
            Self::ConnectivityCheckFailed { .. } => "Kiểm tra kết nối thất bại.",
            Self::AuthenticationFailed { .. } => "Xác thực thiết bị thất bại.",
            Self::SessionExpired => "Phiên kết nối đã hết hạn.",
            Self::PeerOffline => "Thiết bị đích đang ngoại tuyến.",
            Self::FirewallBlocked => "Tường lửa đang chặn kết nối UDP.",
            Self::NetworkChanged => "Mạng đã thay đổi trong khi kết nối.",
            Self::NoDirectRoute => "Không thể tạo đường truyền trực tiếp.",
            Self::InvalidProbe { .. } => "Gói tin thăm dò không hợp lệ.",
            Self::SocketBindFailed { .. } => "Không thể mở cổng mạng.",
            Self::SignalingError { .. } => "Lỗi trao đổi tín hiệu.",
            Self::Internal { .. } => "Lỗi nội bộ hệ thống P2P.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_ui_key_and_message() {
        let errors: Vec<P2pError> = vec![
            P2pError::NoLocalInterface,
            P2pError::StunFailed {
                reason: "test".into(),
            },
            P2pError::StunTimeout { timeout_ms: 3000 },
            P2pError::PortMappingFailed {
                reason: "test".into(),
            },
            P2pError::NoCandidateGathered,
            P2pError::CandidateExchangeTimeout,
            P2pError::PunchFailed { attempts: 8 },
            P2pError::ConnectivityCheckFailed {
                pair_id: "test".into(),
            },
            P2pError::AuthenticationFailed {
                reason: "test".into(),
            },
            P2pError::SessionExpired,
            P2pError::PeerOffline,
            P2pError::FirewallBlocked,
            P2pError::NetworkChanged,
            P2pError::NoDirectRoute,
            P2pError::InvalidProbe {
                reason: "test".into(),
            },
            P2pError::SocketBindFailed {
                reason: "test".into(),
            },
            P2pError::SignalingError {
                reason: "test".into(),
            },
            P2pError::Internal {
                reason: "test".into(),
            },
        ];
        for error in &errors {
            assert!(!error.ui_key().is_empty(), "empty ui_key for {error}");
            assert!(
                !error.user_message_vi().is_empty(),
                "empty message for {error}"
            );
        }
    }
}
