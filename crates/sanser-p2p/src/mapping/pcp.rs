//! PCP (Port Control Protocol, RFC 6887) client.

use crate::mapping::{PortMapper, MappingConfig, PortMapping, MappingError, MappingProtocolKind};
use tokio::net::UdpSocket;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// PCP opcodes.
pub const PCP_OPCODE_MAP: u8 = 1;
pub const PCP_OPCODE_PEER: u8 = 2;

/// PCP result codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcpResult {
    Success,
    UnsupportedVersion,
    NotAuthorized,
    MalformedRequest,
    UnsupportedOpcode,
    UnsupportedOption,
    MalformedOption,
    NetworkFailure,
    NoResources,
    UnsupportedProtocol,
    AddressMismatch,
    ExcessiveRemotePeers,
    CannotProvideExternal,
}

pub struct PcpMapper {
    pub gateway: IpAddr,
}

#[async_trait::async_trait]
impl PortMapper for PcpMapper {
    async fn try_map_port(
        &self,
        config: &MappingConfig,
        internal_port: u16,
    ) -> Result<PortMapping, MappingError> {
        let socket = UdpSocket::bind(if self.gateway.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).await
            .map_err(|e| MappingError::NetworkError { reason: e.to_string() })?;

        let server_addr = SocketAddr::new(self.gateway, 5351);

        let mut req = vec![0u8; 60];
        req[0] = 2; // Version 2
        req[1] = PCP_OPCODE_MAP; // Opcode 1 (Map)
        // 2..4 reserved (0)
        req[4..8].copy_from_slice(&config.lifetime_seconds.to_be_bytes());

        match socket.local_addr().unwrap().ip() {
            IpAddr::V4(v4) => {
                req[8..20].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff]);
                req[20..24].copy_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                req[8..24].copy_from_slice(&v6.octets());
            }
        }

        let mut nonce = [0u8; 12];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
        req[24..36].copy_from_slice(&nonce);

        req[36] = 17; // Protocol 17 (UDP)
        req[40..42].copy_from_slice(&internal_port.to_be_bytes());
        req[42..44].copy_from_slice(&config.requested_port.to_be_bytes());

        socket.send_to(&req, server_addr).await
            .map_err(|e| MappingError::NetworkError { reason: e.to_string() })?;

        let mut buf = [0u8; 128];
        let sleep_timer = tokio::time::sleep(Duration::from_millis(200));
        tokio::pin!(sleep_timer);

        tokio::select! {
            _ = &mut sleep_timer => {
                Err(MappingError::Timeout)
            }
            recv_res = socket.recv_from(&mut buf) => {
                match recv_res {
                    Ok((n, _)) => {
                        if n < 60 {
                            return Err(MappingError::NetworkError { reason: "response too short".into() });
                        }
                        if buf[0] != 2 {
                            return Err(MappingError::NetworkError { reason: "invalid version".into() });
                        }
                        if buf[1] != 128 + PCP_OPCODE_MAP {
                            return Err(MappingError::NetworkError { reason: "invalid opcode".into() });
                        }
                        let result_code = buf[3];
                        if result_code != 0 {
                            return Err(MappingError::Refused { reason: format!("error code: {result_code}") });
                        }
                        let lifetime = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
                        if buf[24..36] != nonce {
                            return Err(MappingError::NetworkError { reason: "nonce mismatch".into() });
                        }
                        let int_port = u16::from_be_bytes([buf[40], buf[41]]);
                        let ext_port = u16::from_be_bytes([buf[42], buf[43]]);

                        let mut ext_ip_bytes = [0u8; 16];
                        ext_ip_bytes.copy_from_slice(&buf[44..60]);
                        let ext_ip = if ext_ip_bytes[..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff] {
                            IpAddr::V4(std::net::Ipv4Addr::new(ext_ip_bytes[12], ext_ip_bytes[13], ext_ip_bytes[14], ext_ip_bytes[15]))
                        } else {
                            IpAddr::V6(std::net::Ipv6Addr::from(ext_ip_bytes))
                        };

                        Ok(PortMapping {
                            protocol: MappingProtocolKind::Pcp,
                            external: SocketAddr::new(ext_ip, ext_port),
                            internal: SocketAddr::new(socket.local_addr().unwrap().ip(), int_port),
                            ttl: Duration::from_secs(u64::from(lifetime)),
                            needs_renewal: true,
                        })
                    }
                    Err(e) => Err(MappingError::NetworkError { reason: e.to_string() }),
                }
            }
        }
    }
}
