use std::{collections::BTreeSet, net::IpAddr};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{AuthContext, audit, clean_label},
    config::{PROTOCOL_VERSION, SANSER_VERSION},
    error::{ApiJson, AppError},
    events,
    models::{Device, Page},
    state::AppState,
    time::now_unix,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterDeviceRequest {
    #[serde(default)]
    id: Option<String>,
    name: String,
    platform: String,
    #[serde(default)]
    os_version: String,
    #[serde(default)]
    gpu: String,
    #[serde(default = "default_version", alias = "version")]
    sanser_version: String,
    #[serde(default = "default_protocol_version")]
    protocol_version: u8,
    #[serde(default = "default_codecs")]
    codecs: Vec<String>,
    #[serde(default)]
    native_transport: bool,
    #[serde(default = "default_true", alias = "webRtc")]
    webrtc: bool,
    #[serde(default = "default_true")]
    audio: bool,
    #[serde(default)]
    gamepad: bool,
    #[serde(default)]
    route_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeartbeatRequest {
    device_id: String,
    #[serde(default)]
    streaming: bool,
    #[serde(default)]
    route_address: Option<String>,
    #[serde(default)]
    network_quality: Option<String>,
    #[serde(default)]
    latency_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfflineRequest {
    device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateDeviceRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pinned: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<u16>,
}

#[derive(Serialize, Deserialize)]
struct DeviceCursor {
    updated_at: i64,
    id: String,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<Device>>, AppError> {
    let limit = i64::from(query.limit.unwrap_or(50).clamp(1, 100));
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let rows = if let Some(cursor) = cursor {
        sqlx::query(
            "SELECT * FROM devices WHERE user_id = $1 AND \
             (updated_at < $2 OR (updated_at = $3 AND id < $4)) \
             ORDER BY updated_at DESC, id DESC LIMIT $5",
        )
        .bind(&auth.user_id)
        .bind(cursor.updated_at)
        .bind(cursor.updated_at)
        .bind(cursor.id)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::from_db)?
    } else {
        sqlx::query(
            "SELECT * FROM devices WHERE user_id = $1 \
             ORDER BY updated_at DESC, id DESC LIMIT $2",
        )
        .bind(&auth.user_id)
        .bind(limit + 1)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::from_db)?
    };

    let has_more = rows.len() > usize::try_from(limit).unwrap_or(100);
    let mut items = rows
        .into_iter()
        .take(usize::try_from(limit).unwrap_or(100))
        .map(row_to_device)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if has_more {
        items.last().map(encode_cursor).transpose()?
    } else {
        None
    };
    // A stale device is immediately represented as offline even before the
    // periodic cleanup has written that state to the database.
    let offline_before = now_unix().saturating_sub(
        i64::try_from(state.config.device_offline_after.as_secs()).unwrap_or(i64::MAX),
    );
    for device in &mut items {
        if device
            .last_seen_at
            .is_some_and(|last_seen| last_seen < offline_before)
        {
            device.online = false;
            device.streaming = false;
        }
    }
    Ok(Json(Page { items, next_cursor }))
}

pub async fn register(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<RegisterDeviceRequest>,
) -> Result<Response, AppError> {
    let id = match request.id {
        Some(id) => validate_uuid(&id, "id")?,
        None => Uuid::new_v4().to_string(),
    };
    let name = clean_label(&request.name, "name", 120)?;
    let platform = clean_label(&request.platform, "platform", 40)?;
    let os_version = optional_label(&request.os_version, "osVersion", 120)?;
    let gpu = optional_label(&request.gpu, "gpu", 240)?;
    let version_prefix = SANSER_VERSION.rsplitn(2, '.').last().unwrap_or(SANSER_VERSION);
    let client_prefix = request.sanser_version.rsplitn(2, '.').last().unwrap_or(&request.sanser_version);
    if client_prefix != version_prefix {
        return Err(AppError::Validation(format!(
            "sanserVersion must be compatible with {SANSER_VERSION}"
        )));
    }
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(AppError::Conflict(format!(
            "protocolVersion {} is not compatible with server protocol {PROTOCOL_VERSION}",
            request.protocol_version
        )));
    }
    let codecs = normalize_codecs(request.codecs)?;
    let route_address = normalize_route(request.route_address)?;
    let existing_owner =
        sqlx::query_scalar::<_, String>("SELECT user_id FROM devices WHERE id = $1")
            .bind(&id)
            .fetch_optional(&state.pool)
            .await
            .map_err(AppError::from_db)?;
    if existing_owner
        .as_deref()
        .is_some_and(|owner| owner != auth.user_id)
    {
        return Err(AppError::Conflict("device id is already registered".into()));
    }

    let now = now_unix();
    let codecs_json = serde_json::to_string(&codecs).map_err(|_| AppError::Internal)?;
    let created = existing_owner.is_none();
    if created {
        sqlx::query(
            "INSERT INTO devices \
             (id, user_id, name, platform, os_version, gpu, sanser_version, protocol_version, \
              online, streaming, pinned, route_address, network_quality, latency_ms, codecs_json, \
              native_transport, webrtc, audio, gamepad, last_seen_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, FALSE, FALSE, $9, NULL, NULL, \
                     $10, $11, $12, $13, $14, $15, $16, $17)",
        )
        .bind(&id)
        .bind(&auth.user_id)
        .bind(&name)
        .bind(&platform)
        .bind(&os_version)
        .bind(&gpu)
        .bind(SANSER_VERSION)
        .bind(i64::from(PROTOCOL_VERSION))
        .bind(&route_address)
        .bind(&codecs_json)
        .bind(request.native_transport)
        .bind(request.webrtc)
        .bind(request.audio)
        .bind(request.gamepad)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    } else {
        sqlx::query(
            "UPDATE devices SET name = $1, platform = $2, os_version = $3, gpu = $4, \
             sanser_version = $5, protocol_version = $6, online = TRUE, route_address = $7, \
             codecs_json = $8, native_transport = $9, webrtc = $10, audio = $11, gamepad = $12, \
             last_seen_at = $13, updated_at = $14 WHERE id = $15 AND user_id = $16",
        )
        .bind(&name)
        .bind(&platform)
        .bind(&os_version)
        .bind(&gpu)
        .bind(SANSER_VERSION)
        .bind(i64::from(PROTOCOL_VERSION))
        .bind(&route_address)
        .bind(&codecs_json)
        .bind(request.native_transport)
        .bind(request.webrtc)
        .bind(request.audio)
        .bind(request.gamepad)
        .bind(now)
        .bind(now)
        .bind(&id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    }
    let device = fetch_owned(&state, &auth.user_id, &id).await?;
    events::publish(
        &state,
        &auth.user_id,
        None,
        if created {
            "device.registered"
        } else {
            "device.updated"
        },
        serde_json::json!({"deviceId": id}),
    )
    .await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        if created {
            "device.register"
        } else {
            "device.reregister"
        },
        Some("device"),
        Some(&device.id),
        serde_json::json!({"platform": platform}),
    )
    .await;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(device)).into_response())
}

