//! Opt-in Wake-on-LAN information reported by a host.
//!
//! The server only stores what a host chose to share (adapter MAC and IPv4
//! LAN details) so another computer of the same account can send a magic
//! packet from inside that LAN. The server never sends packets itself.
use std::net::Ipv4Addr;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::{AuthContext, audit},
    error::{ApiJson, AppError},
    events,
    models::{Device, WakeInterface, WakeOnLan},
    routes::devices,
    state::AppState,
    time::now_unix,
};

pub const MAX_WAKE_INTERFACES: usize = 8;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WakeOnLanRequest {
    interfaces: Vec<WakeInterface>,
}

pub async fn set(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<WakeOnLanRequest>,
) -> Result<Json<Device>, AppError> {
    let id = validate_device_id(&id)?;
    let device = devices::fetch_owned(&state, &auth.user_id, &id).await?;
    if device.device_role != "host" {
        return Err(AppError::Conflict(
            "only a host registration can share Wake-on-LAN details".into(),
        ));
    }
    let wake = WakeOnLan {
        interfaces: normalize_interfaces(request.interfaces)?,
        updated_at: now_unix(),
    };
    let stored = serde_json::to_string(&wake).map_err(|_| AppError::Internal)?;
    store(&state, &auth, &id, Some(stored), "device.wake_on_lan.set").await
}

