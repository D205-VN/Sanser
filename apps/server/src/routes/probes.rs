//! Live measurements before the stream advisor answers. See
//! `crate::advisor_probe` and docs/stream-advisor.md.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    advisor_probe::{CHUNK_BYTES, HostMeasurement, NOISE, Transfer},
    auth::AuthContext,
    error::{ApiJson, AppError},
    routes::sessions::validate_uuid,
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProbe {
    host_device_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCreated {
    probe_id: String,
    expires_in_seconds: u64,
    chunk_bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingQuery {
    device_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProbes {
    items: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeState {
    host_answered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<HostMeasurement>,
}

/// Viewer: opens a probe for one of its own online hosts.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<CreateProbe>,
) -> Result<(StatusCode, Json<ProbeCreated>), AppError> {
    let host = validate_uuid(&request.host_device_id, "hostDeviceId")?;
    if !state
        .advisor_probe_limiter
        .check(auth.user_id.clone())
        .await
    {
        return Err(AppError::RateLimited);
    }
    let online: bool = sqlx::query("SELECT online FROM devices WHERE id = $1 AND user_id = $2")
        .bind(&host)
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?
        .try_get("online")
        .map_err(AppError::from_db)?;
    if !online {
        return Err(AppError::Conflict("the host computer is offline".into()));
    }
    let probe_id = state
        .advisor_probes
        .create(&auth.user_id, &host)
        .await
        .ok_or(AppError::Unavailable)?;
    Ok((
        StatusCode::CREATED,
        Json(ProbeCreated {
            probe_id,
            expires_in_seconds: crate::advisor_probe::PROBE_TTL.as_secs(),
            chunk_bytes: CHUNK_BYTES,
        }),
    ))
}

/// Host: probes waiting for this computer. Memory only, cheap to poll.
pub async fn pending(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(query): Query<PendingQuery>,
) -> Result<Json<PendingProbes>, AppError> {
    let device = validate_uuid(&query.device_id, "deviceId")?;
    Ok(Json(PendingProbes {
        items: state.advisor_probes.claim(&auth.user_id, &device).await,
    }))
}

/// Host: one timed upload chunk. The bytes are counted and dropped.
pub async fn upload(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(probe): Path<String>,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    let probe = validate_uuid(&probe, "probeId")?;
    if body.len() > CHUNK_BYTES {
        return Err(AppError::PayloadTooLarge);
    }
    match state
        .advisor_probes
        .transfer(&probe, &auth.user_id, true)
        .await
    {
        Transfer::Allowed => Ok(StatusCode::NO_CONTENT),
        Transfer::Exhausted => Err(AppError::RateLimited),
        Transfer::Missing => Err(AppError::NotFound),
    }
}

/// Viewer: one timed download chunk of incompressible bytes.
pub async fn download(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(probe): Path<String>,
) -> Result<Response, AppError> {
    let probe = validate_uuid(&probe, "probeId")?;
    match state
        .advisor_probes
        .transfer(&probe, &auth.user_id, false)
        .await
    {
        Transfer::Allowed => {}
        Transfer::Exhausted => return Err(AppError::RateLimited),
        Transfer::Missing => return Err(AppError::NotFound),
    }
    let bytes: &'static [u8] = NOISE.as_slice();
    let mut response = Bytes::from_static(bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Host: what it measured and knows about itself.
pub async fn finish(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(probe): Path<String>,
    ApiJson(result): ApiJson<HostMeasurement>,
) -> Result<StatusCode, AppError> {
    let probe = validate_uuid(&probe, "probeId")?;
    if state
        .advisor_probes
        .finish(&probe, &auth.user_id, result)
        .await
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound)
    }
}

/// Viewer: whether the host has answered yet.
pub async fn show(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(probe): Path<String>,
) -> Result<Json<ProbeState>, AppError> {
    let probe = validate_uuid(&probe, "probeId")?;
    let host = state
        .advisor_probes
        .result(&probe, &auth.user_id)
        .await
        .ok_or(AppError::NotFound)?;
    Ok(Json(ProbeState {
        host_answered: host.is_some(),
        host,
    }))
}
