use std::{env, net::IpAddr, str::FromStr, sync::Arc, time::Duration};

use base64::{Engine, engine::general_purpose::STANDARD};
use http::HeaderValue;
pub use sanser_core::NetworkMode;
use thiserror::Error;
use zeroize::Zeroizing;

pub const SANSER_VERSION: &str = sanser_core::VERSION;
pub const PROTOCOL_VERSION: u8 = sanser_core::PROTOCOL_VERSION;

#[derive(Clone)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub database_url: String,
    pub database_max_connections: u32,
    pub database_min_connections: u32,
    pub database_acquire_timeout: Duration,
    pub allowed_origins: Vec<HeaderValue>,
    pub network_mode: NetworkMode,
    pub stun_urls: Vec<String>,
    pub turn_urls: Vec<String>,
    pub turn_username: Option<String>,
    pub turn_credential: Option<String>,
    pub turn_shared_secret: Option<String>,
    pub turn_credential_ttl: Duration,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    pub request_timeout: Duration,
    pub body_limit_bytes: usize,
    pub general_rate_limit_per_minute: u32,
    pub login_rate_limit_per_ten_minutes: u32,
    pub device_offline_after: Duration,
    pub pending_session_ttl: Duration,
    pub native_base_port: u16,
    pub session_credential_ttl: Duration,
    pub(crate) session_credential_key: Arc<Zeroizing<Vec<u8>>>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid Sanser server configuration: {0}")]
    Invalid(String),
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        if let Ok(version) = env::var("SANSER_VERSION") {
            if version.trim() != SANSER_VERSION {
                return Err(ConfigError::Invalid(format!(
                    "SANSER_VERSION must be {SANSER_VERSION}"
                )));
            }
        }
        if let Ok(protocol) = env::var("SANSER_PROTOCOL_VERSION") {
            if protocol.trim() != PROTOCOL_VERSION.to_string() {
                return Err(ConfigError::Invalid(format!(
                    "SANSER_PROTOCOL_VERSION must be {PROTOCOL_VERSION}"
                )));
            }
        }

        let host = parse_env("SERVER_HOST", "127.0.0.1")?;
        // Cloud platforms (Render, Railway, etc.) inject PORT. Prefer
        // SERVER_PORT for explicitness, fall back to PORT, then default.
        let port: u16 = optional_env("SERVER_PORT")
            .or_else(|| optional_env("PORT"))
            .unwrap_or_else(|| "5174".to_owned())
            .parse()
            .map_err(|_| ConfigError::Invalid("SERVER_PORT / PORT has an invalid value".into()))?;
        let database_url = required_env("DATABASE_URL")?;
        validate_neon_database_url(&database_url, false)?;
        let session_credential_key = session_credential_key_from_env()?;
        let allowed_origins = parse_origins(&env_or(
            "ALLOWED_ORIGINS",
            "http://127.0.0.1:5174,http://localhost:5174,tauri://localhost,http://tauri.localhost",
        ))?;
        let network_mode = parse_network_mode(&env_or("NETWORK_MODE", "auto"))?;
        let stun_urls = parse_urls(
            "STUN_URLS",
            &env_or("STUN_URLS", "stun:stun.l.google.com:19302"),
            &["stun:", "stuns:"],
        )?;
        let turn_urls = parse_urls("TURN_URLS", &env_or("TURN_URLS", ""), &["turn:", "turns:"])?;
        let turn_username = optional_env("TURN_USERNAME");
        let turn_credential = optional_env("TURN_CREDENTIAL");
        let turn_shared_secret = optional_env("TURN_SHARED_SECRET");

        if turn_username.is_some() != turn_credential.is_some() {
            return Err(ConfigError::Invalid(
                "TURN_USERNAME and TURN_CREDENTIAL must be configured together".into(),
            ));
        }
        if !turn_urls.is_empty()
            && turn_shared_secret.is_none()
            && (turn_username.is_none() || turn_credential.is_none())
        {
            return Err(ConfigError::Invalid(
                "TURN_URLS requires TURN_SHARED_SECRET or TURN_USERNAME/TURN_CREDENTIAL".into(),
            ));
        }
        // Relay checks removed as TURN is deprecated.

        let config = Self {
            host,
            port,
            database_url,
            database_max_connections: parse_env("DATABASE_MAX_CONNECTIONS", "10")?,
            database_min_connections: parse_env("DATABASE_MIN_CONNECTIONS", "1")?,
            database_acquire_timeout: Duration::from_secs(parse_env(
                "DATABASE_ACQUIRE_TIMEOUT_SECONDS",
                "10",
            )?),
            allowed_origins,
            network_mode,
            stun_urls,
            turn_urls,
            turn_username,
            turn_credential,
            turn_shared_secret,
            turn_credential_ttl: Duration::from_secs(parse_env(
                "TURN_CREDENTIAL_TTL_SECONDS",
                "3600",
            )?),
            access_token_ttl: Duration::from_secs(parse_env("ACCESS_TOKEN_TTL_SECONDS", "900")?),
            refresh_token_ttl: Duration::from_secs(parse_env(
                "REFRESH_TOKEN_TTL_SECONDS",
                "2592000",
            )?),
            request_timeout: Duration::from_secs(parse_env("REQUEST_TIMEOUT_SECONDS", "15")?),
            body_limit_bytes: parse_env("REQUEST_BODY_LIMIT_BYTES", "262144")?,
            general_rate_limit_per_minute: parse_env("RATE_LIMIT_REQUESTS_PER_MINUTE", "600")?,
            login_rate_limit_per_ten_minutes: parse_env("LOGIN_RATE_LIMIT_PER_TEN_MINUTES", "8")?,
            device_offline_after: Duration::from_secs(parse_env(
                "DEVICE_OFFLINE_AFTER_SECONDS",
                "20",
            )?),
            pending_session_ttl: Duration::from_secs(parse_env(
                "PENDING_SESSION_TTL_SECONDS",
                "30",
            )?),
            native_base_port: parse_env("NATIVE_BASE_PORT", "50000")?,
            session_credential_ttl: Duration::from_secs(parse_env(
                "SESSION_CREDENTIAL_TTL_SECONDS",
                "60",
            )?),
            session_credential_key,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn test(database_url: String) -> Result<Self, ConfigError> {
        validate_neon_database_url(&database_url, true)?;
        let config = Self {
            host: "127.0.0.1".parse().expect("valid loopback address"),
            port: 0,
            database_url,
            database_max_connections: 4,
            database_min_connections: 0,
            database_acquire_timeout: Duration::from_secs(10),
            allowed_origins: vec![HeaderValue::from_static("http://127.0.0.1:5174")],
            network_mode: NetworkMode::Auto,
            stun_urls: vec!["stun:stun.example.test:3478".into()],
            turn_urls: Vec::new(),
            turn_username: None,
            turn_credential: None,
            turn_shared_secret: None,
            turn_credential_ttl: Duration::from_secs(3600),
            access_token_ttl: Duration::from_secs(900),
            refresh_token_ttl: Duration::from_secs(2592000),
            request_timeout: Duration::from_secs(15),
            body_limit_bytes: 262_144,
            general_rate_limit_per_minute: 10_000,
            login_rate_limit_per_ten_minutes: 8,
            device_offline_after: Duration::from_secs(20),
            pending_session_ttl: Duration::from_secs(30),
            native_base_port: 50_000,
            session_credential_ttl: Duration::from_secs(60),
            session_credential_key: Arc::new(Zeroizing::new(vec![0xA5; 32])),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=100).contains(&self.database_max_connections)
            || self.database_min_connections > self.database_max_connections
        {
            return Err(ConfigError::Invalid(
                "DATABASE_MAX_CONNECTIONS must be 1–100 and DATABASE_MIN_CONNECTIONS may not exceed it"
                    .into(),
            ));
        }
        if !(1..=60).contains(&self.database_acquire_timeout.as_secs()) {
            return Err(ConfigError::Invalid(
                "DATABASE_ACQUIRE_TIMEOUT_SECONDS must be between 1 and 60".into(),
            ));
        }
        if !(60..=86_400).contains(&self.access_token_ttl.as_secs()) {
            return Err(ConfigError::Invalid(
                "ACCESS_TOKEN_TTL_SECONDS must be between 60 and 86400".into(),
            ));
        }
        if self.refresh_token_ttl <= self.access_token_ttl
            || self.refresh_token_ttl > Duration::from_secs(365 * 24 * 60 * 60)
        {
            return Err(ConfigError::Invalid(
                "REFRESH_TOKEN_TTL_SECONDS must exceed the access TTL and be at most one year"
                    .into(),
            ));
        }
        if !(4_096..=2_097_152).contains(&self.body_limit_bytes) {
            return Err(ConfigError::Invalid(
                "REQUEST_BODY_LIMIT_BYTES must be between 4096 and 2097152".into(),
            ));
        }
        if self.allowed_origins.is_empty() {
            return Err(ConfigError::Invalid(
                "ALLOWED_ORIGINS must contain at least one explicit origin".into(),
            ));
        }
        if self.general_rate_limit_per_minute == 0 || self.login_rate_limit_per_ten_minutes == 0 {
            return Err(ConfigError::Invalid(
                "rate limits must be greater than zero".into(),
            ));
        }
        if !(60..=86_400).contains(&self.turn_credential_ttl.as_secs()) {
            return Err(ConfigError::Invalid(
                "TURN_CREDENTIAL_TTL_SECONDS must be between 60 and 86400".into(),
            ));
        }
        if !(1_024..=65_533).contains(&self.native_base_port) {
            return Err(ConfigError::Invalid(
                "NATIVE_BASE_PORT must be between 1024 and 65533".into(),
            ));
        }
        if !(15..=300).contains(&self.session_credential_ttl.as_secs()) {
            return Err(ConfigError::Invalid(
                "SESSION_CREDENTIAL_TTL_SECONDS must be between 15 and 300".into(),
            ));
        }
        Ok(())
    }
}

