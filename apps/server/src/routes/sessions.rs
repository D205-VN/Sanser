use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sanser_core::NetworkMode;
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{AuthContext, audit},
    error::{ApiJson, AppError},
    events,
    models::{ConnectionSession, Device},
    routes::devices,
    state::AppState,
    time::now_unix,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSessionRequest {
    #[serde(default)]
    requester_device_id: Option<String>,
    host_device_id: String,
    #[serde(default)]
    network_mode: NetworkMode,
    #[serde(default = "default_quality")]
    quality_profile: String,
    #[serde(default = "default_codec")]
    requested_codec: String,
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<CreateSessionRequest>,
) -> Result<Response, AppError> {
    let host_id = validate_uuid(&request.host_device_id, "hostDeviceId")?;
    let host = devices::fetch_owned(&state, &auth.user_id, &host_id).await?;
    ensure_device_online(&state, &host)?;
    let requester_id = match request.requester_device_id {
        Some(id) => validate_uuid(&id, "requesterDeviceId")?,
        None => most_recent_requester(&state, &auth.user_id, &host_id).await?,
    };
    if requester_id == host_id {
        return Err(AppError::Validation(
            "requesterDeviceId and hostDeviceId must differ".into(),
        ));
    }
    let requester = devices::fetch_owned(&state, &auth.user_id, &requester_id).await?;
    ensure_device_online(&state, &requester)?;

    let quality_profile = normalize_quality(&request.quality_profile)?;
    let requested_codec = normalize_codec(&request.requested_codec)?;
    if requested_codec != "auto"
        && (!host.codecs.iter().any(|codec| codec == &requested_codec)
            || !requester
                .codecs
                .iter()
                .any(|codec| codec == &requested_codec))
    {
        return Err(AppError::Conflict(format!(
            "both devices must support {requested_codec}"
        )));
    }
    if request.network_mode == NetworkMode::Relay && state.config.turn_urls.is_empty() {
        return Err(AppError::Unavailable);
    }
    if !host.webrtc && !host.native_transport {
        return Err(AppError::Conflict(
            "host has no compatible transport".into(),
        ));
    }
    if !requester.webrtc && !requester.native_transport {
        return Err(AppError::Conflict(
            "requester has no compatible transport".into(),
        ));
    }

    let active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM connection_sessions WHERE user_id = $1 \
         AND state IN ('pending', 'accepted') \
         AND (requester_device_id IN ($2, $3) OR host_device_id IN ($4, $5))",
    )
    .bind(&auth.user_id)
    .bind(&requester_id)
    .bind(&host_id)
    .bind(&requester_id)
    .bind(&host_id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if active > 0 {
        return Err(AppError::Conflict(
            "one of the devices already has an active session".into(),
        ));
    }

    let now = now_unix();
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO connection_sessions \
         (id, user_id, requester_device_id, host_device_id, state, network_mode, \
          quality_profile, requested_codec, selected_transport, created_at, updated_at, \
          accepted_at, ended_at, disconnect_reason) \
         VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7, NULL, $8, $9, NULL, NULL, NULL)",
    )
    .bind(&id)
    .bind(&auth.user_id)
    .bind(&requester_id)
    .bind(&host_id)
    .bind(network_mode_str(request.network_mode))
    .bind(&quality_profile)
    .bind(&requested_codec)
    .bind(now)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let session = fetch_owned(&state, &auth.user_id, &id).await?;
    events::publish(
        &state,
        &auth.user_id,
        Some(&id),
        "session.requested",
        serde_json::json!({
            "sessionId": id,
            "requesterDeviceId": requester_id,
            "hostDeviceId": host_id
        }),
    )
    .await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "session.create",
        Some("connection_session"),
        Some(&id),
        serde_json::json!({"networkMode": network_mode_str(request.network_mode)}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(session)).into_response())
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<ConnectionSession>, AppError> {
    let id = validate_uuid(&id, "id")?;
    Ok(Json(fetch_owned(&state, &auth.user_id, &id).await?))
}

