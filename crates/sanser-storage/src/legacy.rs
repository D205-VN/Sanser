use crate::{Storage, StorageError};
use serde_json::{Map, Value, json};
use sqlx::Acquire;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

pub const LEGACY_MIGRATION_VERSION: u8 = 2;
const MAX_LEGACY_BYTES: u64 = 1_048_576;
const LEGACY_FILES: [&str; 2] = ["store.json", "settings.json"];
const ALLOWED_PREFERENCES: [&str; 15] = [
    "theme",
    "language",
    "auto_start",
    "auto_online",
    "auto_accept",
    "quality_profile",
    "network_mode",
    "audio_enabled",
    "input_enabled",
    "clipboard_enabled",
    "codec",
    "fps",
    "bitrate",
    "resolution",
    "volume",
];

#[derive(Clone, Debug)]
pub struct LegacyMigrator {
    data_directory: PathBuf,
}

impl LegacyMigrator {
    #[must_use]
    pub fn new(data_directory: PathBuf) -> Self {
        Self { data_directory }
    }

    pub fn detect(&self) -> Result<Option<PathBuf>, LegacyMigrationError> {
        for name in LEGACY_FILES {
            let candidate = self.data_directory.join(name);
            if candidate.try_exists()? {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    pub async fn migrate_detected(
        &self,
        storage: &Storage,
        migrated_at: u64,
    ) -> Result<Option<LegacyMigrationReport>, LegacyMigrationError> {
        let Some(path) = self.detect()? else {
            return Ok(None);
        };
        self.migrate_file(storage, &path, migrated_at)
            .await
            .map(Some)
    }

    pub async fn migrate_file(
        &self,
        storage: &Storage,
        source: &Path,
        migrated_at: u64,
    ) -> Result<LegacyMigrationReport, LegacyMigrationError> {
        let source = self.validate_source(source)?;
        let metadata = fs::metadata(&source)?;
        if !metadata.is_file() || metadata.len() > MAX_LEGACY_BYTES {
            return Err(LegacyMigrationError::InvalidSource);
        }
        let raw = fs::read(&source)?;
        let legacy: Value =
            serde_json::from_slice(&raw).map_err(|_| LegacyMigrationError::InvalidJson)?;
        let sanitized = extract_safe_values(&legacy)?;

        // A byte-for-byte backup is durable before the first database write.
        let backup_path = write_backup(&source, &raw)?;
        let pool = storage.sqlite_pool()?;
        storage.initialize().await?;
        let mut transaction = pool.begin().await?;
        for (key, value) in &sanitized.preferences {
            let serialized =
                serde_json::to_string(value).map_err(|_| LegacyMigrationError::InvalidJson)?;
            let migrated_at =
                i64::try_from(migrated_at).map_err(|_| LegacyMigrationError::TimestampOverflow)?;
            sqlx::query(
                "INSERT INTO app_preferences(key, value_json, updated_at) VALUES(?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, \
                 updated_at=excluded.updated_at",
            )
            .bind(key)
            .bind(serialized)
            .bind(migrated_at)
            .execute(&mut *transaction)
            .await?;
        }
        if let Some(server_url) = &sanitized.server_url {
            sqlx::query(
                "INSERT INTO app_metadata(key, value) VALUES('server_url', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )
            .bind(server_url.as_str())
            .execute(&mut *transaction)
            .await?;
        }
        if let Some(device_id) = sanitized.device_id {
            sqlx::query(
                "INSERT INTO app_metadata(key, value) VALUES('device_id', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )
            .bind(device_id.to_string())
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO app_metadata(key, value) VALUES('migration_version', '2') \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(LegacyMigrationReport {
            source,
            backup_path,
            imported_preferences: sanitized.preferences.len(),
            imported_server_url: sanitized.server_url.is_some(),
            imported_device_id: sanitized.device_id.is_some(),
            skipped_sensitive_fields: sanitized.skipped_sensitive_fields,
            migration_version: LEGACY_MIGRATION_VERSION,
        })
    }

    fn validate_source(&self, source: &Path) -> Result<PathBuf, LegacyMigrationError> {
        let root = self.data_directory.canonicalize()?;
        let source = source.canonicalize()?;
        let valid_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| LEGACY_FILES.contains(&name));
        if source.parent() != Some(root.as_path()) || !valid_name {
            return Err(LegacyMigrationError::PathOutsideDataDirectory);
        }
        Ok(source)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyMigrationReport {
    pub source: PathBuf,
    pub backup_path: PathBuf,
    pub imported_preferences: usize,
    pub imported_server_url: bool,
    pub imported_device_id: bool,
    pub skipped_sensitive_fields: usize,
    pub migration_version: u8,
}

#[derive(Debug)]
struct SafeLegacyValues {
    server_url: Option<Url>,
    device_id: Option<Uuid>,
    preferences: Map<String, Value>,
    skipped_sensitive_fields: usize,
}

fn extract_safe_values(legacy: &Value) -> Result<SafeLegacyValues, LegacyMigrationError> {
    let object = legacy
        .as_object()
        .ok_or(LegacyMigrationError::InvalidJson)?;
    let skipped_sensitive_fields = count_sensitive_fields(legacy);
    let server_url =
        find_string(object, &["server_url", "serverUrl"]).and_then(validate_legacy_server_url);
    let device_id = find_string(object, &["device_id", "deviceId"])
        .and_then(|value| Uuid::parse_str(value).ok())
        .filter(|value| !value.is_nil());

    let preference_source = object
        .get("preferences")
        .and_then(Value::as_object)
        .unwrap_or(object);
    let mut preferences = Map::new();
    for key in ALLOWED_PREFERENCES {
        let camel = snake_to_camel(key);
        let Some(value) = preference_source
            .get(key)
            .or_else(|| preference_source.get(&camel))
        else {
            continue;
        };
        if !is_safe_preference_value(value) {
            continue;
        }
        let mapped = match key {
            "network_mode" if contains_legacy_network(value) => json!("auto"),
            "quality_profile" if contains_legacy_network(value) => json!("balanced"),
            _ => value.clone(),
        };
        preferences.insert(key.to_owned(), mapped);
    }

    // Old VPN-specific keys are mapped, never retained.
    if object
        .keys()
        .any(|key| normalize_key(key).contains("tailscale"))
    {
        preferences.insert("network_mode".to_owned(), json!("auto"));
        preferences
            .entry("quality_profile".to_owned())
            .or_insert_with(|| json!("balanced"));
    }

    Ok(SafeLegacyValues {
        server_url,
        device_id,
        preferences,
        skipped_sensitive_fields,
    })
}

fn find_string<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| object.get(*name).and_then(Value::as_str))
}

fn validate_legacy_server_url(value: &str) -> Option<Url> {
    let parsed = Url::parse(value).ok()?;
    (matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none())
    .then_some(parsed)
}

fn is_safe_preference_value(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
        Value::String(value) => value.len() <= 1_024 && !value.chars().any(char::is_control),
        Value::Array(_) | Value::Object(_) => false,
    }
}

fn contains_legacy_network(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.to_ascii_lowercase().contains("tailscale"))
}

fn count_sensitive_fields(value: &Value) -> usize {
    match value {
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| usize::from(is_sensitive_key(key)) + count_sensitive_fields(value))
            .sum(),
        Value::Array(values) => values.iter().map(count_sensitive_fields).sum(),
        _ => 0,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = normalize_key(key);
    [
        "password",
        "token",
        "secret",
        "credential",
        "privatekey",
        "databaseurl",
    ]
    .iter()
    .any(|sensitive| key == *sensitive || key.ends_with(sensitive))
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn snake_to_camel(key: &str) -> String {
    let mut output = String::with_capacity(key.len());
    let mut uppercase = false;
    for character in key.chars() {
        if character == '_' {
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn write_backup(source: &Path, bytes: &[u8]) -> Result<PathBuf, LegacyMigrationError> {
    for suffix in 0..=10 {
        let extension = if suffix == 0 {
            "sanser-v1.backup".to_owned()
        } else {
            format!("sanser-v1.backup.{suffix}")
        };
        let backup = source.with_extension(extension);
        let opened = secure_create(&backup);
        match opened {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                return Ok(backup);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(LegacyMigrationError::BackupSlotsExhausted)
}

#[cfg(unix)]
fn secure_create(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn secure_create(path: &Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[derive(Debug, Error)]
pub enum LegacyMigrationError {
    #[error("legacy settings path is outside the data directory")]
    PathOutsideDataDirectory,
    #[error("legacy settings source is invalid or too large")]
    InvalidSource,
    #[error("legacy settings JSON is invalid")]
    InvalidJson,
    #[error("legacy migration timestamp is too large")]
    TimestampOverflow,
    #[error("all protected legacy backup slots already exist")]
    BackupSlotsExhausted,
    #[error("legacy migration I/O failed")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("legacy migration database transaction failed")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DatabaseTarget, PreferenceStore, StorageOptions};
    use tempfile::tempdir;

    #[tokio::test]
    async fn imports_safe_values_transactionally_and_skips_secrets() {
        let directory = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let source = directory.path().join("store.json");
        fs::write(
            &source,
            br#"{
                "serverUrl":"https://signal.example.com",
                "deviceId":"550e8400-e29b-41d4-a716-446655440000",
                "accessToken":"must-not-import",
                "preferences": {
                    "networkMode":"tailscale",
                    "qualityProfile":"tailscale",
                    "theme":"dark",
                    "password":"must-not-import"
                }
            }"#,
        )
        .unwrap_or_else(|error| panic!("fixture write failed: {error}"));
        let storage = Storage::connect(DatabaseTarget::SqliteMemory, StorageOptions::default())
            .await
            .unwrap_or_else(|error| panic!("connect failed: {error}"));
        let report = LegacyMigrator::new(directory.path().to_path_buf())
            .migrate_file(&storage, &source, 10)
            .await
            .unwrap_or_else(|error| panic!("migration failed: {error}"));

        assert!(report.backup_path.exists());
        assert!(report.skipped_sensitive_fields >= 2);
        assert_eq!(
            storage
                .preference("network_mode")
                .await
                .unwrap_or_else(|error| panic!("read failed: {error}")),
            Some(json!("auto"))
        );
        assert_eq!(
            storage
                .preference("quality_profile")
                .await
                .unwrap_or_else(|error| panic!("read failed: {error}")),
            Some(json!("balanced"))
        );
        assert_eq!(
            storage
                .preference("access_token")
                .await
                .unwrap_or_else(|error| panic!("read failed: {error}")),
            None
        );

        let pool = storage
            .sqlite_pool()
            .unwrap_or_else(|error| panic!("{error}"));
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM app_metadata ORDER BY key")
                .fetch_all(pool)
                .await
                .unwrap_or_else(|error| panic!("metadata read failed: {error}"));
        let rendered = format!("{rows:?}");
        assert!(!rendered.contains("must-not-import"));
        assert!(rendered.contains("migration_version"));
    }

    #[tokio::test]
    async fn rejects_path_traversal_and_symlinks_outside_data_directory() {
        let root = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
        let data = root.path().join("data");
        fs::create_dir(&data).unwrap_or_else(|error| panic!("mkdir failed: {error}"));
        let outside = root.path().join("store.json");
        fs::write(&outside, "{}").unwrap_or_else(|error| panic!("fixture write failed: {error}"));
        let storage = Storage::connect(DatabaseTarget::SqliteMemory, StorageOptions::default())
            .await
            .unwrap_or_else(|error| panic!("connect failed: {error}"));
        let error = match LegacyMigrator::new(data)
            .migrate_file(&storage, &outside, 1)
            .await
        {
            Ok(_) => panic!("outside path must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            LegacyMigrationError::PathOutsideDataDirectory
        ));
    }
}