fn parse_network_mode(value: &str) -> Result<NetworkMode, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(NetworkMode::Auto),
        "direct" | "directonly" => Ok(NetworkMode::DirectOnly),
        "manual" => Ok(NetworkMode::Manual),
        "relay" => Ok(NetworkMode::Relay),
        _ => Err(ConfigError::Invalid(
            "NETWORK_MODE must be one of: auto, direct, relay, manual".into(),
        )),
    }
}

fn validate_neon_database_url(
    database_url: &str,
    allow_test_schema_option: bool,
) -> Result<(), ConfigError> {
    let parsed = url::Url::parse(database_url)
        .map_err(|_| ConfigError::Invalid("DATABASE_URL is not a valid URL".into()))?;
    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return Err(ConfigError::Invalid(
            "DATABASE_URL must use postgresql:// or postgres://".into(),
        ));
    }

    let host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ConfigError::Invalid("DATABASE_URL must include a Neon host".into()))?;
    if !host.ends_with(".neon.tech") {
        return Err(ConfigError::Invalid(
            "DATABASE_URL host must be a Neon endpoint ending in .neon.tech".into(),
        ));
    }
    if parsed.username().is_empty() {
        return Err(ConfigError::Invalid(
            "DATABASE_URL must include a PostgreSQL user".into(),
        ));
    }
    if parsed.password().is_none_or(str::is_empty) {
        return Err(ConfigError::Invalid(
            "DATABASE_URL must include a PostgreSQL password".into(),
        ));
    }
    if parsed.fragment().is_some() {
        return Err(ConfigError::Invalid(
            "DATABASE_URL may not contain a fragment".into(),
        ));
    }
    if parsed.path().trim_matches('/').is_empty() || parsed.path().trim_matches('/').contains('/') {
        return Err(ConfigError::Invalid(
            "DATABASE_URL must include exactly one database name".into(),
        ));
    }

    let query_pairs = parsed.query_pairs().collect::<Vec<_>>();
    let ssl_modes = query_pairs
        .iter()
        .filter(|(key, _)| key == "sslmode")
        .map(|(_, value)| value.as_ref())
        .collect::<Vec<_>>();
    if ssl_modes.len() != 1 || ssl_modes[0] != "require" {
        return Err(ConfigError::Invalid(
            "DATABASE_URL must contain sslmode=require for Neon TLS".into(),
        ));
    }
    for (key, value) in query_pairs {
        if key == "sslmode" {
            continue;
        }
        if key == "channel_binding" && value == "require" {
            continue;
        }
        if allow_test_schema_option
            && key == "options"
            && value
                .strip_prefix("-csearch_path=sanser_test_")
                .is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        {
            continue;
        }
        return Err(ConfigError::Invalid(
            "DATABASE_URL contains an unsupported connection parameter".into(),
        ));
    }
    Ok(())
}

