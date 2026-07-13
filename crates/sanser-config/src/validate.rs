use crate::SecretString;
use sanser_core::{NetworkMode, PROTOCOL_VERSION, VERSION};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
};
use thiserror::Error;
use url::Url;

const MAX_LIST_ENTRIES: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppConfig {
    pub version: String,
    pub protocol_version: u8,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub network: NetworkConfig,
    pub auth: AuthConfig,
    pub rust_log: String,
}

impl AppConfig {
    /// Loads and validates configuration from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when a required value is missing or invalid.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(std::env::vars())
    }

    /// Validates configuration supplied as key/value pairs.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when a required value is missing or invalid.
    pub fn from_values<I, K, V>(values: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let values: BTreeMap<String, String> = values
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();

        let version = value_or(&values, "SANSER_VERSION", VERSION).to_owned();
        if version != VERSION {
            return Err(ConfigError::Version {
                expected: VERSION,
                actual: version,
            });
        }
        let protocol_version = parse_or(&values, "SANSER_PROTOCOL_VERSION", PROTOCOL_VERSION)?;
        if protocol_version != PROTOCOL_VERSION {
            return Err(ConfigError::ProtocolVersion {
                expected: PROTOCOL_VERSION,
                actual: protocol_version,
            });
        }

        let host = parse_or(&values, "SERVER_HOST", IpAddr::V4(Ipv4Addr::LOCALHOST))?;
        let port = parse_or(&values, "SERVER_PORT", 5_174_u16)?;
        if port == 0 {
            return Err(ConfigError::InvalidValue("SERVER_PORT"));
        }
        let public_base_url = parse_http_url(
            "PUBLIC_BASE_URL",
            value_or(&values, "PUBLIC_BASE_URL", "http://127.0.0.1:5174"),
        )?;
        let allowed_origins = parse_http_url_list(
            "ALLOWED_ORIGINS",
            value_or(&values, "ALLOWED_ORIGINS", "http://127.0.0.1:5174"),
        )?;

        let database = parse_database(&values)?;

        let mode = parse_network_mode(value_or(&values, "NETWORK_MODE", "auto"))?;
        let stun_urls = parse_ice_urls(
            "STUN_URLS",
            value_or(&values, "STUN_URLS", "stun:stun.l.google.com:19302"),
            &["stun", "stuns"],
        )?;
        let turn_urls = parse_ice_urls(
            "TURN_URLS",
            value_or(&values, "TURN_URLS", ""),
            &["turn", "turns"],
        )?;
        let turn_username = values
            .get("TURN_USERNAME")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let turn_credential = values
            .get("TURN_CREDENTIAL")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if turn_username.is_some() != turn_credential.is_some() {
            return Err(ConfigError::IncompleteTurnCredentials);
        }
        // Relay checks removed as TURN is deprecated.
        let turn = if turn_urls.is_empty() {
            None
        } else {
            Some(TurnConfig {
                urls: turn_urls,
                username: turn_username.unwrap_or_default(),
                credential: SecretString::new(turn_credential.unwrap_or_default()),
            })
        };

        let access_token_ttl_seconds = parse_or(&values, "ACCESS_TOKEN_TTL_SECONDS", 900_u64)?;
        let refresh_token_ttl_seconds =
            parse_or(&values, "REFRESH_TOKEN_TTL_SECONDS", 2_592_000_u64)?;
        if !(60..=86_400).contains(&access_token_ttl_seconds) {
            return Err(ConfigError::InvalidValue("ACCESS_TOKEN_TTL_SECONDS"));
        }
        if refresh_token_ttl_seconds <= access_token_ttl_seconds
            || refresh_token_ttl_seconds > 31_536_000
        {
            return Err(ConfigError::InvalidValue("REFRESH_TOKEN_TTL_SECONDS"));
        }

        let rust_log = parse_rust_log(&values)?;

        Ok(Self {
            version: VERSION.to_owned(),
            protocol_version,
            server: ServerConfig {
                host,
                port,
                public_base_url,
                allowed_origins,
            },
            database,
            network: NetworkConfig {
                mode,
                stun_urls,
                turn,
            },
            auth: AuthConfig {
                access_token_ttl_seconds,
                refresh_token_ttl_seconds,
            },
            rust_log,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
    pub public_base_url: Url,
    pub allowed_origins: Vec<Url>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseConfig {
    pub url: SecretString,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkConfig {
    pub mode: NetworkMode,
    pub stun_urls: Vec<Url>,
    pub turn: Option<TurnConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnConfig {
    pub urls: Vec<Url>,
    pub username: String,
    pub credential: SecretString,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthConfig {
    pub access_token_ttl_seconds: u64,
    pub refresh_token_ttl_seconds: u64,
}

fn value_or<'a>(values: &'a BTreeMap<String, String>, key: &str, default: &'a str) -> &'a str {
    values.get(key).map_or(default, String::as_str)
}

fn parse_database(values: &BTreeMap<String, String>) -> Result<DatabaseConfig, ConfigError> {
    let database_url = values
        .get("DATABASE_URL")
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::Missing("DATABASE_URL"))?;
    Ok(DatabaseConfig {
        url: SecretString::new(validate_database_url(database_url)?),
    })
}

fn parse_rust_log(values: &BTreeMap<String, String>) -> Result<String, ConfigError> {
    let rust_log = value_or(values, "RUST_LOG", "info").trim().to_owned();
    if rust_log.is_empty() || rust_log.len() > 256 || rust_log.chars().any(char::is_control) {
        return Err(ConfigError::InvalidValue("RUST_LOG"));
    }
    Ok(rust_log)
}

fn parse_or<T>(
    values: &BTreeMap<String, String>,
    key: &'static str,
    default: T,
) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
{
    values.get(key).map_or(Ok(default), |value| {
        value
            .trim()
            .parse()
            .map_err(|_| ConfigError::InvalidValue(key))
    })
}

fn parse_network_mode(value: &str) -> Result<NetworkMode, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" | "relay" => Ok(NetworkMode::Auto), // Fallback relay config to auto
        "direct" | "directonly" => Ok(NetworkMode::DirectOnly),
        "manual" => Ok(NetworkMode::Manual),
        _ => Err(ConfigError::InvalidValue("NETWORK_MODE")),
    }
}

fn parse_http_url(name: &'static str, value: &str) -> Result<Url, ConfigError> {
    let parsed = Url::parse(value.trim()).map_err(|_| ConfigError::InvalidValue(name))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ConfigError::InvalidValue(name));
    }
    Ok(parsed)
}

fn parse_http_url_list(name: &'static str, value: &str) -> Result<Vec<Url>, ConfigError> {
    let entries = split_list(name, value)?;
    if entries.is_empty() {
        return Err(ConfigError::Missing(name));
    }
    entries
        .into_iter()
        .map(|entry| parse_http_url(name, entry))
        .collect()
}

fn parse_ice_urls(
    name: &'static str,
    value: &str,
    schemes: &[&str],
) -> Result<Vec<Url>, ConfigError> {
    split_list(name, value)?
        .into_iter()
        .map(|entry| {
            let parsed = Url::parse(entry).map_err(|_| ConfigError::InvalidValue(name))?;
            if !schemes.contains(&parsed.scheme()) || parsed.path().trim().is_empty() {
                return Err(ConfigError::InvalidValue(name));
            }
            Ok(parsed)
        })
        .collect()
}

fn split_list<'a>(name: &'static str, value: &'a str) -> Result<Vec<&'a str>, ConfigError> {
    let entries: Vec<_> = value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if entries.len() > MAX_LIST_ENTRIES {
        return Err(ConfigError::TooManyEntries(name));
    }
    Ok(entries)
}

fn validate_database_url(value: &str) -> Result<String, ConfigError> {
    let parsed = Url::parse(value.trim()).map_err(|_| ConfigError::InvalidValue("DATABASE_URL"))?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let neon_host = host == "neon.tech" || host.ends_with(".neon.tech");
    let tls_required = parsed.query_pairs().any(|(key, value)| {
        key == "sslmode" && matches!(value.as_ref(), "require" | "verify-ca" | "verify-full")
    });
    if !matches!(parsed.scheme(), "postgres" | "postgresql")
        || !neon_host
        || parsed.username().is_empty()
        || parsed.password().is_none_or(str::is_empty)
        || parsed.path().trim_matches('/').is_empty()
        || parsed.fragment().is_some()
        || !tls_required
    {
        return Err(ConfigError::InvalidValue("DATABASE_URL"));
    }
    Ok(value.trim().to_owned())
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{0} has an invalid value")]
    InvalidValue(&'static str),
    #[error("{0} contains too many entries")]
    TooManyEntries(&'static str),
    #[error("Sanser version must be {expected}, got {actual}")]
    Version {
        expected: &'static str,
        actual: String,
    },
    #[error("protocol version must be {expected}, got {actual}")]
    ProtocolVersion { expected: u8, actual: u8 },
    #[error("TURN_USERNAME and TURN_CREDENTIAL must either both be set or both be empty")]
    IncompleteTurnCredentials,
    #[error("relay network mode requires at least one TURN URL")]
    RelayRequiresTurn,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn base() -> Vec<(&'static str, &'static str)> {
        vec![
            ("SANSER_VERSION", "2.0.4"),
            ("SANSER_PROTOCOL_VERSION", "2"),
            (
                "DATABASE_URL",
                "postgresql://user:secret@ep-example-pooler.us-east-1.aws.neon.tech/sanser?sslmode=require",
            ),
        ]
    }

    #[test]
    fn parses_defaults_without_logging_database_password() {
        let config = AppConfig::from_values(base())
            .unwrap_or_else(|error| panic!("config parse failed: {error}"));
        assert_eq!(config.network.mode, NetworkMode::Auto);
        assert_eq!(config.server.port, 5_174);
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("secret"));
        assert!(rendered.contains("[REDACTED]"));
    }

    #[test]
    fn parses_network_mode_correctly() {
        let mut values = base();
        values.push(("NETWORK_MODE", "directonly"));
        let config = AppConfig::from_values(values).unwrap();
        assert_eq!(config.network.mode, NetworkMode::DirectOnly);

        let mut values = base();
        values.push(("NETWORK_MODE", "manual"));
        let config = AppConfig::from_values(values).unwrap();
        assert_eq!(config.network.mode, NetworkMode::Manual);

        let mut values = base();
        values.push(("NETWORK_MODE", "relay"));
        let config = AppConfig::from_values(values).unwrap();
        assert_eq!(config.network.mode, NetworkMode::Auto); // Fallback to Auto
    }

    #[test]
    fn incomplete_turn_credentials() {
        let mut values = base();
        values.extend([
            ("TURN_URLS", "turn:relay.example.com:3478"),
            ("TURN_USERNAME", "temporary"),
        ]);
        assert_eq!(
            AppConfig::from_values(values),
            Err(ConfigError::IncompleteTurnCredentials)
        );
    }

    #[test]
    fn rejects_legacy_network_values() {
        let mut values = base();
        values.push(("NETWORK_MODE", "tailscale"));
        assert_eq!(
            AppConfig::from_values(values),
            Err(ConfigError::InvalidValue("NETWORK_MODE"))
        );
    }

    #[test]
    fn requires_a_tls_neon_database() {
        let missing = base().into_iter().filter(|(key, _)| *key != "DATABASE_URL");
        assert_eq!(
            AppConfig::from_values(missing),
            Err(ConfigError::Missing("DATABASE_URL"))
        );

        let mut non_neon = base();
        non_neon.retain(|(key, _)| *key != "DATABASE_URL");
        non_neon.push((
            "DATABASE_URL",
            "postgresql://user:secret@db.example.com/sanser?sslmode=require",
        ));
        assert_eq!(
            AppConfig::from_values(non_neon),
            Err(ConfigError::InvalidValue("DATABASE_URL"))
        );

        let mut no_tls = base();
        no_tls.retain(|(key, _)| *key != "DATABASE_URL");
        no_tls.push((
            "DATABASE_URL",
            "postgresql://user:secret@ep-example.us-east-1.aws.neon.tech/sanser",
        ));
        assert_eq!(
            AppConfig::from_values(no_tls),
            Err(ConfigError::InvalidValue("DATABASE_URL"))
        );
    }
}
