//! Bounded native host membership. Account ownership/consent is enforced by
//! the server; native credentials and disabled input enforce media permissions.
use std::collections::HashMap;

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    error::DesktopError,
    models::{EngineKind, LaunchEngineRequest, VideoCodec},
};

fn invalid(message: &str) -> DesktopError {
    DesktopError::InvalidRequest(message.into())
}

#[derive(Clone, PartialEq, Eq)]
struct Encoding {
    fps: u16,
    bitrate: u32,
    width: u16,
    height: u16,
    ultra: bool,
}

pub(crate) struct SharingPeer {
    group: Uuid,
    pub viewer: Uuid,
    session: Uuid,
    endpoint: String,
    port: u16,
    encoding: Encoding,
}

impl SharingPeer {
    pub fn from_request(request: &LaunchEngineRequest) -> Result<Option<Self>, DesktopError> {
        let (Some(group), Some(viewer)) = (&request.sharing_id, &request.sharing_viewer_id) else {
            if request.sharing_id.is_some() || request.sharing_viewer_id.is_some() {
                return Err(invalid(
                    "sharingId and sharingViewerId must be provided together",
                ));
            }
            return Ok(None);
        };
        if request.kind != EngineKind::Host
            || request.wire_protocol.as_deref() != Some("snv2")
            || request.input_enabled
            || request.audio_enabled
            || request.relative_mouse
            || request.relay
            || !matches!(request.codec, VideoCodec::Auto | VideoCodec::H264)
            || !(1_000..=200_000).contains(&request.bitrate_kbps)
        {
            return Err(invalid(
                "sharing requires a view-only SNV2/H.264 Direct host without audio",
            ));
        }
        let parse = |value: &str| {
            Uuid::parse_str(value).map_err(|_| invalid("sharing identifiers must be UUIDs"))
        };
        let session = parse(
            request
                .session_id
                .as_deref()
                .ok_or_else(|| invalid("sharing session is required"))?,
        )?;
        let address: std::net::Ipv4Addr = request
            .address
            .as_deref()
            .ok_or_else(|| invalid("sharing endpoint is required"))?
            .parse()
            .map_err(|_| invalid("sharing endpoint must be IPv4"))?;
        let remote = request
            .port
            .filter(|port| *port > 0)
            .ok_or_else(|| invalid("sharing remote port is required"))?;
        let port = request
            .udp_bind_port
            .filter(|port| *port > 0)
            .ok_or_else(|| invalid("sharing requires its own reserved UDP port"))?;
        let token = request
            .session_token
            .as_deref()
            .ok_or_else(|| invalid("sharing credential is required"))?;
        if !(32..=256).contains(&token.len())
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(invalid("sharing credential must be bounded base64url"));
        }
        Ok(Some(Self {
            group: parse(group)?,
            viewer: parse(viewer)?,
            session,
            endpoint: format!("{address}:{remote}"),
            port,
            encoding: Encoding {
                fps: request.fps,
                bitrate: request.bitrate_kbps,
                width: request.width,
                height: request.height,
                ultra: request.ultra_low_latency,
            },
        }))
    }
    pub fn add_command(&self, request_id: Uuid, token: &str) -> Zeroizing<String> {
        Zeroizing::new(format!(
            "share-add\t{request_id}\t{}\t{}\t{}\t{token}\n",
            self.viewer, self.port, self.endpoint
        ))
    }
}

pub(crate) struct SharingHost {
    group: Uuid,
    encoding: Encoding,
    viewers: HashMap<Uuid, SharingPeer>,
    pub desynchronized: bool,
}

