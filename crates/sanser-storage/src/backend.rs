use async_trait::async_trait;
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{fmt, time::Duration};
use thiserror::Error;
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop};

const MAX_PREFERENCE_KEY_BYTES: usize = 128;
const MAX_PREFERENCE_VALUE_BYTES: usize = 256 * 1_024;

#[derive(Clone, Eq, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct SecretConnectionString(String);

impl SecretConnectionString {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretConnectionString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretConnectionString([REDACTED])")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum DatabaseTarget {
    NeonPostgres(SecretConnectionString),
}

impl fmt::Debug for DatabaseTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DatabaseTarget::NeonPostgres([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageOptions {
    pub min_connections: u32,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}

impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            min_connections: 1,
            max_connections: 10,
            acquire_timeout: Duration::from_secs(10),
        }
    }
}

impl StorageOptions {
    fn validate(self) -> Result<Self, StorageError> {
        if self.max_connections == 0
            || self.max_connections > 64
            || self.min_connections > self.max_connections
        {
            return Err(StorageError::InvalidPoolSize {
                min: self.min_connections,
                max: self.max_connections,
            });
        }
        if self.acquire_timeout.is_zero() || self.acquire_timeout > Duration::from_secs(60) {
            return Err(StorageError::InvalidAcquireTimeout);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageBackendKind {
    NeonPostgres,
}

#[derive(Clone)]
pub struct Storage {
    pool: PgPool,
}

impl fmt::Debug for Storage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Storage(NeonPostgres)")
    }
}

impl Storage {
    /// Connects to a TLS-enabled Neon `PostgreSQL` database with a bounded pool.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the URL, pool settings, or connection fails.
    pub async fn connect(
        target: DatabaseTarget,
        options: StorageOptions,
    ) -> Result<Self, StorageError> {
        let options = options.validate()?;
        let DatabaseTarget::NeonPostgres(connection) = target;
        validate_neon_connection(connection.expose())?;
        let pool = PgPoolOptions::new()
            .min_connections(options.min_connections)
            .max_connections(options.max_connections)
            .acquire_timeout(options.acquire_timeout)
            .idle_timeout(Some(Duration::from_secs(300)))
            .max_lifetime(Some(Duration::from_secs(1_800)))
            .connect(connection.expose())
            .await?;
        Ok(Self { pool })
    }

    /// Creates the preference and migration metadata tables when absent.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when Neon rejects a schema operation.
    pub async fn initialize(&self) -> Result<(), StorageError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_preferences (\
             key VARCHAR(128) PRIMARY KEY NOT NULL, \
             value_json TEXT NOT NULL, updated_at BIGINT NOT NULL)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_metadata (\
             key VARCHAR(128) PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Verifies that a pooled Neon connection can execute a lightweight query.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when a connection cannot be acquired or queried.
    pub async fn health_check(&self) -> Result<(), StorageError> {
        let _: i32 = sqlx::query_scalar("SELECT 1").fetch_one(&self.pool).await?;
        Ok(())
    }

    #[must_use]
    pub const fn kind(&self) -> StorageBackendKind {
        StorageBackendKind::NeonPostgres
    }

    pub(crate) const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
pub trait PreferenceStore: Send + Sync {
    async fn set_preference(
        &self,
        key: &str,
        value: &Value,
        updated_at: u64,
    ) -> Result<(), StorageError>;
    async fn preference(&self, key: &str) -> Result<Option<Value>, StorageError>;
    async fn delete_preference(&self, key: &str) -> Result<bool, StorageError>;
}

#[async_trait]
impl PreferenceStore for Storage {
    async fn set_preference(
        &self,
        key: &str,
        value: &Value,
        updated_at: u64,
    ) -> Result<(), StorageError> {
        validate_key(key)?;
        let serialized = serde_json::to_string(value).map_err(|_| StorageError::Serialization)?;
        if serialized.len() > MAX_PREFERENCE_VALUE_BYTES {
            return Err(StorageError::ValueTooLarge(serialized.len()));
        }
        let updated_at = i64::try_from(updated_at).map_err(|_| StorageError::TimestampOverflow)?;
        sqlx::query(
            "INSERT INTO app_preferences(key, value_json, updated_at) VALUES($1, $2, $3) \
             ON CONFLICT(key) DO UPDATE SET value_json=EXCLUDED.value_json, \
             updated_at=EXCLUDED.updated_at",
        )
        .bind(key)
        .bind(serialized)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn preference(&self, key: &str) -> Result<Option<Value>, StorageError> {
        validate_key(key)?;
        let serialized = sqlx::query("SELECT value_json FROM app_preferences WHERE key=$1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| row.get::<String, _>(0));
        serialized
            .map(|value| serde_json::from_str(&value).map_err(|_| StorageError::CorruptValue))
            .transpose()
    }

    async fn delete_preference(&self, key: &str) -> Result<bool, StorageError> {
        validate_key(key)?;
        let affected = sqlx::query("DELETE FROM app_preferences WHERE key=$1")
            .bind(key)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(affected != 0)
    }
}

fn validate_neon_connection(value: &str) -> Result<(), StorageError> {
    let parsed = Url::parse(value).map_err(|_| StorageError::InvalidConnection)?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let neon_host = host == "neon.tech" || host.ends_with(".neon.tech");
    let tls = parsed.query_pairs().any(|(key, value)| {
        key == "sslmode" && matches!(value.as_ref(), "require" | "verify-ca" | "verify-full")
    });
    if !matches!(parsed.scheme(), "postgres" | "postgresql")
        || !neon_host
        || parsed.username().is_empty()
        || parsed.password().is_none_or(str::is_empty)
        || parsed.path().trim_matches('/').is_empty()
        || parsed.fragment().is_some()
        || !tls
    {
        return Err(StorageError::InvalidConnection);
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), StorageError> {
    if key.is_empty()
        || key.len() > MAX_PREFERENCE_KEY_BYTES
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(StorageError::InvalidKey);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Neon PostgreSQL connection string is invalid or TLS is not required")]
    InvalidConnection,
    #[error("storage pool bounds min={min}, max={max} are invalid")]
    InvalidPoolSize { min: u32, max: u32 },
    #[error("storage acquire timeout is outside 1ms..=60s")]
    InvalidAcquireTimeout,
    #[error("preference key is invalid")]
    InvalidKey,
    #[error("preference value has {0} bytes, exceeding 262144")]
    ValueTooLarge(usize),
    #[error("preference timestamp exceeds the database representation")]
    TimestampOverflow,
    #[error("preference serialization failed")]
    Serialization,
    #[error("stored preference is corrupt")]
    CorruptValue,
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_tls_neon_connections() {
        assert!(
            validate_neon_connection(
                "postgresql://user:secret@ep-test-pooler.us-east-1.aws.neon.tech/sanser?sslmode=require"
            )
            .is_ok()
        );
        assert!(
            validate_neon_connection(
                "postgresql://user:secret@db.example.com/sanser?sslmode=require"
            )
            .is_err()
        );
        assert!(
            validate_neon_connection(
                "postgresql://user:secret@ep-test.us-east-1.aws.neon.tech/sanser"
            )
            .is_err()
        );
    }

    #[test]
    fn pool_limits_are_bounded() {
        assert!(StorageOptions::default().validate().is_ok());
        assert!(
            StorageOptions {
                min_connections: 2,
                max_connections: 1,
                acquire_timeout: Duration::from_secs(1),
            }
            .validate()
            .is_err()
        );
    }
}
