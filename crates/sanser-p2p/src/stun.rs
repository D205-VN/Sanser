//! STUN Binding Request/Response types (RFC 5389).
//!
//! Phase 1 defines the wire format types and parsing logic. Actual socket I/O
//! (sending requests to `stun.l.google.com:19302`) is added in Phase 3.

use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use crate::error::P2pError;
use std::time::Duration;
use tokio::net::UdpSocket;

// STUN constants (RFC 5389)
const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
const STUN_HEADER_SIZE: usize = 20;
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_BINDING_ERROR: u16 = 0x0111;

// Attribute types
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const ATTR_ERROR_CODE: u16 = 0x0009;

const FAMILY_IPV4: u8 = 0x01;
const FAMILY_IPV6: u8 = 0x02;

/// A STUN server endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StunServer {
    pub address: SocketAddr,
}

impl StunServer {
    /// The default public STUN server.
    #[must_use]
    pub fn google() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(74, 125, 250, 129)), 19302),
        }
    }

    /// Cloudflare public STUN server.
    #[must_use]
    pub fn cloudflare() -> Self {
        Self {
            address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(162, 159, 200, 1)), 3478),
        }
    }
}

/// Result of a successful STUN Binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StunBinding {
    /// Public IP as seen by the STUN server.
    pub mapped_address: IpAddr,
    /// Public port as seen by the STUN server.
    pub mapped_port: u16,
    /// STUN server that responded.
    pub server: String,
    /// Round-trip time of the STUN transaction in milliseconds.
    pub rtt_ms: u64,
}

/// 12-byte STUN transaction ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionId([u8; 12]);

impl TransactionId {
    /// Creates a new random transaction ID.
    #[must_use]
    pub fn random() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Returns the raw 12 bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 12] {
        &self.0
    }
}

/// Builds a STUN Binding Request packet (20 bytes).
#[must_use]
pub fn build_binding_request(transaction_id: &TransactionId) -> [u8; STUN_HEADER_SIZE] {
    let mut packet = [0u8; STUN_HEADER_SIZE];
    // Message Type: Binding Request (0x0001)
    packet[0] = (STUN_BINDING_REQUEST >> 8) as u8;
    packet[1] = STUN_BINDING_REQUEST as u8;
    // Message Length: 0 (no attributes)
    packet[2] = 0;
    packet[3] = 0;
    // Magic Cookie
    packet[4] = (STUN_MAGIC_COOKIE >> 24) as u8;
    packet[5] = (STUN_MAGIC_COOKIE >> 16) as u8;
    packet[6] = (STUN_MAGIC_COOKIE >> 8) as u8;
    packet[7] = STUN_MAGIC_COOKIE as u8;
    // Transaction ID (12 bytes)
    packet[8..20].copy_from_slice(transaction_id.as_bytes());
    packet
}

/// Error when parsing a STUN response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StunParseError {
    TooShort,
    BadMagicCookie,
    TransactionIdMismatch,
    NotBindingResponse,
    BindingError { code: u16, reason: String },
    NoMappedAddress,
    InvalidAttributeLength,
    UnsupportedAddressFamily,
}

impl std::fmt::Display for StunParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "STUN response too short"),
            Self::BadMagicCookie => write!(f, "STUN magic cookie mismatch"),
            Self::TransactionIdMismatch => write!(f, "STUN transaction ID mismatch"),
            Self::NotBindingResponse => write!(f, "not a STUN Binding Response"),
            Self::BindingError { code, reason } => {
                write!(f, "STUN Binding Error {code}: {reason}")
            }
            Self::NoMappedAddress => write!(f, "no XOR-MAPPED-ADDRESS in response"),
            Self::InvalidAttributeLength => write!(f, "invalid STUN attribute length"),
            Self::UnsupportedAddressFamily => write!(f, "unsupported STUN address family"),
        }
    }
}

impl std::error::Error for StunParseError {}