impl SharingHost {
    pub fn new(peer: SharingPeer) -> Self {
        Self {
            group: peer.group,
            encoding: peer.encoding.clone(),
            viewers: HashMap::from([(peer.viewer, peer)]),
            desynchronized: false,
        }
    }
    pub fn validate_add(&self, peer: &SharingPeer) -> Result<(), DesktopError> {
        if self.desynchronized {
            return Err(invalid(
                "sharing command acknowledgement was lost; stop and restart this sharing host",
            ));
        }
        if self.group != peer.group || self.encoding != peer.encoding {
            return Err(invalid(
                "all viewers must use this sharing group and its existing encoder settings",
            ));
        }
        if self.viewers.len() >= 3 || self.viewers.contains_key(&peer.viewer) {
            return Err(invalid("sharing viewer limit or duplicate viewer"));
        }
        if self.viewers.values().any(|current| {
            current.session == peer.session
                || current.endpoint == peer.endpoint
                || current.port == peer.port
        }) {
            return Err(invalid(
                "each sharing viewer requires its own session, endpoint and UDP port",
            ));
        }
        Ok(())
    }
    pub fn add(&mut self, peer: SharingPeer) {
        self.viewers.insert(peer.viewer, peer);
    }
    pub fn validate_remove(&self, group: Uuid, viewer: Uuid) -> Result<bool, DesktopError> {
        if self.desynchronized || self.group != group {
            return Err(invalid("sharing group is unavailable or must be restarted"));
        }
        Ok(self.viewers.contains_key(&viewer))
    }
    pub fn remove(&mut self, viewer: Uuid) {
        self.viewers.remove(&viewer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(group: Uuid) -> Result<LaunchEngineRequest, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "kind":"host", "sessionId":Uuid::new_v4(), "sharingId":group, "sharingViewerId":Uuid::new_v4(),
            "wireProtocol":"snv2", "codec":"h264", "fps":60, "bitrateKbps":25000, "width":1920, "height":1080,
            "networkMode":"direct", "address":"192.0.2.1", "port":50001, "udpBindPort":50000, "sessionToken":"a".repeat(48)
        }))
    }
    fn peer(request: &LaunchEngineRequest) -> Result<SharingPeer, DesktopError> {
        SharingPeer::from_request(request)?
            .ok_or_else(|| invalid("test fixture is missing sharing"))
    }
    #[test]
    fn sharing_rejects_input_relay_audio_and_invalid_credentials()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut request = request(Uuid::new_v4())?;
        assert!(SharingPeer::from_request(&request)?.is_some());
        request.input_enabled = true;
        assert!(SharingPeer::from_request(&request).is_err());
        request.input_enabled = false;
        request.relay = true;
        assert!(SharingPeer::from_request(&request).is_err());
        request.relay = false;
        request.audio_enabled = true;
        assert!(SharingPeer::from_request(&request).is_err());
        request.audio_enabled = false;
        request.session_token = Some("a".repeat(48) + "\tshare-remove");
        assert!(SharingPeer::from_request(&request).is_err());
        Ok(())
    }
    #[test]
    fn membership_is_bounded_isolated_and_does_not_restart_on_remove()
    -> Result<(), Box<dyn std::error::Error>> {
        let group = Uuid::new_v4();
        let first = peer(&request(group)?)?;
        let viewer = first.viewer;
        let mut host = SharingHost::new(first);
        for index in 1..=2 {
            let mut next = request(group)?;
            next.port = Some(50001 + index * 2);
            next.udp_bind_port = Some(50000 + index * 2);
            let peer = peer(&next)?;
            host.validate_add(&peer)?;
            host.add(peer);
        }
        let mut fourth = request(group)?;
        fourth.port = Some(50007);
        fourth.udp_bind_port = Some(50006);
        let peer = peer(&fourth)?;
        assert!(host.validate_add(&peer).is_err());
        assert!(host.validate_remove(group, viewer)?);
        host.remove(viewer);
        assert_eq!(host.viewers.len(), 2);
        assert!(!host.validate_remove(group, viewer)?);
        assert!(host.validate_remove(Uuid::new_v4(), viewer).is_err());
        host.validate_add(&peer)?;
        host.desynchronized = true;
        assert!(host.validate_add(&peer).is_err());
        Ok(())
    }
    #[test]
    fn encoder_settings_and_socket_reservations_cannot_be_changed_by_new_viewer()
    -> Result<(), Box<dyn std::error::Error>> {
        let group = Uuid::new_v4();
        let host = SharingHost::new(peer(&request(group)?)?);
        let mut next = request(group)?;
        assert!(host.validate_add(&peer(&next)?).is_err());
        next.port = Some(50003);
        next.udp_bind_port = Some(50002);
        next.fps = 30;
        assert!(host.validate_add(&peer(&next)?).is_err());
        next.fps = 60;
        next.sharing_id = Some(Uuid::new_v4().to_string());
        assert!(host.validate_add(&peer(&next)?).is_err());
        Ok(())
    }
}