pub async fn heartbeat(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<HeartbeatRequest>,
) -> Result<Json<Device>, AppError> {
    let id = validate_uuid(&request.device_id, "deviceId")?;
    let route = normalize_route(request.route_address)?;
    let quality = request
        .network_quality
        .map(|value| normalize_quality(&value))
        .transpose()?;
    if request
        .latency_ms
        .is_some_and(|latency| !(0..=60_000).contains(&latency))
    {
        return Err(AppError::Validation(
            "latencyMs must be between 0 and 60000".into(),
        ));
    }
    let previous = fetch_owned(&state, &auth.user_id, &id).await?;
    let now = now_unix();
    let updated = sqlx::query(
        "UPDATE devices SET online = TRUE, streaming = $1, \
         route_address = COALESCE($2, route_address), \
         network_quality = COALESCE($3, network_quality), \
         latency_ms = COALESCE($4, latency_ms), last_seen_at = $5, updated_at = $6 \
         WHERE id = $7 AND user_id = $8",
    )
    .bind(request.streaming)
    .bind(&route)
    .bind(&quality)
    .bind(request.latency_ms)
    .bind(now)
    .bind(now)
    .bind(&id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if updated.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    if !previous.online || previous.streaming != request.streaming {
        events::publish(
            &state,
            &auth.user_id,
            None,
            "device.presence",
            serde_json::json!({
                "deviceId": id,
                "online": true,
                "streaming": request.streaming
            }),
        )
        .await?;
    }
    Ok(Json(fetch_owned(&state, &auth.user_id, &id).await?))
}

pub async fn offline(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<OfflineRequest>,
) -> Result<Json<Device>, AppError> {
    let id = validate_uuid(&request.device_id, "deviceId")?;
    let mut device = fetch_owned(&state, &auth.user_id, &id).await?;
    let changed = device.online || device.streaming;
    let now = now_unix();
    let mut transaction = state.pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE devices SET streaming = FALSE, updated_at = $1 WHERE user_id = $2 AND id IN (\
         SELECT host_device_id FROM connection_sessions WHERE user_id = $3 \
         AND state IN ('pending', 'accepted') \
         AND (requester_device_id = $4 OR host_device_id = $5))",
    )
    .bind(now)
    .bind(&auth.user_id)
    .bind(&auth.user_id)
    .bind(&id)
    .bind(&id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    let ended_sessions = sqlx::query_scalar::<_, String>(
        "UPDATE connection_sessions SET state = 'disconnected', ended_at = $1, updated_at = $1, \
         disconnect_reason = 'device_offline' WHERE user_id = $2 AND state IN ('pending', 'accepted') \
         AND (requester_device_id = $3 OR host_device_id = $4) RETURNING id",
    )
    .bind(now)
    .bind(&auth.user_id)
    .bind(&id)
    .bind(&id)
    .fetch_all(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    let updated = sqlx::query(
        "UPDATE devices SET online = FALSE, streaming = FALSE, last_seen_at = $1, updated_at = $1 \
         WHERE id = $2 AND user_id = $3",
    )
    .bind(now)
    .bind(&id)
    .bind(&auth.user_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    if updated.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    transaction.commit().await.map_err(AppError::from_db)?;
    device.online = false;
    device.streaming = false;
    device.last_seen_at = Some(now);
    device.updated_at = now;
    if changed {
        events::publish(
            &state,
            &auth.user_id,
            None,
            "device.presence",
            serde_json::json!({
                "deviceId": id,
                "online": false,
                "streaming": false
            }),
        )
        .await?;
    }
    for session_id in ended_sessions {
        events::publish(
            &state,
            &auth.user_id,
            Some(&session_id),
            "session.disconnected",
            serde_json::json!({
                "sessionId": session_id,
                "reason": "device_offline"
            }),
        )
        .await?;
    }
    audit(
        &state.pool,
        Some(&auth.user_id),
        "device.offline",
        Some("device"),
        Some(&id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(device))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateDeviceRequest>,
) -> Result<Json<Device>, AppError> {
    let id = validate_uuid(&id, "id")?;
    if request.name.is_none() && request.pinned.is_none() {
        return Err(AppError::Validation(
            "at least one of name or pinned is required".into(),
        ));
    }
    let current = fetch_owned(&state, &auth.user_id, &id).await?;
    let name = request
        .name
        .map(|name| clean_label(&name, "name", 120))
        .transpose()?
        .unwrap_or(current.name);
    let pinned = request.pinned.unwrap_or(current.pinned);
    sqlx::query(
        "UPDATE devices SET name = $1, pinned = $2, updated_at = $3 WHERE id = $4 AND user_id = $5",
    )
    .bind(name)
    .bind(pinned)
    .bind(now_unix())
    .bind(&id)
    .bind(&auth.user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        None,
        "device.updated",
        serde_json::json!({"deviceId": id}),
    )
    .await?;
    Ok(Json(fetch_owned(&state, &auth.user_id, &id).await?))
}

pub async fn remove(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = validate_uuid(&id, "id")?;
    fetch_owned(&state, &auth.user_id, &id).await?;
    let active = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM connection_sessions WHERE user_id = $1 \
         AND (requester_device_id = $2 OR host_device_id = $3) \
         AND state IN ('pending', 'accepted')",
    )
    .bind(&auth.user_id)
    .bind(&id)
    .bind(&id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if active > 0 {
        return Err(AppError::Conflict(
            "disconnect active sessions before deleting this device".into(),
        ));
    }
    sqlx::query("DELETE FROM devices WHERE id = $1 AND user_id = $2")
        .bind(&id)
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        None,
        "device.removed",
        serde_json::json!({"deviceId": id}),
    )
    .await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "device.remove",
        Some("device"),
        Some(&id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn fetch_owned(
    state: &AppState,
    user_id: &str,
    id: &str,
) -> Result<Device, AppError> {
    let row = sqlx::query("SELECT * FROM devices WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?;
    row_to_device(row)
}

fn row_to_device(row: sqlx::postgres::PgRow) -> Result<Device, AppError> {
    let codecs_json: String = row.try_get("codecs_json").map_err(AppError::from_db)?;
    let codecs = serde_json::from_str(&codecs_json).map_err(|error| {
        tracing::error!(error = %error, "stored device codecs contain invalid JSON");
        AppError::Internal
    })?;
    Ok(Device {
        id: row.try_get("id").map_err(AppError::from_db)?,
        name: row.try_get("name").map_err(AppError::from_db)?,
        platform: row.try_get("platform").map_err(AppError::from_db)?,
        os_version: row.try_get("os_version").map_err(AppError::from_db)?,
        gpu: row.try_get("gpu").map_err(AppError::from_db)?,
        sanser_version: row.try_get("sanser_version").map_err(AppError::from_db)?,
        protocol_version: row.try_get("protocol_version").map_err(AppError::from_db)?,
        online: row.try_get("online").map_err(AppError::from_db)?,
        streaming: row.try_get("streaming").map_err(AppError::from_db)?,
        pinned: row.try_get("pinned").map_err(AppError::from_db)?,
        route_address: row.try_get("route_address").map_err(AppError::from_db)?,
        network_quality: row.try_get("network_quality").map_err(AppError::from_db)?,
        latency_ms: row.try_get("latency_ms").map_err(AppError::from_db)?,
        codecs,
        native_transport: row.try_get("native_transport").map_err(AppError::from_db)?,
        webrtc: row.try_get("webrtc").map_err(AppError::from_db)?,
        audio: row.try_get("audio").map_err(AppError::from_db)?,
        gamepad: row.try_get("gamepad").map_err(AppError::from_db)?,
        last_seen_at: row.try_get("last_seen_at").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
        updated_at: row.try_get("updated_at").map_err(AppError::from_db)?,
    })
}

fn normalize_codecs(values: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut codecs = BTreeSet::new();
    for value in values {
        let codec = value.trim().to_ascii_lowercase();
        if !matches!(codec.as_str(), "auto" | "h264" | "hevc") {
            return Err(AppError::Validation(format!(
                "unsupported video codec: {value}"
            )));
        }
        codecs.insert(codec);
    }
    if codecs.is_empty() {
        return Err(AppError::Validation(
            "codecs must contain at least one codec".into(),
        ));
    }
    Ok(codecs.into_iter().collect())
}

fn normalize_route(value: Option<String>) -> Result<Option<String>, AppError> {
    value
        .map(|value| {
            value
                .trim()
                .parse::<IpAddr>()
                .map(|address| address.to_string())
                .map_err(|_| AppError::Validation("routeAddress must be an IP address".into()))
        })
        .transpose()
}

fn normalize_quality(value: &str) -> Result<String, AppError> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(
        value.as_str(),
        "excellent" | "good" | "fair" | "poor" | "unknown"
    ) {
        Ok(value)
    } else {
        Err(AppError::Validation(
            "networkQuality must be excellent, good, fair, poor, or unknown".into(),
        ))
    }
}

fn optional_label(value: &str, field: &str, max_length: usize) -> Result<String, AppError> {
    if value.trim().is_empty() {
        Ok(String::new())
    } else {
        clean_label(value, field, max_length)
    }
}

fn validate_uuid(value: &str, field: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation(format!("{field} must be a UUID")))
}

fn encode_cursor(device: &Device) -> Result<String, AppError> {
    let payload = serde_json::to_vec(&DeviceCursor {
        updated_at: device.updated_at,
        id: device.id.clone(),
    })
    .map_err(|_| AppError::Internal)?;
    Ok(URL_SAFE_NO_PAD.encode(payload))
}

fn decode_cursor(value: &str) -> Result<DeviceCursor, AppError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| AppError::Validation("cursor is invalid".into()))?;
    if bytes.len() > 256 {
        return Err(AppError::Validation("cursor is invalid".into()));
    }
    serde_json::from_slice(&bytes).map_err(|_| AppError::Validation("cursor is invalid".into()))
}

fn default_version() -> String {
    SANSER_VERSION.into()
}

const fn default_protocol_version() -> u8 {
    PROTOCOL_VERSION
}

fn default_codecs() -> Vec<String> {
    vec!["auto".into(), "h264".into(), "hevc".into()]
}

const fn default_true() -> bool {
    true
}
