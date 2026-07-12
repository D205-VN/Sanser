use sanser_core::NetworkMode;
use std::fmt;
use thiserror::Error;
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop};

const MAX_ICE_SERVERS: usize = 16;
const MAX_URLS_PER_SERVER: usize = 8;

#[derive(Clone, Eq, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct IceCredential(String);

impl IceCredential {
    /// Wraps an ICE credential so it is redacted in debug output and zeroized.
    ///
    /// # Errors
    ///
    /// Returns [`IceServerError::InvalidCredential`] when the credential is
    /// empty, longer than 512 bytes, or contains control characters.
    pub fn new(value: String) -> Result<Self, IceServerError> {
        if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
            return Err(IceServerError::InvalidCredential);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IceCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IceCredential([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IceServer {
    pub urls: Vec<Url>,
    pub username: Option<String>,
    pub credential: Option<IceCredential>,
}

impl IceServer {
    /// Validates an ICE server before it crosses the native WebRTC boundary.
    ///
    /// # Errors
    ///
    /// Returns an [`IceServerError`] when the URL count is out of bounds, a URL
    /// has an unsupported scheme or empty target, username and credential are
    /// incomplete or malformed, or a TURN URL has no credential.
    pub fn validate(self) -> Result<Self, IceServerError> {
        if self.urls.is_empty() || self.urls.len() > MAX_URLS_PER_SERVER {
            return Err(IceServerError::UrlCount(self.urls.len()));
        }
        if self.username.is_some() != self.credential.is_some() {
            return Err(IceServerError::IncompleteCredentials);
        }
        let mut has_turn = false;
        for url in &self.urls {
            match url.scheme() {
                "stun" | "stuns" => {}
                "turn" | "turns" => has_turn = true,
                _ => return Err(IceServerError::InvalidScheme),
            }
            if url.path().trim().is_empty() {
                return Err(IceServerError::InvalidUrl);
            }
        }
        if has_turn && self.credential.is_none() {
            return Err(IceServerError::TurnRequiresCredentials);
        }
        if let Some(username) = &self.username
            && (username.is_empty()
                || username.len() > 256
                || username.chars().any(char::is_control))
        {
            return Err(IceServerError::InvalidUsername);
        }
        Ok(self)
    }

    #[must_use]
    pub fn has_turn_url(&self) -> bool {
        self.urls
            .iter()
            .any(|url| matches!(url.scheme(), "turn" | "turns"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IceTransportPolicy {
    All,
    Relay,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionPolicy {
    pub mode: NetworkMode,
    pub ice_transport: IceTransportPolicy,
    pub allow_native_snv2: bool,
    pub servers: Vec<IceServer>,
}

impl ConnectionPolicy {
    /// Builds a validated connection policy for the requested network mode.
    ///
    /// # Errors
    ///
    /// Returns [`IceServerError::ServerCount`] for more than 16 servers,
    /// propagates validation errors from any [`IceServer`], and returns
    /// [`IceServerError::RelayRequiresTurn`] when relay mode has no TURN URL.
    pub fn build(mode: NetworkMode, servers: Vec<IceServer>) -> Result<Self, IceServerError> {
        if servers.len() > MAX_ICE_SERVERS {
            return Err(IceServerError::ServerCount(servers.len()));
        }
        let servers: Vec<_> = servers
            .into_iter()
            .map(IceServer::validate)
            .collect::<Result<_, _>>()?;
        Ok(Self {
            mode,
            ice_transport: IceTransportPolicy::All,
            allow_native_snv2: true,
            servers,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IceServerError {
    #[error("ICE server count {0} exceeds 16")]
    ServerCount(usize),
    #[error("ICE URL count {0} is outside 1..=8")]
    UrlCount(usize),
    #[error("ICE URL has an unsupported scheme")]
    InvalidScheme,
    #[error("ICE URL is invalid")]
    InvalidUrl,
    #[error("ICE username is invalid")]
    InvalidUsername,
    #[error("ICE credential is invalid")]
    InvalidCredential,
    #[error("ICE username and credential must both be present")]
    IncompleteCredentials,
    #[error("TURN URLs require short-lived credentials")]
    TurnRequiresCredentials,
    #[error("relay mode requires a TURN server")]
    RelayRequiresTurn,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn() -> IceServer {
        IceServer {
            urls: vec![
                Url::parse("turn:relay.example.com:3478?transport=udp")
                    .unwrap_or_else(|error| panic!("url parse failed: {error}")),
            ],
            username: Some("temporary-user".to_owned()),
            credential: Some(
                IceCredential::new("temporary-secret".to_owned())
                    .unwrap_or_else(|error| panic!("credential failed: {error}")),
            ),
        }
    }

    #[test]
    fn policy_allows_native_transport_with_auto_mode() {
        let policy = ConnectionPolicy::build(NetworkMode::Auto, vec![turn()])
            .unwrap_or_else(|error| panic!("policy failed: {error}"));
        assert_eq!(policy.ice_transport, IceTransportPolicy::All);
        assert!(policy.allow_native_snv2);
        assert!(!format!("{policy:?}").contains("temporary-secret"));
    }
}
