//! NAT-PMP (RFC 6886) client.

use crate::mapping::{MappingConfig, MappingError, MappingProtocolKind, PortMapper, PortMapping};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

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
        if !self.gateway.is_ipv4() {
            return Err(MappingError::NotSupported);
        }
        map_on_gateway(config, internal_port, SocketAddr::new(self.gateway, 5351)).await
    }
}

async fn map_on_gateway(
    config: &MappingConfig,
    internal_port: u16,
    server: SocketAddr,
) -> Result<PortMapping, MappingError> {
    if !server.is_ipv4() || internal_port == 0 || config.lifetime_seconds == 0 {
        return Err(MappingError::NotSupported);
    }
    let socket =
        UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|error| MappingError::NetworkError {
                reason: error.to_string(),
            })?;
    socket
        .connect(server)
        .await
        .map_err(|error| MappingError::NetworkError {
            reason: error.to_string(),
        })?;
    let local = socket
        .local_addr()
        .map_err(|error| MappingError::NetworkError {
            reason: error.to_string(),
        })?
        .ip();
    // The MAP reply contains no public IP. Query external-address opcode 0;
    // returning the private gateway address creates a fake Direct candidate.
    let address = super::exchange(&socket, &[0, NATPMP_OPCODE_EXTERNAL_ADDRESS], |r| {
        r.len() >= 8
            && r[0] == 0
            && r[1] == 128 + NATPMP_OPCODE_EXTERNAL_ADDRESS
            && (r[2..4] != [0, 0] || r.len() == 12)
    })
    .await?;
    check_result(&address)?;
    let public = std::net::Ipv4Addr::new(address[8], address[9], address[10], address[11]);
    let mut request = [0u8; 12];
    request[1] = NATPMP_OPCODE_MAP_UDP;
    request[4..6].copy_from_slice(&internal_port.to_be_bytes());
    request[6..8].copy_from_slice(&config.requested_port.to_be_bytes());
    request[8..12].copy_from_slice(&config.lifetime_seconds.to_be_bytes());
    let response = super::exchange(&socket, &request, |r| {
        r.len() >= 8
            && r[0] == 0
            && r[1] == 128 + NATPMP_OPCODE_MAP_UDP
            && (r[2..4] != [0, 0] || (r.len() == 16 && r[8..10] == request[4..6]))
    })
    .await?;
    check_result(&response)?;
    let port = u16::from_be_bytes(response[10..12].try_into().unwrap());
    let lifetime = u32::from_be_bytes(response[12..16].try_into().unwrap());
    if port == 0 || lifetime == 0 {
        return Err(MappingError::Expired);
    }
    Ok(PortMapping {
        protocol: MappingProtocolKind::NatPmp,
        external: SocketAddr::new(public.into(), port),
        internal: SocketAddr::new(local, internal_port),
        ttl: Duration::from_secs(lifetime.into()),
        needs_renewal: true,
    })
}
fn check_result(response: &[u8]) -> Result<(), MappingError> {
    let code = u16::from_be_bytes(response[2..4].try_into().unwrap());
    if code != 0 {
        Err(MappingError::Refused {
            reason: format!("NAT-PMP result {code}"),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn nat_pmp_queries_public_address_instead_of_advertising_private_gateway() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let mut request = [0; 128];
            let (size, client) = server.recv_from(&mut request).await.unwrap();
            assert_eq!(&request[..size], &[0, 0]);
            let mut response = [0u8; 12];
            response[1] = 128;
            response[8..12].copy_from_slice(&[8, 8, 4, 4]);
            server.send_to(&response, client).await.unwrap();
            let (size, client) = server.recv_from(&mut request).await.unwrap();
            assert_eq!(size, 12);
            assert_eq!(request[1], 1);
            let mut response = [0u8; 16];
            response[1] = 129;
            response[8..10].copy_from_slice(&request[4..6]);
            response[10..12].copy_from_slice(&55002u16.to_be_bytes());
            response[12..16].copy_from_slice(&120u32.to_be_bytes());
            let mut wrong = response;
            wrong[8] ^= 1;
            server.send_to(&wrong, client).await.unwrap();
            server.send_to(&response, client).await.unwrap();
        });
        let mapping = map_on_gateway(&MappingConfig::default(), 50000, address)
            .await
            .unwrap();
        responder.await.unwrap();
        assert_eq!(mapping.external, "8.8.4.4:55002".parse().unwrap());
        assert_eq!(mapping.internal, "127.0.0.1:50000".parse().unwrap());
        assert_eq!(mapping.protocol, MappingProtocolKind::NatPmp);
    }
}
