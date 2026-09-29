use std::{fs, io::Write, path::Path, sync::Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::error::DesktopError;

static IDENTITY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
struct SavedIdentity {
    computer_id: Uuid,
    client_id: Uuid,
    host_id: Uuid,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    id: Uuid,
    computer_id: Uuid,
    name: String,
}

fn load_or_create(
    path: &Path,
    client: Option<Uuid>,
    host: Option<Uuid>,
) -> Result<SavedIdentity, DesktopError> {
    // A damaged identity must not silently become another computer on the server.
    match fs::read(path) {
        Ok(bytes) => {
            return serde_json::from_slice(&bytes).map_err(|error| {
                DesktopError::Storage(format!("Cannot read saved computer identity: {error}"))
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let identity = SavedIdentity {
        computer_id: Uuid::new_v4(),
        client_id: client.unwrap_or_else(Uuid::new_v4),
        host_id: host.unwrap_or_else(Uuid::new_v4),
    };
    let bytes =
        serde_json::to_vec(&identity).map_err(|error| DesktopError::Storage(error.to_string()))?;
    let parent = path
        .parent()
        .ok_or_else(|| DesktopError::Storage("Identity directory is unavailable".into()))?;
    fs::create_dir_all(parent)?;
    // Publish a complete file without replacing another process's identity.
    let temporary = parent.join(format!(".identity-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<SavedIdentity, DesktopError> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        match fs::hard_link(&temporary, path) {
            Ok(()) => Ok(identity),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                serde_json::from_slice(&fs::read(path)?)
                    .map_err(|error| DesktopError::Storage(error.to_string()))
            }
            Err(error) => Err(error.into()),
        }
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        && output.status.success()
    {
        let name = String::from_utf8_lossy(&output.stdout)
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(100)
            .collect::<String>();
        if !name.is_empty() {
            return name;
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|name| {
            name.trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("{} computer", std::env::consts::OS))
}

// Tauri IPC extracts owned arguments.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn get_device_identity(
    app: AppHandle,
    account_id: String,
    role: String,
    legacy_client_id: Option<String>,
    legacy_host_id: Option<String>,
) -> Result<DeviceIdentity, DesktopError> {
    let account = Uuid::parse_str(&account_id)
        .map_err(|_| DesktopError::InvalidRequest("Invalid account ID".into()))?;
    if role != "client" && role != "host" {
        return Err(DesktopError::InvalidRequest("Invalid device role".into()));
    }
    let directory = app
        .path()
        .app_config_dir()
        .map_err(|error| DesktopError::Storage(error.to_string()))?;
    let _guard = IDENTITY_LOCK
        .lock()
        .map_err(|_| DesktopError::Storage("Identity lock unavailable".into()))?;
    let saved = load_or_create(
        &directory.join("computers").join(format!("{account}.json")),
        legacy_client_id.and_then(|id| Uuid::parse_str(&id).ok()),
        legacy_host_id.and_then(|id| Uuid::parse_str(&id).ok()),
    )?;
    Ok(DeviceIdentity {
        id: if role == "host" {
            saved.host_id
        } else {
            saved.client_id
        },
        computer_id: saved.computer_id,
        name: computer_name(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_starts_publish_only_one_complete_identity() -> Result<(), DesktopError> {
        let directory = std::env::temp_dir().join(format!("sanser-identity-{}", Uuid::new_v4()));
        let handles = (0..8)
            .map(|_| {
                let path = directory.join("account.json");
                std::thread::spawn(move || {
                    load_or_create(&path, None, None).map(|saved| saved.computer_id)
                })
            })
            .collect::<Vec<_>>();
        let mut ids = Vec::new();
        for handle in handles {
            ids.push(
                handle
                    .join()
                    .map_err(|_| DesktopError::Storage("identity worker panicked".into()))??,
            );
        }
        assert!(ids.iter().all(|id| Some(id) == ids.first()));
        let other = load_or_create(&directory.join("another-account.json"), None, None)?;
        assert_ne!(ids.first(), Some(&other.computer_id));
        fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[test]
    fn survives_restart_and_keeps_existing_role_ids() -> Result<(), DesktopError> {
        let directory = std::env::temp_dir().join(format!("sanser-identity-{}", Uuid::new_v4()));
        let path = directory.join("account.json");
        let old = Uuid::new_v4();
        let first = load_or_create(&path, Some(old), None)?;
        let next = load_or_create(&path, None, Some(Uuid::new_v4()))?;
        assert_eq!(next.client_id, old);
        assert_eq!(first.host_id, next.host_id);
        assert_eq!(first.computer_id, next.computer_id);
        assert_ne!(next.client_id, next.host_id);
        fs::write(&path, "broken")?;
        assert!(load_or_create(&path, None, None).is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
