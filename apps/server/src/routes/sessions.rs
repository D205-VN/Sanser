use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sanser_core::NetworkMode;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{AuthContext, audit},
    error::{ApiJson, AppError},
    events,
    models::{ConnectionSession, Device, Page},
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeCredentialsQuery {
    device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeReadyRequest {
    device_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListSessionsQuery {
    host_device_id: String,
    #[serde(default = "default_active_state")]
    state: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeSessionCredentials {
    session_id: String,
    device_id: String,
    peer_device_id: String,
    peer_route_address: String,
    base_port: u16,
    expires_at: i64,
    session_token: String,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<Page<ConnectionSession>>, AppError> {
    let host_id = validate_uuid(&query.host_device_id, "hostDeviceId")?;
    // Confirm ownership before querying session rows. This deliberately returns
    // the same not-found response as the rest of the device API for another
    // account's device identifier.
    devices::fetch_owned(&state, &auth.user_id, &host_id).await?;
    let requested_state = query.state.trim().to_ascii_lowercase();
    let rows = match requested_state.as_str() {
        "active" => sqlx::query(
            "SELECT * FROM connection_sessions WHERE user_id = $1 AND host_device_id = $2 \
                 AND state IN ('pending', 'accepted') ORDER BY created_at DESC, id DESC LIMIT 20",
        )
        .bind(&auth.user_id)
        .bind(&host_id)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::from_db)?,
        "pending" | "accepted" | "rejected" | "disconnected" | "expired" => sqlx::query(
            "SELECT * FROM connection_sessions WHERE user_id = $1 AND host_device_id = $2 \
                 AND state = $3 ORDER BY created_at DESC, id DESC LIMIT 20",
        )
        .bind(&auth.user_id)
        .bind(&host_id)
        .bind(&requested_state)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::from_db)?,
        _ => {
            return Err(AppError::Validation(
                "state must be active, pending, accepted, rejected, disconnected, or expired"
                    .into(),
            ));
        }
    };
    let items = rows
        .into_iter()
        .map(row_to_session)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(Page {
        items,
        next_cursor: None,
    }))
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
    // Relay checks removed as TURN is deprecated.
    // Validate the pair now so the UI never creates a request that can only
    // fail later when the host attempts to accept it.
    select_transport(None, request.network_mode, &host, &requester)?;

    // Serialize session creation for both devices. The previous COUNT followed
    // by an independent INSERT allowed two concurrent requests to create
    // overlapping active sessions before either row became visible.
    let mut transaction = state.pool.begin().await.map_err(AppError::from_db)?;
    let locked_devices = sqlx::query_scalar::<_, String>(
        "SELECT id FROM devices WHERE user_id = $1 AND id IN ($2, $3) \
         ORDER BY id FOR UPDATE",
    )
    .bind(&auth.user_id)
    .bind(&requester_id)
    .bind(&host_id)
    .fetch_all(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    if locked_devices.len() != 2 {
        return Err(AppError::Conflict(
            "one of the session devices changed while the request was being created".into(),
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
    .fetch_one(&mut *transaction)
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
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    transaction.commit().await.map_err(AppError::from_db)?;
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

pub async fn credentials(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    Query(query): Query<NativeCredentialsQuery>,
) -> Result<Response, AppError> {
    let id = validate_uuid(&id, "id")?;
    let device_id = validate_uuid(&query.device_id, "deviceId")?;
    let session = fetch_owned(&state, &auth.user_id, &id).await?;
    if session.state != "accepted" {
        return Err(AppError::Conflict(
            "native credentials require an accepted session".into(),
        ));
    }
    if session.selected_transport.as_deref() != Some("native") {
        return Err(AppError::Conflict(
            "native credentials require the authenticated native transport".into(),
        ));
    }

    let peer_device_id = if device_id == session.requester_device_id {
        session.host_device_id.clone()
    } else if device_id == session.host_device_id {
        session.requester_device_id.clone()
    } else {
        return Err(AppError::Forbidden);
    };
    let device = devices::fetch_owned(&state, &auth.user_id, &device_id).await?;
    let peer = devices::fetch_owned(&state, &auth.user_id, &peer_device_id).await?;
    if !device.native_transport || !peer.native_transport {
        return Err(AppError::Conflict(
            "both session devices must support the native transport".into(),
        ));
    }
    ensure_device_online(&state, &device)?;
    ensure_device_online(&state, &peer)?;
    // Candidate gathering selects the actual direct endpoint, while relay mode
    // targets a local loopback bridge. Keep this compatibility field bounded
    // even when a relay-capable peer has no advertisable LAN address.
    let peer_route_address = peer.route_address.unwrap_or_else(|| "127.0.0.1".into());
    let credential_epoch = session.requester_ready_at.ok_or_else(|| {
        AppError::Conflict(
            "requester must announce native readiness before credentials are issued".into(),
        )
    })?;
    let expires_at = credential_epoch
        .checked_add(
            i64::try_from(state.config.session_credential_ttl.as_secs())
                .map_err(|_| AppError::Internal)?,
        )
        .ok_or(AppError::Internal)?;
    if expires_at <= now_unix() {
        return Err(AppError::Conflict(
            "native session credential has expired; announce readiness again".into(),
        ));
    }
    let session_token = derive_session_token(
        state.config.session_credential_key.as_slice(),
        &session.id,
        &session.requester_device_id,
        &session.host_device_id,
        credential_epoch,
        expires_at,
    )?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "session.native_credentials.issue",
        Some("connection_session"),
        Some(&session.id),
        serde_json::json!({
            "deviceId": device_id,
            "peerDeviceId": peer_device_id,
            "expiresAt": expires_at
        }),
    )
    .await;

    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(NativeSessionCredentials {
            session_id: session.id,
            device_id,
            peer_device_id,
            peer_route_address,
            base_port: state.config.native_base_port,
            expires_at,
            session_token,
        }),
    )
        .into_response())
}

pub async fn native_ready(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<NativeReadyRequest>,
) -> Result<Json<ConnectionSession>, AppError> {
    let id = validate_uuid(&id, "id")?;
    let device_id = validate_uuid(&request.device_id, "deviceId")?;
    let current = fetch_owned(&state, &auth.user_id, &id).await?;
    if current.state != "accepted" {
        return Err(AppError::Conflict(format!(
            "native readiness requires an accepted session; current state is {}",
            current.state
        )));
    }
    if current.selected_transport.as_deref() != Some("native") {
        return Err(AppError::Conflict(
            "native readiness requires the selected native transport".into(),
        ));
    }
    if device_id != current.requester_device_id {
        return Err(AppError::Forbidden);
    }
    let requester = devices::fetch_owned(&state, &auth.user_id, &device_id).await?;
    let host = devices::fetch_owned(&state, &auth.user_id, &current.host_device_id).await?;
    ensure_device_online(&state, &requester)?;
    ensure_device_online(&state, &host)?;

    let now = now_unix();
    // Recheck every session invariant in the write itself. A concurrent
    // transition then affects zero rows and is reported as a conflict below.
    let updated = sqlx::query(
        "UPDATE connection_sessions SET requester_ready_at = \
         GREATEST(COALESCE(requester_ready_at + 1, $1), $1), \
         updated_at = $2 WHERE id = $3 AND user_id = $4 AND requester_device_id = $5 \
         AND state = 'accepted' AND selected_transport = 'native'",
    )
    .bind(now)
    .bind(now)
    .bind(&id)
    .bind(&auth.user_id)
    .bind(&device_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if updated.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "session state changed before native readiness was recorded".into(),
        ));
    }
    events::publish(
        &state,
        &auth.user_id,
        Some(&id),
        "session.native_ready",
        serde_json::json!({"sessionId": id, "requesterDeviceId": device_id}),
    )
    .await?;
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
    sqlx::query(
        "UPDATE devices SET streaming = TRUE, updated_at = $1 WHERE id = $2 AND user_id = $3",
    )
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
    sqlx::query(
        "UPDATE devices SET streaming = FALSE, updated_at = $1 WHERE id = $2 AND user_id = $3",
    )
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

fn row_to_session(row: sqlx::postgres::PgRow) -> Result<ConnectionSession, AppError> {
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
        requester_ready_at: row
            .try_get("requester_ready_at")
            .map_err(AppError::from_db)?,
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
        "SELECT id FROM devices WHERE user_id = $1 AND id != $2 AND online = TRUE \
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
            if mode != NetworkMode::Manual && host.native_transport && requester.native_transport {
                "native"
            } else {
                "webrtc"
            }
        },
        str::trim,
    );
    match selected {
        "native"
            if mode != NetworkMode::Manual
                && host.native_transport
                && requester.native_transport =>
        {
            Ok(selected.into())
        }
        "webrtc" if host.webrtc && requester.webrtc => Ok(selected.into()),
        "native" | "webrtc" => Err(AppError::Conflict(
            "selected transport is not supported by both devices".into(),
        )),
        _ => Err(AppError::Validation(
            "selectedTransport must be native or webrtc".into(),
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
        "direct" | "directonly" => Ok(NetworkMode::DirectOnly),
        "manual" => Ok(NetworkMode::Manual),
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
        NetworkMode::DirectOnly => "direct",
        NetworkMode::Relay => "relay",
        NetworkMode::Manual => "manual",
    }
}

fn validate_uuid(value: &str, field: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation(format!("{field} must be a UUID")))
}

fn derive_session_token(
    key: &[u8],
    session_id: &str,
    requester_device_id: &str,
    host_device_id: &str,
    credential_epoch: i64,
    expires_at: i64,
) -> Result<String, AppError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| AppError::Internal)?;
    mac.update(b"sanser-native-session-credential-v1\0");
    for field in [session_id, requester_device_id, host_device_id] {
        let length = u32::try_from(field.len()).map_err(|_| AppError::Internal)?;
        mac.update(&length.to_be_bytes());
        mac.update(field.as_bytes());
    }
    mac.update(&credential_epoch.to_be_bytes());
    mac.update(&expires_at.to_be_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn default_quality() -> String {
    "auto".into()
}

fn default_codec() -> String {
    "auto".into()
}

fn default_active_state() -> String {
    "active".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_mode_uses_the_database_and_desktop_wire_value() {
        assert_eq!(
            parse_network_mode("direct").expect("direct network mode"),
            NetworkMode::DirectOnly
        );
        assert_eq!(network_mode_str(NetworkMode::DirectOnly), "direct");
    }

    #[test]
    fn native_session_token_is_stable_bound_and_opaque() {
        let key = [0xA5; 32];
        let session = "018f4d89-5e8b-7a80-bd1e-cb7cb9f43189";
        let requester = "018f4d89-5e8b-7a80-bd1e-cb7cb9f43190";
        let host = "018f4d89-5e8b-7a80-bd1e-cb7cb9f43191";
        let token = derive_session_token(&key, session, requester, host, 1_000, 1_060)
            .expect("session token");
        assert_eq!(token.len(), 43);
        assert_eq!(
            token,
            derive_session_token(&key, session, requester, host, 1_000, 1_060)
                .expect("same session token")
        );
        assert_ne!(
            token,
            derive_session_token(&key, session, requester, host, 1_000, 1_061)
                .expect("expiry-bound token")
        );
        assert_ne!(
            token,
            derive_session_token(&key, session, host, requester, 1_000, 1_060)
                .expect("device-bound token")
        );
        assert!(!token.contains(session));
        assert!(!token.contains(requester));
        assert!(!token.contains(host));
    }
}
