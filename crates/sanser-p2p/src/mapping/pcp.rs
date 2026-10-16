//! PCP (Port Control Protocol, RFC 6887) client.

use crate::mapping::{MappingConfig, MappingError, MappingProtocolKind, PortMapper, PortMapping};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

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
        map_on_gateway(config, internal_port, SocketAddr::new(self.gateway, 5351)).await
    }
}

async fn map_on_gateway(
    config: &MappingConfig,
    internal_port: u16,
    server: SocketAddr,
) -> Result<PortMapping, MappingError> {
    if internal_port == 0 || config.lifetime_seconds == 0 {
        return Err(MappingError::NotSupported);
    }
    let socket = UdpSocket::bind(if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
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
    // connect selects the source interface. getsockname before connect would
    // return 0.0.0.0, which is not a valid PCP client address.
    let local = socket
        .local_addr()
        .map_err(|error| MappingError::NetworkError {
            reason: error.to_string(),
        })?
        .ip();
    let mut request = [0u8; 60];
    request[0] = 2;
    request[1] = PCP_OPCODE_MAP;
    request[4..8].copy_from_slice(&config.lifetime_seconds.to_be_bytes());
    let client = match local {
        IpAddr::V4(ip) => ip.to_ipv6_mapped(),
        IpAddr::V6(ip) => ip,
    };
    request[8..24].copy_from_slice(&client.octets());
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut request[24..36]);
    request[36] = 17;
    request[40..42].copy_from_slice(&internal_port.to_be_bytes());
    request[42..44].copy_from_slice(&config.requested_port.to_be_bytes());
    let response = super::exchange(&socket, &request, |r| {
        r.len() >= 24
            && r[0] == 2
            && r[1] == 128 + PCP_OPCODE_MAP
            && (r[3] != 0
                || (r.len() >= 60
                    && r[24..36] == request[24..36]
                    && r[36] == 17
                    && r[40..42] == request[40..42]))
    })
    .await?;
    if response[3] != 0 {
        return Err(MappingError::Refused {
            reason: format!("PCP result {}", response[3]),
        });
    }
    let lifetime = u32::from_be_bytes(response[4..8].try_into().unwrap());
    let port = u16::from_be_bytes(response[42..44].try_into().unwrap());
    if lifetime == 0 || port == 0 {
        return Err(MappingError::Expired);
    }
    let ip = std::net::Ipv6Addr::from(<[u8; 16]>::try_from(&response[44..60]).unwrap());
    let external = ip
        .to_ipv4_mapped()
        .map(IpAddr::V4)
        .unwrap_or(IpAddr::V6(ip));
    Ok(PortMapping {
        protocol: MappingProtocolKind::Pcp,
        external: SocketAddr::new(external, port),
        internal: SocketAddr::new(local, internal_port),
        ttl: Duration::from_secs(lifetime.into()),
        needs_renewal: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pcp_uses_real_source_and_validates_nonce_and_internal_port() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let mut request = [0; 128];
            let (size, client) = server.recv_from(&mut request).await.unwrap();
            assert_eq!(size, 60);
            assert_eq!(
                &request[8..24],
                &std::net::Ipv4Addr::LOCALHOST.to_ipv6_mapped().octets()
            );
            let mut reply = request[..60].to_vec();
            reply[1] |= 128;
            reply[42..44].copy_from_slice(&55001u16.to_be_bytes());
            reply[44..60].copy_from_slice(
                &std::net::Ipv4Addr::new(8, 8, 8, 8)
                    .to_ipv6_mapped()
                    .octets(),
            );
            let mut wrong = reply.clone();
            wrong[24] ^= 1;
            server.send_to(&wrong, client).await.unwrap();
            wrong = reply.clone();
            wrong[40] ^= 1;
            server.send_to(&wrong, client).await.unwrap();
            server.send_to(&reply, client).await.unwrap();
        });
        let mapping = map_on_gateway(&MappingConfig::default(), 50000, address)
            .await
            .unwrap();
        responder.await.unwrap();
        assert_eq!(mapping.external, "8.8.8.8:55001".parse().unwrap());
        assert_eq!(mapping.internal, "127.0.0.1:50000".parse().unwrap());
        assert_eq!(mapping.protocol, MappingProtocolKind::Pcp);
    }
}
