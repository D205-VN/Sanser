use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Manager, ResourceId, Webview};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

use crate::error::DesktopError;

const RELEASE_API: &str = "https://api.github.com/repos/D205-VN/Sanser/releases/latest";
const ASSET_API: &str = "https://api.github.com/repos/D205-VN/Sanser/releases/assets/";
const DOWNLOAD_ROOT: &str = "https://github.com/D205-VN/Sanser/releases/download/";
const MAX_METADATA: usize = 512 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    browser_download_url: String,
}

impl Release {
    fn validate(&self) -> Result<(), DesktopError> {
        let version = self.tag_name.strip_prefix('v').unwrap_or("");
        if self.draft
            || self.prerelease
            || version.is_empty()
            || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        {
            return Err(invalid_release());
        }
        Ok(())
    }

    fn asset(&self, name: &str) -> Result<&Asset, DesktopError> {
        let mut matches = self.assets.iter().filter(|asset| asset.name == name);
        let asset = matches.next().ok_or_else(invalid_release)?;
        let expected = format!("{DOWNLOAD_ROOT}{}/{name}", self.tag_name);
        if matches.next().is_some() || asset.id == 0 || asset.browser_download_url != expected {
            return Err(invalid_release());
        }
        Ok(asset)
    }

    fn download_asset(&self, url: &str) -> Result<&Asset, DesktopError> {
        let prefix = format!("{DOWNLOAD_ROOT}{}/", self.tag_name);
        let name = url.strip_prefix(&prefix).ok_or_else(invalid_release)?;
        if name.contains(['/', '\\', '?', '#', '%']) || !name.starts_with("Sanser-") {
            return Err(invalid_release());
        }
        self.asset(name)
    }
}

fn invalid_release() -> DesktopError {
    DesktopError::Unavailable("The GitHub update release is incomplete or inconsistent".into())
}

fn asset_url(asset: &Asset) -> Result<Url, DesktopError> {
    Url::parse(&format!("{ASSET_API}{}", asset.id)).map_err(|_| invalid_release())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateMetadata {
    rid: ResourceId,
    current_version: String,
    version: String,
    date: Option<String>,
    body: Option<String>,
    raw_json: serde_json::Value,
}

// Reuse Tauri's actual Update resource: its semver comparison, pinned public
// key, signature verification and install/restart flow remain authoritative.
#[tauri::command]
pub(crate) async fn check_update_via_github(
    webview: Webview,
) -> Result<Option<UpdateMetadata>, DesktopError> {
    if webview.label() != "main" {
        return Err(DesktopError::InvalidRequest(
            "Update checks require the main desktop window".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Sanser-Updater")
        .build()
        .map_err(|_| invalid_release())?;
    let mut response = client
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| DesktopError::Unavailable("GitHub update metadata is unreachable".into()))?
        .error_for_status()
        .map_err(|error| {
            DesktopError::Unavailable(format!(
                "GitHub update metadata returned HTTP {}",
                error.status().map_or(0, |status| status.as_u16()),
            ))
        })?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| invalid_release())? {
        if chunk.len() > MAX_METADATA - bytes.len() {
            return Err(invalid_release());
        }
        bytes.extend_from_slice(&chunk);
    }
    let release: Release = serde_json::from_slice(&bytes).map_err(|_| invalid_release())?;
    release.validate()?;
    let manifest = release.asset("updater-latest.json")?;
    if manifest.size == 0 || manifest.size > u64::try_from(MAX_METADATA).unwrap_or(0) {
        return Err(invalid_release());
    }
    let updater = webview
        .updater_builder()
        .endpoints(vec![asset_url(manifest)?])
        .and_then(|builder| builder.header("Accept", "application/octet-stream"))
        .map(|builder| builder.timeout(Duration::from_secs(10)))
        .and_then(tauri_plugin_updater::UpdaterBuilder::build)
        .map_err(|_| invalid_release())?;
    let Some(mut update) = updater.check().await.map_err(|_| invalid_release())? else {
        return Ok(None);
    };
    if release.tag_name != format!("v{}", update.version) {
        return Err(invalid_release());
    }
    let installer = release.download_asset(update.download_url.as_str())?;
    update.download_url = asset_url(installer)?;
    // The same public GitHub asset API avoids a failing browser redirect for
    // the installer too. No GitHub token or account credentials are attached.
    update.timeout = Some(Duration::from_secs(120));
    let metadata = UpdateMetadata {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date: update
            .raw_json
            .get("pub_date")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        rid: webview.resources_table().add(update),
    };
    Ok(Some(metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> Release {
        Release {
            tag_name: "v2.3.7".into(),
            draft: false,
            prerelease: false,
            assets: vec![
                Asset {
                    id: 17,
                    name: "updater-latest.json".into(),
                    size: 512,
                    browser_download_url: format!("{DOWNLOAD_ROOT}v2.3.7/updater-latest.json"),
                },
                Asset {
                    id: 18,
                    name: "Sanser-Windows-2.3.7-x64-setup.exe".into(),
                    size: 4096,
                    browser_download_url: format!(
                        "{DOWNLOAD_ROOT}v2.3.7/Sanser-Windows-2.3.7-x64-setup.exe"
                    ),
                },
            ],
        }
    }

    #[test]
    fn fallback_only_uses_own_stable_release_assets() -> Result<(), DesktopError> {
        let mut release = release();
        release.validate()?;
        assert_eq!(
            asset_url(release.asset("updater-latest.json")?)?.as_str(),
            format!("{ASSET_API}17")
        );
        assert!(release.asset("missing.json").is_err());
        release.assets[0].browser_download_url = "https://untrusted.example/update.json".into();
        assert!(release.asset("updater-latest.json").is_err());
        release.draft = true;
        assert!(release.validate().is_err());
        release.draft = false;
        release.prerelease = true;
        assert!(release.validate().is_err());
        release.prerelease = false;
        release.tag_name = "v../other".into();
        assert!(release.validate().is_err());
        Ok(())
    }

    #[test]
    fn installer_mapping_rejects_cross_release_or_external_urls() -> Result<(), DesktopError> {
        let mut release = release();
        let own = format!("{DOWNLOAD_ROOT}v2.3.7/Sanser-Windows-2.3.7-x64-setup.exe");
        assert_eq!(
            asset_url(release.download_asset(&own)?)?.as_str(),
            format!("{ASSET_API}18")
        );
        for bad in [
            "https://untrusted.example/setup.exe".into(),
            own.replace("v2.3.7/", "v2.3.6/"),
            format!("{own}?token=ignored"),
            own.replace("Sanser-", "../Sanser-"),
        ] {
            assert!(release.download_asset(&bad).is_err());
        }
        release.assets[1].id = 0;
        assert!(release.download_asset(&own).is_err());
        Ok(())
    }
}
