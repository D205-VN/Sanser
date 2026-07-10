use std::{env, net::IpAddr, path::PathBuf, str::FromStr, time::Duration};

use http::HeaderValue;
pub use sanser_core::NetworkMode;
use thiserror::Error;

pub const SANSER_VERSION: &str = sanser_core::VERSION;
pub const PROTOCOL_VERSION: u8 = sanser_core::PROTOCOL_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageMode {
    Local,
    Shared,
}

#[derive(Clone)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub public_base_url: String,
    pub storage_mode: StorageMode,
    pub database_url: String,
    pub sqlite_path: Option<PathBuf>,
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
        let port = parse_env("SERVER_PORT", "5174")?;
        let public_base_url = env_or("PUBLIC_BASE_URL", "http://127.0.0.1:5174");
        validate_public_url(&public_base_url)?;

        let storage_mode = parse_storage_mode(&env_or("STORAGE_MODE", "local"))?;
        let (database_url, sqlite_path) = database_config(
            storage_mode,
            optional_env("DATABASE_URL"),
            optional_env("SQLITE_PATH"),
        )?;
        let allowed_origins = parse_origins(&env_or(
            "ALLOWED_ORIGINS",
            "http://127.0.0.1:5174,http://localhost:5174,tauri://localhost",
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
        if network_mode == NetworkMode::Relay && turn_urls.is_empty() {
            return Err(ConfigError::Invalid(
                "NETWORK_MODE=relay requires at least one TURN_URLS entry".into(),
            ));
        }

        let config = Self {
            host,
            port,
            public_base_url,
            storage_mode,
            database_url,
            sqlite_path,
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
        };
        config.validate()?;
        Ok(config)
    }

    pub fn test(database_url: String) -> Self {
        Self {
            host: "127.0.0.1".parse().expect("valid loopback address"),
            port: 0,
            public_base_url: "http://127.0.0.1:5174".into(),
            storage_mode: StorageMode::Local,
            database_url,
            sqlite_path: None,
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
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
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
        Ok(())
    }
}

fn parse_network_mode(value: &str) -> Result<NetworkMode, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(NetworkMode::Auto),
        "direct" => Ok(NetworkMode::Direct),
        "relay" => Ok(NetworkMode::Relay),
        _ => Err(ConfigError::Invalid(
            "NETWORK_MODE must be one of: auto, direct, relay".into(),
        )),
    }
}

fn parse_storage_mode(value: &str) -> Result<StorageMode, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "local" => Ok(StorageMode::Local),
        "shared" => Ok(StorageMode::Shared),
        _ => Err(ConfigError::Invalid(
            "STORAGE_MODE must be one of: local, shared".into(),
        )),
    }
}

fn database_config(
    mode: StorageMode,
    database_url: Option<String>,
    sqlite_path: Option<String>,
) -> Result<(String, Option<PathBuf>), ConfigError> {
    match mode {
        StorageMode::Local => {
            // DATABASE_URL is deliberately ignored in local mode. This makes a
            // copied .env.example safe even while it contains a cloud placeholder.
            let path = PathBuf::from(sqlite_path.unwrap_or_else(|| "./data/sanser.db".into()));
            let path_string = path.to_string_lossy().replace('\\', "/");
            if path_string
                .chars()
                .any(|character| matches!(character, '?' | '#' | '\n' | '\r'))
            {
                return Err(ConfigError::Invalid(
                    "SQLITE_PATH may not contain ?, #, or line breaks".into(),
                ));
            }
            Ok((
                format!("sqlite://{path_string}?mode=rwc"),
                Some(path),
            ))
        }
        StorageMode::Shared => {
            let database_url = database_url.ok_or_else(|| {
                ConfigError::Invalid(
                    "STORAGE_MODE=shared requires a PostgreSQL DATABASE_URL".into(),
                )
            })?;
            if !(database_url.starts_with("postgres://")
                || database_url.starts_with("postgresql://"))
            {
                return Err(ConfigError::Invalid(
                    "STORAGE_MODE=shared requires DATABASE_URL to use postgresql:// or postgres://"
                        .into(),
                ));
            }
            database_url
                .parse::<sqlx::postgres::PgConnectOptions>()
                .map_err(|_| {
                    ConfigError::Invalid("DATABASE_URL is not a valid PostgreSQL URL".into())
                })?;
            Ok((database_url, None))
        }
    }
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

fn validate_public_url(value: &str) -> Result<(), ConfigError> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(())
    } else {
        Err(ConfigError::Invalid(
            "PUBLIC_BASE_URL must start with http:// or https://".into(),
        ))
    }
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
    fn local_mode_uses_sqlite_and_ignores_database_url_placeholder() {
        let (url, path) = database_config(
            StorageMode::Local,
            Some("postgresql://USER:PASSWORD@HOST:5432/sanser".into()),
            Some("./data/local.db".into()),
        )
        .expect("local storage configuration");

        assert!(url.starts_with("sqlite://./data/local.db?"));
        assert_eq!(path, Some(PathBuf::from("./data/local.db")));
        assert!(!url.contains("PASSWORD"));
    }

    #[test]
    fn shared_mode_requires_a_valid_postgresql_url() {
        assert!(database_config(StorageMode::Shared, None, None).is_err());
        assert!(
            database_config(
                StorageMode::Shared,
                Some("sqlite://./data/sanser.db".into()),
                None,
            )
            .is_err()
        );
        assert!(database_config(StorageMode::Shared, Some("postgresql://".into()), None,).is_err());

        let (url, path) = database_config(
            StorageMode::Shared,
            Some("postgresql://user:password@localhost:5432/sanser".into()),
            Some("./ignored.db".into()),
        )
        .expect("shared storage configuration");
        assert_eq!(url, "postgresql://user:password@localhost:5432/sanser");
        assert_eq!(path, None);
    }

    #[test]
    fn storage_mode_is_strict() {
        assert_eq!(parse_storage_mode("local").ok(), Some(StorageMode::Local));
        assert_eq!(parse_storage_mode("SHARED").ok(), Some(StorageMode::Shared));
        assert!(parse_storage_mode("auto").is_err());
        assert!(parse_storage_mode("sqlite").is_err());
    }
}
