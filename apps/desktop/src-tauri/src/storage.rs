use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

use keyring::Entry;
use tauri::{AppHandle, Manager};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    error::DesktopError,
    models::{DiagnosticsExport, Preferences},
};

const SERVICE_NAME: &str = "com.sanser.desktop";
const MAX_PREFERENCES_BYTES: u64 = 64 * 1024;
const MAX_DIAGNOSTICS_BYTES: usize = 1024 * 1024;
const MAX_SECRET_BYTES: usize = 16 * 1024;
const SECRET_KEYS: [&str; 3] = ["access_token", "refresh_token", "device_identity"];

fn preferences_path(app: &AppHandle) -> Result<PathBuf, DesktopError> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("preferences-v2.json"))
        .map_err(|error| DesktopError::Storage(error.to_string()))
}

fn validate_preferences(preferences: &Preferences) -> Result<(), DesktopError> {
    if preferences.schema_version != 2 {
        return Err(DesktopError::InvalidRequest(
            "preference schema version must be 2".into(),
        ));
    }
    let url = Url::parse(&preferences.server_url)
        .map_err(|_| DesktopError::InvalidRequest("server URL is invalid".into()))?;
    let loopback = url
        .host_str()
        .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(DesktopError::InvalidRequest(
            "server URL must use HTTPS outside localhost".into(),
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DesktopError::InvalidRequest(
            "server URL contains unsupported credentials or parameters".into(),
        ));
    }
    if !matches!(preferences.stream.fps, 30 | 60 | 90 | 120)
        || !(1.0..=200.0).contains(&preferences.stream.bitrate_mbps)
    {
        return Err(DesktopError::InvalidRequest(
            "stream settings are outside supported bounds".into(),
        ));
    }
    if !matches!(preferences.input.polling_rate, 60 | 90 | 120 | 240)
        || preferences.input.release_shortcut.is_empty()
        || preferences.input.release_shortcut.len() > 80
    {
        return Err(DesktopError::InvalidRequest(
            "input settings are outside supported bounds".into(),
        ));
    }
    if preferences.host.direct_udp_port < 1_024 {
        return Err(DesktopError::InvalidRequest(
            "host direct UDP port must be between 1024 and 65535".into(),
        ));
    }
    if preferences.pinned_device_ids.len() > 200
        || preferences
            .pinned_device_ids
            .iter()
            .any(|identifier| identifier.is_empty() || identifier.len() > 128)
    {
        return Err(DesktopError::InvalidRequest(
            "pinned device list is outside supported bounds".into(),
        ));
    }
    if preferences.trusted_device_ids.len() > 200
        || preferences
            .trusted_device_ids
            .iter()
            .any(|identifier| Uuid::parse_str(identifier).is_err())
    {
        return Err(DesktopError::InvalidRequest(
            "trusted device list is outside supported bounds".into(),
        ));
    }
    Ok(())
}

pub fn load_preferences(app: &AppHandle) -> Result<Option<Preferences>, DesktopError> {
    let path = preferences_path(app)?;
    let Ok(metadata) = fs::metadata(&path) else {
        return Ok(None);
    };
    if metadata.len() > MAX_PREFERENCES_BYTES {
        return Err(DesktopError::Storage(
            "preferences file is too large".into(),
        ));
    }
    let file = fs::File::open(path)?;
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| DesktopError::Storage("preferences file length is invalid".into()))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_PREFERENCES_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let preferences: Preferences = serde_json::from_slice(&bytes)
        .map_err(|error| DesktopError::Storage(format!("invalid preferences: {error}")))?;
    validate_preferences(&preferences)?;
    Ok(Some(preferences))
}

pub fn save_preferences(app: &AppHandle, preferences: &Preferences) -> Result<(), DesktopError> {
    validate_preferences(preferences)?;
    let serialized = serde_json::to_vec_pretty(preferences)
        .map_err(|error| DesktopError::Storage(error.to_string()))?;
    if serialized.len() as u64 > MAX_PREFERENCES_BYTES {
        return Err(DesktopError::InvalidRequest(
            "preferences are too large".into(),
        ));
    }
    let path = preferences_path(app)?;
    let directory = path
        .parent()
        .ok_or_else(|| DesktopError::Storage("preferences path has no parent".into()))?;
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(".preferences-v2-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<(), DesktopError> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serialized)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    #[cfg(target_os = "windows")]
    if path.exists() {
        // std::fs::rename cannot replace an existing file on Windows. The
        // unique temporary file still prevents concurrent writers from
        // corrupting one another.
        fs::remove_file(&path)?;
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn secret_entry(key: &str) -> Result<Entry, DesktopError> {
    if !SECRET_KEYS.contains(&key) {
        return Err(DesktopError::InvalidRequest(
            "secure storage key is not allowlisted".into(),
        ));
    }
    Entry::new(SERVICE_NAME, key).map_err(|_| DesktopError::SecureStorage)
}

pub fn secure_get(key: &str) -> Result<Option<String>, DesktopError> {
    match secret_entry(key)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(DesktopError::SecureStorage),
    }
}

pub fn secure_set(key: &str, value: String) -> Result<(), DesktopError> {
    let value = Zeroizing::new(value);
    if value.is_empty() || value.len() > MAX_SECRET_BYTES {
        return Err(DesktopError::InvalidRequest(
            "secure value is outside supported bounds".into(),
        ));
    }
    secret_entry(key)?
        .set_password(value.as_str())
        .map_err(|_| DesktopError::SecureStorage)
}

pub fn secure_delete(key: &str) -> Result<(), DesktopError> {
    match secret_entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(DesktopError::SecureStorage),
    }
}

fn sensitive_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    [
        "password",
        "token",
        "credential",
        "secret",
        "authorization",
        "privatekey",
        "private_key",
        "databaseurl",
        "database_url",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn sanitize_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if sensitive_key(key) {
                    *child = serde_json::Value::String("[REDACTED]".into());
                } else {
                    sanitize_json(child);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(sanitize_json),
        serde_json::Value::String(text) => {
            if text.to_ascii_lowercase().contains("bearer ") {
                *text = "Bearer [REDACTED]".into();
            } else if text.len() > 2048 {
                text.truncate(2048);
            }
        }
        _ => {}
    }
}

pub fn export_diagnostics(
    app: &AppHandle,
    contents: &str,
) -> Result<DiagnosticsExport, DesktopError> {
    if contents.len() > MAX_DIAGNOSTICS_BYTES {
        return Err(DesktopError::InvalidRequest(
            "diagnostics export is too large".into(),
        ));
    }
    let mut value: serde_json::Value = serde_json::from_str(contents)
        .map_err(|_| DesktopError::InvalidRequest("diagnostics must be valid JSON".into()))?;
    sanitize_json(&mut value);
    let serialized = serde_json::to_vec_pretty(&value)
        .map_err(|error| DesktopError::Storage(error.to_string()))?;
    let directory = app
        .path()
        .app_log_dir()
        .map_err(|error| DesktopError::Storage(error.to_string()))?;
    fs::create_dir_all(&directory)?;
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| DesktopError::Storage(error.to_string()))?
        .as_secs();
    let path = directory.join(format!("sanser-diagnostics-{epoch}.json"));
    fs::write(&path, serialized)?;
    Ok(DiagnosticsExport {
        path: path.to_string_lossy().into_owned(),
    })
}
