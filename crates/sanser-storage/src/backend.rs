use async_trait::async_trait;
use serde_json::Value;
use sqlx::{
    PgPool, Row, SqlitePool,
    postgres::PgPoolOptions,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fmt, path::PathBuf, str::FromStr, time::Duration};
use thiserror::Error;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DatabaseTarget {
    SqliteMemory,
    SqliteFile(PathBuf),
    Postgres(SecretConnectionString),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageOptions {
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}

impl Default for StorageOptions {
    fn default() -> Self {
        Self {
            max_connections: 8,
            acquire_timeout: Duration::from_secs(5),
        }
    }
}

impl StorageOptions {
    fn validate(self) -> Result<Self, StorageError> {
        if self.max_connections == 0 || self.max_connections > 128 {
            return Err(StorageError::InvalidPoolSize(self.max_connections));
        }
        if self.acquire_timeout.is_zero() || self.acquire_timeout > Duration::from_secs(60) {
            return Err(StorageError::InvalidAcquireTimeout);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageBackendKind {
    Sqlite,
    Postgres,
}

#[derive(Clone, Debug)]
enum Backend {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

#[derive(Clone, Debug)]
pub struct Storage {
    backend: Backend,
}

impl Storage {
    pub async fn connect(
        target: DatabaseTarget,
        options: StorageOptions,
    ) -> Result<Self, StorageError> {
        let options = options.validate()?;
        let backend = match target {
            DatabaseTarget::SqliteMemory => {
                let connection = SqliteConnectOptions::from_str("sqlite::memory:")
                    .map_err(|_| StorageError::InvalidConnection)?
                    .foreign_keys(true);
                Backend::Sqlite(
                    SqlitePoolOptions::new()
                        // A SQLite in-memory database is per connection.
                        .max_connections(1)
                        .acquire_timeout(options.acquire_timeout)
                        .connect_with(connection)
                        .await?,
                )
            }
            DatabaseTarget::SqliteFile(path) => {
                if path.as_os_str().is_empty() {
                    return Err(StorageError::InvalidConnection);
                }
                let connection = SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .foreign_keys(true);
                Backend::Sqlite(
                    SqlitePoolOptions::new()
                        .max_connections(options.max_connections)
                        .acquire_timeout(options.acquire_timeout)
                        .connect_with(connection)
                        .await?,
                )
            }
            DatabaseTarget::Postgres(connection) => {
                let parsed = url::Url::parse(connection.expose())
                    .map_err(|_| StorageError::InvalidConnection)?;
                if !matches!(parsed.scheme(), "postgres" | "postgresql") {
                    return Err(StorageError::InvalidConnection);
                }
                Backend::Postgres(
                    PgPoolOptions::new()
                        .max_connections(options.max_connections)
                        .acquire_timeout(options.acquire_timeout)
                        .connect(connection.expose())
                        .await?,
                )
            }
        };
        Ok(Self { backend })
    }

    pub async fn initialize(&self) -> Result<(), StorageError> {
        match &self.backend {
            Backend::Sqlite(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS app_preferences (\
                     key TEXT PRIMARY KEY NOT NULL CHECK(length(key) <= 128), \
                     value_json TEXT NOT NULL CHECK(length(value_json) <= 262144), \
                     updated_at INTEGER NOT NULL)",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS app_metadata (\
                     key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
                )
                .execute(pool)
                .await?;
            }
            Backend::Postgres(pool) => {
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS app_preferences (\
                     key VARCHAR(128) PRIMARY KEY NOT NULL, \
                     value_json TEXT NOT NULL, updated_at BIGINT NOT NULL)",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE TABLE IF NOT EXISTS app_metadata (\
                     key VARCHAR(128) PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
                )
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn health_check(&self) -> Result<(), StorageError> {
        match &self.backend {
            Backend::Sqlite(pool) => {
                let _value: i64 = sqlx::query_scalar("SELECT 1").fetch_one(pool).await?;
            }
            Backend::Postgres(pool) => {
                let _value: i32 = sqlx::query_scalar("SELECT 1").fetch_one(pool).await?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn kind(&self) -> StorageBackendKind {
        match &self.backend {
            Backend::Sqlite(_) => StorageBackendKind::Sqlite,
            Backend::Postgres(_) => StorageBackendKind::Postgres,
        }
    }

    pub(crate) fn sqlite_pool(&self) -> Result<&SqlitePool, StorageError> {
        match &self.backend {
            Backend::Sqlite(pool) => Ok(pool),
            Backend::Postgres(_) => Err(StorageError::RequiresSqlite),
        }
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
        match &self.backend {
            Backend::Sqlite(pool) => {
                sqlx::query(
                    "INSERT INTO app_preferences(key, value_json, updated_at) VALUES(?1, ?2, ?3) \
                     ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, \
                     updated_at=excluded.updated_at",
                )
                .bind(key)
                .bind(serialized)
                .bind(updated_at)
                .execute(pool)
                .await?;
            }
            Backend::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO app_preferences(key, value_json, updated_at) VALUES($1, $2, $3) \
                     ON CONFLICT(key) DO UPDATE SET value_json=EXCLUDED.value_json, \
                     updated_at=EXCLUDED.updated_at",
                )
                .bind(key)
                .bind(serialized)
                .bind(updated_at)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    async fn preference(&self, key: &str) -> Result<Option<Value>, StorageError> {
        validate_key(key)?;
        let serialized: Option<String> = match &self.backend {
            Backend::Sqlite(pool) => {
                sqlx::query("SELECT value_json FROM app_preferences WHERE key=?1")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?
                    .map(|row| row.get(0))
            }
            Backend::Postgres(pool) => {
                sqlx::query("SELECT value_json FROM app_preferences WHERE key=$1")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?
                    .map(|row| row.get(0))
            }
        };
        serialized
            .map(|value| serde_json::from_str(&value).map_err(|_| StorageError::CorruptValue))
            .transpose()
    }

    async fn delete_preference(&self, key: &str) -> Result<bool, StorageError> {
        validate_key(key)?;
        let affected = match &self.backend {
            Backend::Sqlite(pool) => sqlx::query("DELETE FROM app_preferences WHERE key=?1")
                .bind(key)
                .execute(pool)
                .await?
                .rows_affected(),
            Backend::Postgres(pool) => sqlx::query("DELETE FROM app_preferences WHERE key=$1")
                .bind(key)
                .execute(pool)
                .await?
                .rows_affected(),
        };
        Ok(affected != 0)
    }
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
    #[error("storage connection string is invalid")]
    InvalidConnection,
    #[error("storage pool size {0} is outside 1..=128")]
    InvalidPoolSize(u32),
    #[error("storage acquire timeout is outside 1ms..=60s")]
    InvalidAcquireTimeout,
    #[error("storage operation requires the SQLite local backend")]
    RequiresSqlite,
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
    use serde_json::json;

    #[tokio::test]
    async fn sqlite_preferences_round_trip_through_shared_abstraction() {
        let storage = Storage::connect(DatabaseTarget::SqliteMemory, StorageOptions::default())
            .await
            .unwrap_or_else(|error| panic!("connect failed: {error}"));
        storage
            .initialize()
            .await
            .unwrap_or_else(|error| panic!("initialize failed: {error}"));
        storage
            .set_preference("network_mode", &json!("auto"), 1)
            .await
            .unwrap_or_else(|error| panic!("write failed: {error}"));
        assert_eq!(
            storage
                .preference("network_mode")
                .await
                .unwrap_or_else(|error| panic!("read failed: {error}")),
            Some(json!("auto"))
        );
        assert!(storage.health_check().await.is_ok());
    }
}