/// Parses a STUN Binding Response and extracts the XOR-MAPPED-ADDRESS.
///
/// # Errors
///
/// Returns [`StunParseError`] if the packet is malformed, has the wrong
/// transaction ID, or does not contain a usable mapped address.
pub fn parse_binding_response(
    data: &[u8],
    expected_tid: &TransactionId,
) -> Result<(IpAddr, u16), StunParseError> {
    if data.len() < STUN_HEADER_SIZE {
        return Err(StunParseError::TooShort);
    }
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

    if cookie != STUN_MAGIC_COOKIE {
        return Err(StunParseError::BadMagicCookie);
    }
    if &data[8..20] != expected_tid.as_bytes() {
        return Err(StunParseError::TransactionIdMismatch);
    }
    if msg_type == STUN_BINDING_ERROR {
        let reason = parse_error_code(&data[STUN_HEADER_SIZE..][..msg_len.min(data.len() - STUN_HEADER_SIZE)]);
        return Err(StunParseError::BindingError {
            code: reason.0,
            reason: reason.1,
        });
    }
    if msg_type != STUN_BINDING_RESPONSE {
        return Err(StunParseError::NotBindingResponse);
    }

    // Walk attributes to find XOR-MAPPED-ADDRESS (or MAPPED-ADDRESS fallback).
    let attrs = &data[STUN_HEADER_SIZE..][..msg_len.min(data.len() - STUN_HEADER_SIZE)];
    let mut mapped = None;
    let mut offset = 0;
    while offset + 4 <= attrs.len() {
        let attr_type = u16::from_be_bytes([attrs[offset], attrs[offset + 1]]);
        let attr_len = u16::from_be_bytes([attrs[offset + 2], attrs[offset + 3]]) as usize;
        let value_start = offset + 4;
        if value_start + attr_len > attrs.len() {
            return Err(StunParseError::InvalidAttributeLength);
        }
        let value = &attrs[value_start..value_start + attr_len];

        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                mapped = Some(parse_xor_mapped(value, &data[4..8], &data[8..20])?);
            }
            ATTR_MAPPED_ADDRESS if mapped.is_none() => {
                mapped = Some(parse_mapped(value)?);
            }
            _ => {}
        }
        // Attributes are padded to 4-byte boundary.
        offset = value_start + ((attr_len + 3) & !3);
    }
    mapped.ok_or(StunParseError::NoMappedAddress)
}

fn parse_xor_mapped(
    value: &[u8],
    magic: &[u8],
    tid: &[u8],
) -> Result<(IpAddr, u16), StunParseError> {
    if value.len() < 4 {
        return Err(StunParseError::InvalidAttributeLength);
    }
    let family = value[1];
    let xport = u16::from_be_bytes([value[2], value[3]]);
    let port = xport ^ (STUN_MAGIC_COOKIE >> 16) as u16;

    match family {
        FAMILY_IPV4 => {
            if value.len() < 8 {
                return Err(StunParseError::InvalidAttributeLength);
            }
            let mut addr_bytes = [0u8; 4];
            for i in 0..4 {
                addr_bytes[i] = value[4 + i] ^ magic[i];
            }
            Ok((IpAddr::V4(Ipv4Addr::from(addr_bytes)), port))
        }
        FAMILY_IPV6 => {
            if value.len() < 20 {
                return Err(StunParseError::InvalidAttributeLength);
            }
            let mut addr_bytes = [0u8; 16];
            for i in 0..4 {
                addr_bytes[i] = value[4 + i] ^ magic[i];
            }
            for i in 0..12 {
                addr_bytes[4 + i] = value[8 + i] ^ tid[i];
            }
            Ok((IpAddr::V6(Ipv6Addr::from(addr_bytes)), port))
        }
        _ => Err(StunParseError::UnsupportedAddressFamily),
    }
}

fn parse_mapped(value: &[u8]) -> Result<(IpAddr, u16), StunParseError> {
    if value.len() < 4 {
        return Err(StunParseError::InvalidAttributeLength);
    }
    let family = value[1];
    let port = u16::from_be_bytes([value[2], value[3]]);

    match family {
        FAMILY_IPV4 => {
            if value.len() < 8 {
                return Err(StunParseError::InvalidAttributeLength);
            }
            let addr = Ipv4Addr::new(value[4], value[5], value[6], value[7]);
            Ok((IpAddr::V4(addr), port))
        }
        FAMILY_IPV6 => {
            if value.len() < 20 {
                return Err(StunParseError::InvalidAttributeLength);
            }
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&value[4..20]);
            Ok((IpAddr::V6(Ipv6Addr::from(bytes)), port))
        }
        _ => Err(StunParseError::UnsupportedAddressFamily),
    }
}

fn parse_error_code(attrs: &[u8]) -> (u16, String) {
    let mut offset = 0;
    while offset + 4 <= attrs.len() {
        let attr_type = u16::from_be_bytes([attrs[offset], attrs[offset + 1]]);
        let attr_len = u16::from_be_bytes([attrs[offset + 2], attrs[offset + 3]]) as usize;
        let value_start = offset + 4;
        if attr_type == ATTR_ERROR_CODE && value_start + attr_len <= attrs.len() && attr_len >= 4 {
            let class = u16::from(attrs[value_start + 2] & 0x07) * 100;
            let number = u16::from(attrs[value_start + 3]);
            let code = class + number;
            let reason =
                String::from_utf8_lossy(&attrs[value_start + 4..value_start + attr_len]).into();
            return (code, reason);
        }
        offset = value_start + ((attr_len + 3) & !3);
    }
    (0, "unknown".into())
}