fn parse_origins(value: &str) -> Result<Vec<HeaderValue>, ConfigError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(|origin| {
            if origin == "*" {
                return Err(ConfigError::Invalid(
                    "ALLOWED_ORIGINS may not contain a wildcard".into(),
                ));
            }
            let parsed = url::Url::parse(origin).map_err(|_| {
                ConfigError::Invalid(format!(
                    "ALLOWED_ORIGINS contains an invalid origin: {origin}"
                ))
            })?;
            let host = parsed.host_str().unwrap_or_default();
            let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
            let tauri_origin = (parsed.scheme() == "tauri" && host == "localhost")
                || (parsed.scheme() == "http" && host == "tauri.localhost");
            if parsed.username() != ""
                || parsed.password().is_some()
                || !matches!(parsed.path(), "" | "/")
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !(parsed.scheme() == "https"
                    || (parsed.scheme() == "http" && loopback)
                    || tauri_origin)
            {
                return Err(ConfigError::Invalid(format!(
                    "ALLOWED_ORIGINS must contain only HTTPS, loopback development, or Tauri origins: {origin}"
                )));
            }
            origin.parse::<HeaderValue>().map_err(|_| {
                ConfigError::Invalid(format!(
                    "ALLOWED_ORIGINS contains an invalid origin: {origin}"
                ))
            })
        })
        .collect()
}