pub async fn accept(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<ConnectionSession>, AppError> {
    let id = validate_uuid(&id, "id")?;
    let current = fetch_owned(&state, &auth.user_id, &id).await?;
    if current.state != "pending" {
        return Err(AppError::Conflict(
            "only a pending session can be accepted".into(),
        ));
    }
    let host = devices::fetch_owned(&state, &auth.user_id, &current.host_device_id).await?;
    let requester =
        devices::fetch_owned(&state, &auth.user_id, &current.requester_device_id).await?;
    ensure_device_online(&state, &host)?;
    ensure_device_online(&state, &requester)?;
    let selected_transport = select_transport(None, current.network_mode, &host, &requester)?;
    let now = now_unix();
    let updated = sqlx::query(
        "UPDATE connection_sessions SET state = 'accepted', selected_transport = $1, \
         accepted_at = $2, updated_at = $3 WHERE id = $4 AND user_id = $5 AND state = 'pending'",
    )
    .bind(&selected_transport)
    .bind(now)
    .bind(now)
    .bind(&id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if updated.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "session state changed concurrently".into(),
        ));
    }
    sqlx::query("UPDATE devices SET streaming = 1, updated_at = $1 WHERE id = $2 AND user_id = $3")
        .bind(now)
        .bind(&current.host_device_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        Some(&id),
        "session.accepted",
        serde_json::json!({"sessionId": id, "transport": selected_transport}),
    )
    .await?;
    Ok(Json(fetch_owned(&state, &auth.user_id, &id).await?))
}

pub async fn reject(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = validate_uuid(&id, "id")?;
    let current = fetch_owned(&state, &auth.user_id, &id).await?;
    if current.state != "pending" {
        return Err(AppError::Conflict(
            "only a pending session can be rejected".into(),
        ));
    }
    transition_to_ended(&state, &auth, &current, "rejected", None).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn disconnect(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = validate_uuid(&id, "id")?;
    let current = fetch_owned(&state, &auth.user_id, &id).await?;
    if matches!(
        current.state.as_str(),
        "disconnected" | "rejected" | "expired"
    ) {
        return Ok(StatusCode::NO_CONTENT);
    }
    transition_to_ended(&state, &auth, &current, "disconnected", None).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn transition_to_ended(
    state: &AppState,
    auth: &AuthContext,
    current: &ConnectionSession,
    new_state: &str,
    reason: Option<String>,
) -> Result<(), AppError> {
    let reason = reason
        .map(|reason| {
            let reason = reason.trim();
            if reason.chars().count() > 240 || reason.chars().any(char::is_control) {
                Err(AppError::Validation(
                    "reason must contain no more than 240 printable characters".into(),
                ))
            } else if reason.is_empty() {
                Ok(None)
            } else {
                Ok(Some(reason.to_owned()))
            }
        })
        .transpose()?
        .flatten();
    let now = now_unix();
    let updated = sqlx::query(
        "UPDATE connection_sessions SET state = $1, ended_at = $2, updated_at = $3, \
         disconnect_reason = $4 WHERE id = $5 AND user_id = $6 AND state IN ('pending', 'accepted')",
    )
    .bind(new_state)
    .bind(now)
    .bind(now)
    .bind(&reason)
    .bind(&current.id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if updated.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "session state changed concurrently".into(),
        ));
    }
    sqlx::query("UPDATE devices SET streaming = 0, updated_at = $1 WHERE id = $2 AND user_id = $3")
        .bind(now)
        .bind(&current.host_device_id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    events::publish(
        state,
        &auth.user_id,
        Some(&current.id),
        &format!("session.{new_state}"),
        serde_json::json!({"sessionId": current.id, "reason": reason}),
    )
    .await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        &format!("session.{new_state}"),
        Some("connection_session"),
        Some(&current.id),
        serde_json::json!({}),
    )
    .await;
    Ok(())
}

