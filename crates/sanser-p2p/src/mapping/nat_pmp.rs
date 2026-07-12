//! NAT-PMP (RFC 6886) client.

use crate::mapping::{PortMapper, MappingConfig, PortMapping, MappingError, MappingProtocolKind};
use tokio::net::UdpSocket;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

/// NAT-PMP opcodes.
pub const NATPMP_OPCODE_EXTERNAL_ADDRESS: u8 = 0;
pub const NATPMP_OPCODE_MAP_UDP: u8 = 1;
pub const NATPMP_OPCODE_MAP_TCP: u8 = 2;

/// NAT-PMP result codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NatPmpResult {
    Success,
    UnsupportedVersion,
    NotAuthorized,
    NetworkFailure,
    OutOfResources,
    UnsupportedOpcode,
}

pub struct NatPmpMapper {
    pub gateway: IpAddr,
}

#[async_trait::async_trait]
impl PortMapper for NatPmpMapper {
    async fn try_map_port(
        &self,
        config: &MappingConfig,
        internal_port: u16,
    ) -> Result<PortMapping, MappingError> {
        let gateway_ip = match self.gateway {
            IpAddr::V4(v4) => v4,
            IpAddr::V6(_) => return Err(MappingError::NotSupported), // NAT-PMP is IPv4 only
        };

        let socket = UdpSocket::bind("0.0.0.0:0").await
            .map_err(|e| MappingError::NetworkError { reason: e.to_string() })?;

        let server_addr = SocketAddr::new(IpAddr::V4(gateway_ip), 5351);

        let mut req = [0u8; 12];
        req[0] = 0; // Version 0
        req[1] = NATPMP_OPCODE_MAP_UDP; // Opcode 1 (UDP)
        // 2..4 reserved (0)
        req[4..6].copy_from_slice(&internal_port.to_be_bytes());
        req[6..8].copy_from_slice(&config.requested_port.to_be_bytes());
        req[8..12].copy_from_slice(&config.lifetime_seconds.to_be_bytes());

        socket.send_to(&req, server_addr).await
            .map_err(|e| MappingError::NetworkError { reason: e.to_string() })?;

        let mut buf = [0u8; 16];
        let sleep_timer = tokio::time::sleep(Duration::from_millis(200));
        tokio::pin!(sleep_timer);

        tokio::select! {
            _ = &mut sleep_timer => {
                Err(MappingError::Timeout)
            }
            recv_res = socket.recv_from(&mut buf) => {
                match recv_res {
                    Ok((n, _)) => {
                        if n < 16 {
                            return Err(MappingError::NetworkError { reason: "response too short".into() });
                        }
                        if buf[0] != 0 {
                            return Err(MappingError::NetworkError { reason: "invalid version".into() });
                        }
                        if buf[1] != 128 + NATPMP_OPCODE_MAP_UDP {
                            return Err(MappingError::NetworkError { reason: "invalid opcode".into() });
                        }
                        let result_code = u16::from_be_bytes([buf[2], buf[3]]);
                        if result_code != 0 {
                            return Err(MappingError::Refused { reason: format!("error: {result_code}") });
                        }
                        let int_port = u16::from_be_bytes([buf[8], buf[9]]);
                        let ext_port = u16::from_be_bytes([buf[10], buf[11]]);
                        let lifetime = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);

                        Ok(PortMapping {
                            protocol: MappingProtocolKind::NatPmp,
                            external: SocketAddr::new(self.gateway, ext_port),
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