fn parse_urls(name: &str, value: &str, schemes: &[&str]) -> Result<Vec<String>, ConfigError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(|url| {
            if schemes.iter().any(|scheme| url.starts_with(scheme)) {
                Ok(url.to_owned())
            } else {
                Err(ConfigError::Invalid(format!(
                    "{name} contains an unsupported URL: {url}"
                )))
            }
        })
        .collect()
}

fn env_or(name: &str, default: &str) -> String {
    optional_env(name).unwrap_or_else(|| default.to_owned())
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn required_env(name: &str) -> Result<String, ConfigError> {
    optional_env(name).ok_or_else(|| ConfigError::Invalid(format!("{name} is required")))
}

fn session_credential_key_from_env() -> Result<Arc<Zeroizing<Vec<u8>>>, ConfigError> {
    let encoded = env::var("SESSION_CREDENTIAL_KEY")
        .map(Zeroizing::new)
        .map_err(|_| ConfigError::Invalid("SESSION_CREDENTIAL_KEY is required".into()))?;
    decode_session_credential_key(encoded.trim())
}

fn decode_session_credential_key(encoded: &str) -> Result<Arc<Zeroizing<Vec<u8>>>, ConfigError> {
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| ConfigError::Invalid("SESSION_CREDENTIAL_KEY must be valid base64".into()))?;
    if !(32..=128).contains(&decoded.len()) {
        return Err(ConfigError::Invalid(
            "SESSION_CREDENTIAL_KEY must decode to 32–128 bytes".into(),
        ));
    }
    Ok(Arc::new(Zeroizing::new(decoded)))
}

fn parse_env<T>(name: &str, default: &str) -> Result<T, ConfigError>
where
    T: FromStr,
{
    env_or(name, default)
        .parse()
        .map_err(|_| ConfigError::Invalid(format!("{name} has an invalid value")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_is_neon_postgres_with_required_tls() {
        assert!(validate_neon_database_url("sqlite://./data/sanser.db", false).is_err());
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@localhost:5432/sanser?sslmode=require",
                false,
            )
            .is_err()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user@ep-test.ap-southeast-1.aws.neon.tech/sanser?sslmode=require",
                false,
            )
            .is_err()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@ep-test.ap-southeast-1.aws.neon.tech/sanser?SSLMODE=require",
                false,
            )
            .is_err()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@ep-test.ap-southeast-1.aws.neon.tech/sanser?sslmode=require&sslmode=require",
                false,
            )
            .is_err()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@ep-test.ap-southeast-1.aws.neon.tech/sanser",
                false,
            )
            .is_err()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@ep-test.ap-southeast-1.aws.neon.tech/sanser?sslmode=disable",
                false,
            )
            .is_err()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@ep-test-pooler.ap-southeast-1.aws.neon.tech/sanser?sslmode=require&channel_binding=require",
                false,
            )
            .is_ok()
        );
        assert!(
            validate_neon_database_url(
                "postgresql://user:pass@ep-test-pooler.ap-southeast-1.aws.neon.tech/sanser?sslmode=require&unknown=value",
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn native_session_key_is_base64_and_at_least_256_bits() {
        assert!(decode_session_credential_key("not base64").is_err());
        assert!(decode_session_credential_key(&STANDARD.encode([7_u8; 31])).is_err());
        let key = decode_session_credential_key(&STANDARD.encode([7_u8; 32]))
            .expect("valid 256-bit session credential key");
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn cors_origins_are_exact_and_secure() {
        assert!(
            parse_origins(
                "https://app.example.com,tauri://localhost,http://tauri.localhost,http://127.0.0.1:1420"
            )
            .is_ok()
        );
        assert!(parse_origins("*").is_err());
        assert!(parse_origins("http://app.example.com").is_err());
        assert!(parse_origins("https://app.example.com/path").is_err());
        assert!(parse_origins("https://user:secret@app.example.com").is_err());
    }
}