pub(crate) async fn fetch_owned(
    state: &AppState,
    user_id: &str,
    id: &str,
) -> Result<ConnectionSession, AppError> {
    let row = sqlx::query("SELECT * FROM connection_sessions WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?;
    row_to_session(row)
}

fn row_to_session(row: sqlx::any::AnyRow) -> Result<ConnectionSession, AppError> {
    let network_mode: String = row.try_get("network_mode").map_err(AppError::from_db)?;
    Ok(ConnectionSession {
        id: row.try_get("id").map_err(AppError::from_db)?,
        requester_device_id: row
            .try_get("requester_device_id")
            .map_err(AppError::from_db)?,
        host_device_id: row.try_get("host_device_id").map_err(AppError::from_db)?,
        state: row.try_get("state").map_err(AppError::from_db)?,
        network_mode: parse_network_mode(&network_mode)?,
        quality_profile: row.try_get("quality_profile").map_err(AppError::from_db)?,
        requested_codec: row.try_get("requested_codec").map_err(AppError::from_db)?,
        selected_transport: row
            .try_get("selected_transport")
            .map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
        updated_at: row.try_get("updated_at").map_err(AppError::from_db)?,
        accepted_at: row.try_get("accepted_at").map_err(AppError::from_db)?,
        ended_at: row.try_get("ended_at").map_err(AppError::from_db)?,
        disconnect_reason: row
            .try_get("disconnect_reason")
            .map_err(AppError::from_db)?,
    })
}

async fn most_recent_requester(
    state: &AppState,
    user_id: &str,
    host_id: &str,
) -> Result<String, AppError> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM devices WHERE user_id = $1 AND id != $2 AND online = 1 \
         ORDER BY last_seen_at DESC, id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(host_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or_else(|| {
        AppError::Validation(
            "requesterDeviceId is required when no other online device is registered".into(),
        )
    })
}

fn select_transport(
    requested: Option<&str>,
    mode: NetworkMode,
    host: &Device,
    requester: &Device,
) -> Result<String, AppError> {
    let selected = requested.map_or_else(
        || {
            if mode != NetworkMode::Relay
                && host.native_transport
                && requester.native_transport
                && host.route_address.is_some()
                && requester.route_address.is_some()
            {
                "snv2"
            } else {
                "webrtc"
            }
        },
        str::trim,
    );
    match selected {
        "snv2"
            if mode != NetworkMode::Relay
                && host.native_transport
                && requester.native_transport =>
        {
            Ok(selected.into())
        }
        "webrtc" if host.webrtc && requester.webrtc => Ok(selected.into()),
        "snv2" | "webrtc" => Err(AppError::Conflict(
            "selected transport is not supported by both devices".into(),
        )),
        _ => Err(AppError::Validation(
            "selectedTransport must be snv2 or webrtc".into(),
        )),
    }
}

fn ensure_device_online(state: &AppState, device: &Device) -> Result<(), AppError> {
    let offline_before = now_unix().saturating_sub(
        i64::try_from(state.config.device_offline_after.as_secs()).unwrap_or(i64::MAX),
    );
    if !device.online
        || device
            .last_seen_at
            .is_none_or(|last_seen| last_seen < offline_before)
    {
        return Err(AppError::Conflict(format!(
            "device {} is offline",
            device.id
        )));
    }
    Ok(())
}

fn normalize_quality(value: &str) -> Result<String, AppError> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(
        value.as_str(),
        "auto" | "competitive" | "balanced" | "quality" | "custom"
    ) {
        Ok(value)
    } else {
        Err(AppError::Validation(
            "qualityProfile must be auto, competitive, balanced, quality, or custom".into(),
        ))
    }
}

fn normalize_codec(value: &str) -> Result<String, AppError> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "auto" | "h264" | "hevc") {
        Ok(value)
    } else {
        Err(AppError::Validation(
            "requestedCodec must be auto, h264, or hevc".into(),
        ))
    }
}

fn parse_network_mode(value: &str) -> Result<NetworkMode, AppError> {
    match value {
        "auto" => Ok(NetworkMode::Auto),
        "direct" => Ok(NetworkMode::Direct),
        "relay" => Ok(NetworkMode::Relay),
        _ => {
            tracing::error!(value, "stored session has an invalid network mode");
            Err(AppError::Internal)
        }
    }
}

const fn network_mode_str(mode: NetworkMode) -> &'static str {
    match mode {
        NetworkMode::Auto => "auto",
        NetworkMode::Direct => "direct",
        NetworkMode::Relay => "relay",
    }
}

fn validate_uuid(value: &str, field: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation(format!("{field} must be a UUID")))
}

fn default_quality() -> String {
    "auto".into()
}

fn default_codec() -> String {
    "auto".into()
}