/// Queries a STUN server over a UDP socket.
///
/// Sends a STUN Binding Request, then listens on the socket for a matching
/// Binding Response. Non-STUN or mismatched packets are ignored to allow sharing
/// the socket.
///
/// # Errors
///
/// Returns a [`P2pError::StunTimeout`] if no matching response is received within
/// the timeout, or a [`P2pError::StunFailed`] if sending/receiving fails or STUN reports an error.
pub async fn query_stun(
    socket: &UdpSocket,
    server_addr: SocketAddr,
    timeout: Duration,
) -> Result<StunBinding, P2pError> {
    let tid = TransactionId::random();
    let request = build_binding_request(&tid);

    let start_time = std::time::Instant::now();

    socket
        .send_to(&request, server_addr)
        .await
        .map_err(|error| P2pError::StunFailed {
            reason: format!("failed to send STUN request to {server_addr}: {error}"),
        })?;

    let mut buf = [0u8; 1024];
    let sleep_timer = tokio::time::sleep(timeout);
    tokio::pin!(sleep_timer);

    loop {
        tokio::select! {
            _ = &mut sleep_timer => {
                return Err(P2pError::StunTimeout {
                    timeout_ms: timeout.as_millis() as u64,
                });
            }
            recv_res = socket.recv_from(&mut buf) => {
                match recv_res {
                    Ok((n, from)) => {
                        // In some NAT environments, the packet might be received from a slightly
                        // different port/address, but RFC 5389 requires it to originate from the STUN server IP.
                        if from.ip() == server_addr.ip() {
                            match parse_binding_response(&buf[..n], &tid) {
                                Ok((mapped_address, mapped_port)) => {
                                    return Ok(StunBinding {
                                        mapped_address,
                                        mapped_port,
                                        server: server_addr.to_string(),
                                        rtt_ms: start_time.elapsed().as_millis() as u64,
                                    });
                                }
                                Err(StunParseError::TransactionIdMismatch | StunParseError::BadMagicCookie) => {
                                    // Ignore concurrent packets or other traffic on this socket
                                }
                                Err(other_error) => {
                                    return Err(P2pError::StunFailed {
                                        reason: format!("STUN parse failed: {other_error}"),
                                    });
                                }
                            }
                        }
                    }
                    Err(error) => {
                        return Err(P2pError::StunFailed {
                            reason: format!("socket recv error during STUN query: {error}"),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_request_has_correct_magic_cookie() {
        let tid = TransactionId([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let packet = build_binding_request(&tid);
        assert_eq!(packet.len(), 20);
        // Magic Cookie at bytes 4–7
        let cookie = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
        assert_eq!(cookie, STUN_MAGIC_COOKIE);
        // Message type = Binding Request
        assert_eq!(u16::from_be_bytes([packet[0], packet[1]]), 0x0001);
        // Message length = 0
        assert_eq!(u16::from_be_bytes([packet[2], packet[3]]), 0);
        // Transaction ID
        assert_eq!(&packet[8..20], tid.as_bytes());
    }

    #[test]
    fn parse_ipv4_xor_mapped_address() {
        let tid = TransactionId([0; 12]);
        let _request = build_binding_request(&tid);

        // Build a minimal Binding Response with XOR-MAPPED-ADDRESS.
        // Response for 203.0.113.5:54321
        let ip = Ipv4Addr::new(203, 0, 113, 5);
        let port: u16 = 54321;

        let xport = port ^ (STUN_MAGIC_COOKIE >> 16) as u16;
        let magic_bytes = STUN_MAGIC_COOKIE.to_be_bytes();
        let ip_bytes = ip.octets();
        let xip: [u8; 4] = [
            ip_bytes[0] ^ magic_bytes[0],
            ip_bytes[1] ^ magic_bytes[1],
            ip_bytes[2] ^ magic_bytes[2],
            ip_bytes[3] ^ magic_bytes[3],
        ];

        let mut response = Vec::with_capacity(32);
        // Header
        response.extend_from_slice(&[0x01, 0x01]); // Binding Response
        response.extend_from_slice(&12u16.to_be_bytes()); // attr length
        response.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
        response.extend_from_slice(tid.as_bytes());
        // XOR-MAPPED-ADDRESS attribute
        response.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        response.extend_from_slice(&8u16.to_be_bytes()); // value length
        response.push(0x00); // reserved
        response.push(FAMILY_IPV4);
        response.extend_from_slice(&xport.to_be_bytes());
        response.extend_from_slice(&xip);

        let result = parse_binding_response(&response, &tid);
        assert!(result.is_ok(), "parse failed: {:?}", result.err());
        let (addr, p) = result.unwrap_or_else(|e| panic!("unwrap failed: {e:?}"));
        assert_eq!(addr, IpAddr::V4(ip));
        assert_eq!(p, port);
    }

    #[test]
    fn wrong_transaction_id_rejected() {
        let tid_a = TransactionId([1; 12]);
        let tid_b = TransactionId([2; 12]);

        let mut response = vec![0x01, 0x01, 0x00, 0x00];
        response.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
        response.extend_from_slice(tid_b.as_bytes());

        let result = parse_binding_response(&response, &tid_a);
        assert_eq!(result, Err(StunParseError::TransactionIdMismatch));
    }

    #[test]
    fn too_short_packet_rejected() {
        let tid = TransactionId([0; 12]);
        let result = parse_binding_response(&[0; 10], &tid);
        assert_eq!(result, Err(StunParseError::TooShort));
    }
}