pub async fn clear(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = validate_device_id(&id)?;
    devices::fetch_owned(&state, &auth.user_id, &id).await?;
    let _cleared = store(&state, &auth, &id, None, "device.wake_on_lan.clear").await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn store(
    state: &AppState,
    auth: &AuthContext,
    id: &str,
    value: Option<String>,
    action: &str,
) -> Result<Json<Device>, AppError> {
    let updated = sqlx::query(
        "UPDATE devices SET wake_on_lan_json = $1, updated_at = $2 WHERE id = $3 AND user_id = $4",
    )
    .bind(&value)
    .bind(now_unix())
    .bind(id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if updated.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    events::publish(
        state,
        &auth.user_id,
        None,
        "device.updated",
        serde_json::json!({"deviceId": id}),
    )
    .await?;
    // MAC addresses stay out of the audit metadata.
    audit(
        &state.pool,
        Some(&auth.user_id),
        action,
        Some("device"),
        Some(id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(devices::fetch_owned(state, &auth.user_id, id).await?))
}

fn validate_device_id(value: &str) -> Result<String, AppError> {
    uuid::Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation("id must be a UUID".into()))
}

/// Reads the stored column. Corrupt data is treated as "not shared" so one bad
/// row can never break the device list.
pub(crate) fn parse_stored(value: Option<String>) -> Option<WakeOnLan> {
    let wake = serde_json::from_str::<WakeOnLan>(value.as_deref()?).ok()?;
    (!wake.interfaces.is_empty()).then_some(wake)
}

pub(crate) fn normalize_interfaces(
    interfaces: Vec<WakeInterface>,
) -> Result<Vec<WakeInterface>, AppError> {
    if interfaces.is_empty() || interfaces.len() > MAX_WAKE_INTERFACES {
        return Err(AppError::Validation(format!(
            "interfaces must contain between 1 and {MAX_WAKE_INTERFACES} adapters"
        )));
    }
    let mut normalized: Vec<WakeInterface> = Vec::with_capacity(interfaces.len());
    for interface in interfaces {
        let mac_address = normalize_mac(&interface.mac_address)?;
        let address = interface
            .ipv4_address
            .trim()
            .parse::<Ipv4Addr>()
            .map_err(|_| AppError::Validation("ipv4Address must be an IPv4 address".into()))?;
        if !address.is_private() {
            return Err(AppError::Validation(
                "ipv4Address must be a private LAN address".into(),
            ));
        }
        if !(8..=30).contains(&interface.prefix_length) {
            return Err(AppError::Validation(
                "prefixLength must be between 8 and 30".into(),
            ));
        }
        let expected = broadcast_for(address, interface.prefix_length);
        let reported = interface
            .broadcast_address
            .trim()
            .parse::<Ipv4Addr>()
            .map_err(|_| AppError::Validation("broadcastAddress must be an IPv4 address".into()))?;
        if reported != expected {
            return Err(AppError::Validation(
                "broadcastAddress does not match ipv4Address and prefixLength".into(),
            ));
        }
        let entry = WakeInterface {
            mac_address,
            ipv4_address: address.to_string(),
            prefix_length: interface.prefix_length,
            broadcast_address: expected.to_string(),
        };
        if !normalized.contains(&entry) {
            normalized.push(entry);
        }
    }
    Ok(normalized)
}

fn normalize_mac(value: &str) -> Result<String, AppError> {
    let invalid = || AppError::Validation("macAddress must be a unicast EUI-48 address".into());
    let value = value.trim();
    let separator = if value.contains('-') { '-' } else { ':' };
    let octets = value
        .split(separator)
        .map(|part| {
            (part.len() == 2)
                .then(|| u8::from_str_radix(part, 16).ok())
                .flatten()
        })
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(invalid)?;
    if octets.len() != 6 || octets.iter().all(|octet| *octet == 0) || octets[0] & 1 == 1 {
        return Err(invalid());
    }
    Ok(octets
        .iter()
        .map(|octet| format!("{octet:02x}"))
        .collect::<Vec<_>>()
        .join(":"))
}

fn broadcast_for(address: Ipv4Addr, prefix_length: u8) -> Ipv4Addr {
    let host_bits = u32::MAX >> u32::from(prefix_length);
    Ipv4Addr::from(u32::from(address) | host_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(mac: &str, ip: &str, prefix: u8, broadcast: &str) -> WakeInterface {
        WakeInterface {
            mac_address: mac.into(),
            ipv4_address: ip.into(),
            prefix_length: prefix,
            broadcast_address: broadcast.into(),
        }
    }

    #[test]
    fn normalizes_mac_formats_and_deduplicates() {
        let result = normalize_interfaces(vec![
            interface("AA-BB-CC-00-11-22", "192.168.1.30", 24, "192.168.1.255"),
            interface("aa:bb:cc:00:11:22", "192.168.1.30", 24, "192.168.1.255"),
            interface("02:00:00:00:00:01", "10.1.2.3", 16, "10.1.255.255"),
        ])
        .expect("valid interfaces");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].mac_address, "aa:bb:cc:00:11:22");
        assert_eq!(result[1].broadcast_address, "10.1.255.255");
    }

    #[test]
    fn rejects_unusable_or_inconsistent_adapters() {
        for bad in [
            interface("00:00:00:00:00:00", "192.168.1.30", 24, "192.168.1.255"),
            interface("ff:ff:ff:ff:ff:ff", "192.168.1.30", 24, "192.168.1.255"),
            interface("01:00:5e:00:00:01", "192.168.1.30", 24, "192.168.1.255"),
            interface("aa:bb:cc:00:11", "192.168.1.30", 24, "192.168.1.255"),
            interface("aa:bb:cc:00:11:2g", "192.168.1.30", 24, "192.168.1.255"),
            interface("aa:bb:cc:00:11:22", "8.8.8.8", 24, "8.8.8.255"),
            interface("aa:bb:cc:00:11:22", "192.168.1.30", 31, "192.168.1.31"),
            interface("aa:bb:cc:00:11:22", "192.168.1.30", 24, "255.255.255.255"),
        ] {
            assert!(normalize_interfaces(vec![bad.clone()]).is_err(), "{bad:?}");
        }
        assert!(normalize_interfaces(Vec::new()).is_err());
        let many = (0..=MAX_WAKE_INTERFACES)
            .map(|index| {
                interface(
                    &format!("02:00:00:00:00:{index:02x}"),
                    "192.168.1.30",
                    24,
                    "192.168.1.255",
                )
            })
            .collect();
        assert!(normalize_interfaces(many).is_err());
    }

    #[test]
    fn stored_values_that_cannot_be_read_are_hidden() {
        assert_eq!(parse_stored(None), None);
        assert_eq!(parse_stored(Some("not json".into())), None);
        assert_eq!(
            parse_stored(Some(r#"{"interfaces":[],"updatedAt":1}"#.into())),
            None
        );
        let stored = r#"{"interfaces":[{"macAddress":"aa:bb:cc:00:11:22","ipv4Address":"192.168.1.30","prefixLength":24,"broadcastAddress":"192.168.1.255"}],"updatedAt":5}"#;
        assert_eq!(
            parse_stored(Some(stored.into())).expect("wake").updated_at,
            5
        );
    }
}
