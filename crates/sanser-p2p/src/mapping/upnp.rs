//! UPnP IGD (Internet Gateway Device) client.

use crate::mapping::{MappingConfig, MappingError, MappingProtocolKind, PortMapper, PortMapping};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

/// UPnP IGD service types.
pub const URN_WAN_IP_CONNECTION: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";
pub const URN_WAN_PPP_CONNECTION: &str = "urn:schemas-upnp-org:service:WANPPPConnection:1";

/// UPnP discovery parameters.
pub const SSDP_MULTICAST_ADDR: &str = "239.255.255.250:1900";
pub const SSDP_MX: u8 = 3;

/// UPnP error codes returned by routers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpnpError {
    NoIgdFound,
    ActionFailed,
    ConflictInMappingEntry,
    NoSuchEntryInArray,
    WildCardNotPermitted,
    InternalPortWildcard,
    SamePortRequired,
    OnlyPermanentLease,
    ExternalPortInUse,
}

pub struct UpnpMapper;

#[async_trait::async_trait]
impl PortMapper for UpnpMapper {
    async fn try_map_port(
        &self,
        config: &MappingConfig,
        internal_port: u16,
    ) -> Result<PortMapping, MappingError> {
        let socket =
            UdpSocket::bind("0.0.0.0:0")
                .await
                .map_err(|e| MappingError::NetworkError {
                    reason: e.to_string(),
                })?;

        let msearch = format!(
            "M-SEARCH * HTTP/1.1\r\n\
             HOST: 239.255.255.250:1900\r\n\
             MAN: \"ssdp:discover\"\r\n\
             MX: {}\r\n\
             ST: {}\r\n\r\n",
            SSDP_MX, URN_WAN_IP_CONNECTION
        );

        socket
            .send_to(msearch.as_bytes(), SSDP_MULTICAST_ADDR)
            .await
            .map_err(|e| MappingError::NetworkError {
                reason: e.to_string(),
            })?;

        let mut buf = [0u8; 1024];
        let sleep_timer = tokio::time::sleep(Duration::from_millis(200));
        tokio::pin!(sleep_timer);

        let location = tokio::select! {
            _ = &mut sleep_timer => {
                return Err(MappingError::Timeout);
            }
            recv_res = socket.recv_from(&mut buf) => {
                match recv_res {
                    Ok((n, _)) => {
                        let resp = String::from_utf8_lossy(&buf[..n]);
                        resp.lines()
                            .find(|line| line.to_uppercase().starts_with("LOCATION:"))
                            .and_then(|line| line.split_once(':'))
                            .map(|(_, val)| val.trim().to_owned())
                    }
                    Err(e) => return Err(MappingError::NetworkError { reason: e.to_string() }),
                }
            }
        };

        let location = location.ok_or(MappingError::NotSupported)?;
        let parsed_url = url::Url::parse(&location).map_err(|_| MappingError::NetworkError {
            reason: "invalid location".into(),
        })?;

        let host = parsed_url.host_str().ok_or(MappingError::NetworkError {
            reason: "no host".into(),
        })?;
        let port = parsed_url.port().unwrap_or(80);
        let control_addr: SocketAddr =
            format!("{host}:{port}")
                .parse()
                .map_err(|e| MappingError::NetworkError {
                    reason: format!("invalid control addr: {e}"),
                })?;

        let mut client =
            TcpStream::connect(control_addr)
                .await
                .map_err(|e| MappingError::NetworkError {
                    reason: e.to_string(),
                })?;

        let req = format!(
            "GET {} HTTP/1.1\r\n\
             Host: {}\r\n\
             Connection: close\r\n\r\n",
            parsed_url.path(),
            host
        );
        client
            .write_all(req.as_bytes())
            .await
            .map_err(|e| MappingError::NetworkError {
                reason: e.to_string(),
            })?;

        let mut root_xml = String::new();
        client
            .read_to_string(&mut root_xml)
            .await
            .map_err(|e| MappingError::NetworkError {
                reason: e.to_string(),
            })?;

        let (service_type, control_path) =
            parse_upnp_desc(&root_xml).ok_or(MappingError::NotSupported)?;

        let mut client =
            TcpStream::connect(control_addr)
                .await
                .map_err(|e| MappingError::NetworkError {
                    reason: e.to_string(),
                })?;

        let local_ip = socket.local_addr().unwrap().ip().to_string();

        let soap_body = format!(
            "<?xml version=\"1.0\"?>\n\
             <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\n\
             <s:Body>\n\
             <u:AddPortMapping xmlns:u=\"{}\">\n\
             <NewRemoteHost></NewRemoteHost>\n\
             <NewExternalPort>{}</NewExternalPort>\n\
             <NewProtocol>UDP</NewProtocol>\n\
             <NewInternalPort>{}</NewInternalPort>\n\
             <NewInternalClient>{}</NewInternalClient>\n\
             <NewEnabled>1</NewEnabled>\n\
             <NewPortMappingDescription>Sanser P2P</NewPortMappingDescription>\n\
             <NewLeaseDuration>{}</NewLeaseDuration>\n\
             </u:AddPortMapping>\n\
             </s:Body>\n\
             </s:Envelope>",
            service_type, config.requested_port, internal_port, local_ip, config.lifetime_seconds
        );

        let soap_req = format!(
            "POST {} HTTP/1.1\r\n\
             Host: {}\r\n\
             Content-Length: {}\r\n\
             Content-Type: text/xml; charset=\"utf-8\"\r\n\
             SOAPAction: \"{}#AddPortMapping\"\r\n\
             Connection: close\r\n\r\n\
             {}",
            control_path,
            host,
            soap_body.len(),
            service_type,
            soap_body
        );

        client
            .write_all(soap_req.as_bytes())
            .await
            .map_err(|e| MappingError::NetworkError {
                reason: e.to_string(),
            })?;

        let mut resp = String::new();
        client
            .read_to_string(&mut resp)
            .await
            .map_err(|e| MappingError::NetworkError {
                reason: e.to_string(),
            })?;

        if !resp.contains("200 OK") && !resp.contains("AddPortMappingResponse") {
            return Err(MappingError::Refused {
                reason: "soap rejected".into(),
            });
        }

        Ok(PortMapping {
            protocol: MappingProtocolKind::Upnp,
            external: SocketAddr::new(control_addr.ip(), config.requested_port),
            internal: SocketAddr::new(socket.local_addr().unwrap().ip(), internal_port),
            ttl: Duration::from_secs(u64::from(config.lifetime_seconds)),
            needs_renewal: true,
        })
    }
}

fn parse_upnp_desc(xml: &str) -> Option<(String, String)> {
    let targets = [URN_WAN_IP_CONNECTION, URN_WAN_PPP_CONNECTION];
    for &target in &targets {
        if let Some(idx) = xml.find(target) {
            let sub = &xml[idx..];
            if let Some(c_idx) = sub.find("<controlURL>") {
                let start = c_idx + "<controlURL>".len();
                if let Some(end) = sub[start..].find("</controlURL>") {
                    let control_path = sub[start..start + end].trim().to_owned();
                    return Some((target.to_owned(), control_path));
                }
            }
        }
    }
    None
}
