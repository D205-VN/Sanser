//! Guest ("support session") access without an account. See docs/guest-access.md.
//!
//! A host account creates a grant for one of its online host computers and
//! reads the access code and one-time password to the guest. The guest redeems
//! both once; the server then creates a temporary guest device and a pending
//! view-only connection session inside the host account, and returns a guest
//! token that only works for that session and device. The host still has to
//! accept the request.

use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use rand::{Rng, rngs::OsRng};
use sanser_core::NetworkMode;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    auth::{
        ACCESS_PREFIX, AuthContext, audit, clean_label, hash_password, new_token, token_digest,
        verify_password,
    },
    config::PROTOCOL_VERSION,
    error::{ApiJson, AppError},
    events,
    models::{ConnectionSession, Device},
    routes::{devices, sessions},
    state::AppState,
    time::now_unix,
};

pub const GRANT_TTL_SECONDS: i64 = 10 * 60;
pub const MAX_FAILED_ATTEMPTS: i32 = 5;
/// Upper bound on a guest token. The token also ends with the session.
pub const GUEST_TOKEN_TTL_SECONDS: i64 = 4 * 60 * 60;
/// A guest cannot simply ask again, so the host gets longer to answer.
pub const GUEST_PENDING_TTL_SECONDS: i64 = 2 * 60;
const CODE_DIGITS: u32 = 9;
const PASSWORD_LENGTH: usize = 8;
const PASSWORD_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateGrantRequest {
    host_device_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListGrantsQuery {
    host_device_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedGrant {
    id: String,
    host_device_id: String,
    /// Shown once. Formatted as three groups of three digits.
    access_code: String,
    /// Shown once.
    password: String,
    expires_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantSummary {
    id: String,
    host_device_id: String,
    state: &'static str,
    created_at: i64,
    expires_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    redeemed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guest_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedeemRequest {
    access_code: String,
    password: String,
    guest_name: String,
    device: GuestDevice,
    #[serde(default)]
    network_mode: NetworkMode,
}

/// The capabilities a guest computer reports, the same fields a signed-in
/// client sends to `/devices/register`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuestDevice {
    platform: String,
    #[serde(default)]
    cross_platform: bool,
    #[serde(default)]
    os_version: String,
    #[serde(alias = "sanserVersion")]
    version: String,
    protocol_version: u8,
    #[serde(default)]
    codecs: Vec<String>,
    #[serde(default)]
    native_transport: bool,
    #[serde(default, alias = "webrtc")]
    web_rtc: bool,
    #[serde(default)]
    audio: bool,
    #[serde(default)]
    gamepad: bool,
    #[serde(default)]
    route_address: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedeemResponse {
    token_type: &'static str,
    access_token: String,
    access_expires_at: i64,
    device_id: String,
    host_name: String,
    session: ConnectionSession,
}

/// The peer address as the server sees it (a proxy address behind a proxy).
pub struct ClientAddress(String);

impl<S: Send + Sync> FromRequestParts<S> for ClientAddress {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map_or_else(|| "local".to_owned(), |address| address.0.ip().to_string()),
        ))
    }
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<CreateGrantRequest>,
) -> Result<Response, AppError> {
    let host_id = sessions::validate_uuid(&request.host_device_id, "hostDeviceId")?;
    let host = devices::fetch_owned(&state, &auth.user_id, &host_id).await?;
    if host.device_role != "host" {
        return Err(AppError::Conflict(
            "guest access codes can only be created for a host computer".into(),
        ));
    }
    sessions::ensure_device_online(&state, &host)?;
    let password = Zeroizing::new(generate_password());
    let password_hash = hash_password(password.to_string()).await?;
    debug_assert_eq!(normalize_password(&password).as_str(), password.as_str());
    let now = now_unix();
    let expires_at = now + GRANT_TTL_SECONDS;
    let id = Uuid::new_v4().to_string();
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    // One live code per host: a new code replaces any unused one.
    sqlx::query(
        "UPDATE guest_access_grants SET revoked_at = $1 \
         WHERE user_id = $2 AND host_device_id = $3 AND redeemed_at IS NULL AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&auth.user_id)
    .bind(&host_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    let mut code = None;
    for _ in 0..4 {
        let candidate = Zeroizing::new(generate_code());
        let inserted = sqlx::query(
            "INSERT INTO guest_access_grants \
             (id, user_id, host_device_id, code_digest, password_hash, failed_attempts, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, 0, $6, $7) ON CONFLICT (code_digest) DO NOTHING",
        )
        .bind(&id)
        .bind(&auth.user_id)
        .bind(&host_id)
        .bind(code_digest(&state, &candidate)?)
        .bind(&password_hash)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(AppError::from_db)?;
        if inserted.rows_affected() == 1 {
            code = Some(candidate);
            break;
        }
    }
    let code = code.ok_or(AppError::Unavailable)?;
    tx.commit().await.map_err(AppError::from_db)?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "guest.grant.create",
        Some("guest_access_grant"),
        Some(&id),
        serde_json::json!({"hostDeviceId": host_id}),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedGrant {
            id,
            host_device_id: host_id,
            access_code: format!("{} {} {}", &code[0..3], &code[3..6], &code[6..9]),
            password: password.to_string(),
            expires_at,
        }),
    )
        .into_response())
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(query): Query<ListGrantsQuery>,
) -> Result<Json<Vec<GrantSummary>>, AppError> {
    let host_id = sessions::validate_uuid(&query.host_device_id, "hostDeviceId")?;
    devices::fetch_owned(&state, &auth.user_id, &host_id).await?;
    let now = now_unix();
    let rows = sqlx::query(
        "SELECT g.id, g.host_device_id, g.created_at, g.expires_at, g.redeemed_at, g.session_id, g.guest_name, \
                s.state AS session_state \
         FROM guest_access_grants g \
         LEFT JOIN connection_sessions s ON s.id = g.session_id AND s.user_id = g.user_id \
         WHERE g.user_id = $1 AND g.host_device_id = $2 AND g.revoked_at IS NULL \
           AND ((g.redeemed_at IS NULL AND g.expires_at > $3) OR s.state IN ('pending', 'accepted')) \
         ORDER BY g.created_at DESC LIMIT 10",
    )
    .bind(&auth.user_id)
    .bind(&host_id)
    .bind(now)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    rows.into_iter()
        .map(|row| {
            let redeemed_at: Option<i64> = row.try_get("redeemed_at").map_err(AppError::from_db)?;
            Ok(GrantSummary {
                id: row.try_get("id").map_err(AppError::from_db)?,
                host_device_id: row.try_get("host_device_id").map_err(AppError::from_db)?,
                state: if redeemed_at.is_some() {
                    "redeemed"
                } else {
                    "waiting"
                },
                created_at: row.try_get("created_at").map_err(AppError::from_db)?,
                expires_at: row.try_get("expires_at").map_err(AppError::from_db)?,
                redeemed_at,
                session_id: row.try_get("session_id").map_err(AppError::from_db)?,
                guest_name: row.try_get("guest_name").map_err(AppError::from_db)?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()
        .map(Json)
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let id = sessions::validate_uuid(&id, "id")?;
    let session_id = sqlx::query_scalar::<_, Option<String>>(
        "SELECT session_id FROM guest_access_grants WHERE id = $1 AND user_id = $2",
    )
    .bind(&id)
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    if let Some(session_id) = session_id {
        let session = sessions::fetch_owned(&state, &auth.user_id, &session_id).await?;
        if matches!(session.state.as_str(), "pending" | "accepted") {
            let ended = sessions::transition_to_ended(
                &state,
                &auth,
                &session,
                "disconnected",
                Some("guest_access_revoked".into()),
            )
            .await;
            // A concurrent end is fine: the grant is revoked below either way.
            match ended {
                Ok(()) | Err(AppError::Conflict(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    end_guest_access(&mut tx, &auth.user_id, &id, now_unix()).await?;
    tx.commit().await.map_err(AppError::from_db)?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "guest.grant.revoke",
        Some("guest_access_grant"),
        Some(&id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn redeem(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    ApiJson(request): ApiJson<RedeemRequest>,
) -> Result<Response, AppError> {
    if !state
        .guest_redeem_global_limiter
        .check("guest-redeem")
        .await
        || !state
            .guest_redeem_limiter
            .check(format!("guest-redeem:{address}"))
            .await
    {
        return Err(AppError::RateLimited);
    }
    let password = normalize_password(&Zeroizing::new(request.password));
    let Some(code) = normalize_code(&request.access_code) else {
        burn_password(password.to_string()).await?;
        return Err(rejected());
    };
    let guest_name = clean_label(&request.guest_name, "guestName", 60)?;
    let now = now_unix();
    let digest = code_digest(&state, &code)?;

    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    let grant = sqlx::query(
        "SELECT id, user_id, host_device_id, password_hash, failed_attempts, expires_at, redeemed_at, revoked_at \
         FROM guest_access_grants WHERE code_digest = $1 FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    let Some(grant) = grant else {
        drop(tx);
        burn_password(password.to_string()).await?;
        return Err(rejected());
    };
    let grant_id: String = grant.try_get("id").map_err(AppError::from_db)?;
    let user_id: String = grant.try_get("user_id").map_err(AppError::from_db)?;
    let host_id: String = grant.try_get("host_device_id").map_err(AppError::from_db)?;
    let failed: i32 = grant
        .try_get("failed_attempts")
        .map_err(AppError::from_db)?;
    let expires_at: i64 = grant.try_get("expires_at").map_err(AppError::from_db)?;
    let redeemed: Option<i64> = grant.try_get("redeemed_at").map_err(AppError::from_db)?;
    let revoked: Option<i64> = grant.try_get("revoked_at").map_err(AppError::from_db)?;
    let password_hash: String = grant.try_get("password_hash").map_err(AppError::from_db)?;
    if redeemed.is_some() || revoked.is_some() || expires_at <= now || failed >= MAX_FAILED_ATTEMPTS
    {
        drop(tx);
        burn_password(password.to_string()).await?;
        audit_rejection(&state, &user_id, &grant_id, "unusable").await;
        return Err(rejected());
    }
    if !verify_password(password.to_string(), password_hash).await? {
        // Too many wrong passwords burn the code for everyone.
        sqlx::query(
            "UPDATE guest_access_grants SET failed_attempts = failed_attempts + 1, \
             revoked_at = CASE WHEN failed_attempts + 1 >= $1 THEN $2 ELSE revoked_at END \
             WHERE id = $3",
        )
        .bind(MAX_FAILED_ATTEMPTS)
        .bind(now)
        .bind(&grant_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::from_db)?;
        tx.commit().await.map_err(AppError::from_db)?;
        audit_rejection(&state, &user_id, &grant_id, "password").await;
        return Err(rejected());
    }

    // Credentials are right. From here a failure leaves the grant unused so
    // the guest can retry once the host is ready.
    let host = devices::fetch_owned(&state, &user_id, &host_id).await?;
    sessions::ensure_device_online(&state, &host).map_err(|_| {
        AppError::Conflict("The host computer is offline. Ask the host to bring it online.".into())
    })?;
    let guest_device_id = Uuid::new_v4().to_string();
    let guest = validated_guest_device(&request.device, &guest_device_id, &guest_name, now)?;
    if !sessions::supports_view_only(&host.sanser_version) {
        return Err(AppError::Conflict(
            "The host computer needs a newer Sanser version for guest access.".into(),
        ));
    }
    sessions::select_transport(None, request.network_mode, &host, &guest)?;
    host_is_free(&mut tx, &user_id, &host_id).await?;

    sqlx::query(
        "INSERT INTO devices \
         (id, user_id, name, platform, os_version, gpu, sanser_version, protocol_version, \
          online, streaming, pinned, route_address, network_quality, latency_ms, codecs_json, \
          native_transport, webrtc, audio, gamepad, last_seen_at, created_at, updated_at, \
          device_role, cross_platform, computer_id, guest_grant_id) \
         VALUES ($1, $2, $3, $4, $5, 'Not reported', $6, $7, TRUE, FALSE, FALSE, $8, NULL, NULL, \
                 $9, $10, $11, $12, $13, $14, $14, $14, 'client', $15, NULL, $16)",
    )
    .bind(&guest.id)
    .bind(&user_id)
    .bind(&guest.name)
    .bind(&guest.platform)
    .bind(&guest.os_version)
    .bind(&guest.sanser_version)
    .bind(i64::from(PROTOCOL_VERSION))
    .bind(&guest.route_address)
    .bind(serde_json::to_string(&guest.codecs).map_err(|_| AppError::Internal)?)
    .bind(guest.native_transport)
    .bind(guest.webrtc)
    .bind(guest.audio)
    .bind(guest.gamepad)
    .bind(now)
    .bind(guest.cross_platform)
    .bind(&grant_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    let session_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO connection_sessions \
         (id, user_id, requester_device_id, host_device_id, state, network_mode, quality_profile, \
          requested_codec, selected_transport, created_at, updated_at, accepted_at, ended_at, \
          disconnect_reason, access_mode, guest_grant_id, guest_name) \
         VALUES ($1, $2, $3, $4, 'pending', $5, 'auto', 'auto', NULL, $6, $6, NULL, NULL, NULL, \
                 'view-only', $7, $8)",
    )
    .bind(&session_id)
    .bind(&user_id)
    .bind(&guest.id)
    .bind(&host_id)
    .bind(sessions::network_mode_str(request.network_mode))
    .bind(now)
    .bind(&grant_id)
    .bind(&guest_name)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    let used = sqlx::query(
        "UPDATE guest_access_grants SET redeemed_at = $1, session_id = $2, guest_device_id = $3, guest_name = $4 \
         WHERE id = $5 AND redeemed_at IS NULL AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&session_id)
    .bind(&guest.id)
    .bind(&guest_name)
    .bind(&grant_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    if used.rows_affected() != 1 {
        return Err(rejected());
    }
    let auth_session_id = Uuid::new_v4().to_string();
    let access_token = new_token(ACCESS_PREFIX);
    let access_expires_at = now + GUEST_TOKEN_TTL_SECONDS;
    sqlx::query(
        "INSERT INTO auth_sessions \
         (id, user_id, device_name, platform, created_at, last_seen_at, expires_at, revoked_at, \
          installation_id, scope, guest_grant_id) \
         VALUES ($1, $2, $3, $4, $5, $5, $6, NULL, NULL, 'guest', $7)",
    )
    .bind(&auth_session_id)
    .bind(&user_id)
    .bind(&guest.name)
    .bind(&guest.platform)
    .bind(now)
    .bind(access_expires_at)
    .bind(&grant_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "INSERT INTO access_tokens (digest, auth_session_id, created_at, expires_at, revoked_at) \
         VALUES ($1, $2, $3, $4, NULL)",
    )
    .bind(token_digest(&access_token))
    .bind(&auth_session_id)
    .bind(now)
    .bind(access_expires_at)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    tx.commit().await.map_err(AppError::from_db)?;

    let session = sessions::fetch_owned(&state, &user_id, &session_id).await?;
    events::publish(
        &state,
        &user_id,
        Some(&session_id),
        "session.requested",
        serde_json::json!({
            "sessionId": session_id,
            "requesterDeviceId": guest.id,
            "hostDeviceId": host_id,
            "guest": true
        }),
    )
    .await?;
    audit(
        &state.pool,
        Some(&user_id),
        "guest.grant.redeem",
        Some("guest_access_grant"),
        Some(&grant_id),
        serde_json::json!({"sessionId": session_id, "platform": guest.platform}),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(RedeemResponse {
            token_type: "Bearer",
            access_token,
            access_expires_at,
            device_id: guest.id,
            host_name: host.name,
            session,
        }),
    )
        .into_response())
}

/// Ends a guest's access: the grant can no longer authenticate, every guest
/// token for it is revoked and the guest device goes offline.
pub(crate) async fn end_guest_access(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    grant_id: &str,
    now: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE guest_access_grants SET revoked_at = COALESCE(revoked_at, $1) WHERE id = $2 AND user_id = $3",
    )
    .bind(now)
    .bind(grant_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE access_tokens SET revoked_at = $1 WHERE revoked_at IS NULL AND auth_session_id IN \
         (SELECT id FROM auth_sessions WHERE user_id = $2 AND scope = 'guest' AND guest_grant_id = $3)",
    )
    .bind(now)
    .bind(user_id)
    .bind(grant_id)
    .execute(&mut **tx)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = $1 \
         WHERE user_id = $2 AND scope = 'guest' AND guest_grant_id = $3 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .bind(grant_id)
    .execute(&mut **tx)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE devices SET online = FALSE, streaming = FALSE, updated_at = $1 \
         WHERE user_id = $2 AND guest_grant_id = $3",
    )
    .bind(now)
    .bind(user_id)
    .bind(grant_id)
    .execute(&mut **tx)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}

/// Periodic upkeep: expire unanswered guest requests, end guest access whose
/// session is over, and drop old grants with their guest devices.
pub async fn cleanup(pool: &sqlx::PgPool, now: i64) {
    let statements = [
        sqlx::query(
            "UPDATE connection_sessions SET state = 'expired', ended_at = $1, updated_at = $1 \
             WHERE state = 'pending' AND guest_grant_id IS NOT NULL AND created_at < $2",
        )
        .bind(now)
        .bind(now - GUEST_PENDING_TTL_SECONDS)
        .execute(pool)
        .await,
        sqlx::query(
            "UPDATE guest_access_grants AS g SET revoked_at = $1 WHERE g.revoked_at IS NULL AND ( \
               (g.redeemed_at IS NULL AND g.expires_at < $1) OR \
               (g.session_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM connection_sessions s \
                  WHERE s.id = g.session_id AND s.state IN ('pending', 'accepted'))))",
        )
        .bind(now)
        .execute(pool)
        .await,
        sqlx::query(
            "UPDATE access_tokens SET revoked_at = $1 WHERE revoked_at IS NULL AND auth_session_id IN \
             (SELECT s.id FROM auth_sessions s JOIN guest_access_grants g ON g.id = s.guest_grant_id \
              WHERE s.scope = 'guest' AND g.revoked_at IS NOT NULL)",
        )
        .bind(now)
        .execute(pool)
        .await,
        sqlx::query(
            "UPDATE auth_sessions AS s SET revoked_at = $1 WHERE s.scope = 'guest' AND s.revoked_at IS NULL \
             AND EXISTS (SELECT 1 FROM guest_access_grants g WHERE g.id = s.guest_grant_id AND g.revoked_at IS NOT NULL)",
        )
        .bind(now)
        .execute(pool)
        .await,
        sqlx::query(
            "UPDATE devices AS d SET online = FALSE, streaming = FALSE, updated_at = $1 \
             WHERE d.guest_grant_id IS NOT NULL AND d.online = TRUE AND EXISTS \
             (SELECT 1 FROM guest_access_grants g WHERE g.id = d.guest_grant_id AND g.revoked_at IS NOT NULL)",
        )
        .bind(now)
        .execute(pool)
        .await,
        // Same retention as ended connection sessions (30 days).
        sqlx::query(
            "DELETE FROM devices WHERE guest_grant_id IS NOT NULL AND updated_at < $1 \
             AND NOT EXISTS (SELECT 1 FROM connection_sessions s WHERE s.requester_device_id = devices.id \
                 AND s.state IN ('pending', 'accepted'))",
        )
        .bind(now - 30 * 86_400)
        .execute(pool)
        .await,
        sqlx::query("DELETE FROM guest_access_grants WHERE created_at < $1 AND revoked_at IS NOT NULL")
            .bind(now - 30 * 86_400)
            .execute(pool)
            .await,
    ];
    for result in statements {
        if let Err(error) = result {
            tracing::warn!(error = %error, "guest access cleanup failed");
        }
    }
}

async fn host_is_free(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    host_id: &str,
) -> Result<(), AppError> {
    sqlx::query("SELECT id FROM devices WHERE id = $1 AND user_id = $2 FOR NO KEY UPDATE")
        .bind(host_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::from_db)?;
    let busy = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM connection_sessions WHERE user_id = $1 AND host_device_id = $2 \
                       AND state IN ('pending', 'accepted')) \
             OR EXISTS(SELECT 1 FROM sharing_groups WHERE host_device_id = $2 AND state IN ('preparing', 'active'))",
    )
    .bind(user_id)
    .bind(host_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(AppError::from_db)?;
    if busy {
        return Err(AppError::Conflict(
            "The host computer is busy with another session. Try again when it is free.".into(),
        ));
    }
    Ok(())
}

fn validated_guest_device(
    device: &GuestDevice,
    id: &str,
    guest_name: &str,
    now: i64,
) -> Result<Device, AppError> {
    let platform = clean_label(&device.platform, "platform", 40)?;
    if !devices::compatible_client_version(&device.version) {
        return Err(AppError::Conflict(
            "Update Sanser on this computer to connect.".into(),
        ));
    }
    if device.protocol_version != PROTOCOL_VERSION {
        return Err(AppError::Conflict(
            "Update Sanser on this computer to connect.".into(),
        ));
    }
    Ok(Device {
        id: id.to_owned(),
        computer_id: None,
        name: format!("{guest_name} (guest)"),
        platform,
        device_role: "client".into(),
        cross_platform: device.cross_platform,
        os_version: devices::optional_label(&device.os_version, "osVersion", 120)?,
        gpu: "Not reported".into(),
        sanser_version: device.version.clone(),
        protocol_version: i64::from(PROTOCOL_VERSION),
        online: true,
        streaming: false,
        pinned: false,
        route_address: devices::normalize_route(device.route_address.clone())?,
        network_quality: None,
        latency_ms: None,
        codecs: devices::normalize_codecs(if device.codecs.is_empty() {
            vec!["auto".into()]
        } else {
            device.codecs.clone()
        })?,
        native_transport: device.native_transport,
        webrtc: device.web_rtc,
        audio: device.audio,
        gamepad: device.gamepad,
        last_seen_at: Some(now),
        created_at: now,
        updated_at: now,
    })
}

/// One response for every failed redemption so codes cannot be probed.
fn rejected() -> AppError {
    AppError::Validation(
        "The access code or password is incorrect, or the code has expired.".into(),
    )
}

async fn audit_rejection(state: &AppState, user_id: &str, grant_id: &str, reason: &str) {
    audit(
        &state.pool,
        Some(user_id),
        "guest.grant.redeem_failed",
        Some("guest_access_grant"),
        Some(grant_id),
        serde_json::json!({"reason": reason}),
    )
    .await;
}

async fn burn_password(password: String) -> Result<(), AppError> {
    const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$N2ZqRVJXY0xYZ0JYaFhnTQ$Y74gYub18WnWQ4EFVgyB7yxGyehPV20xuQiyPcYrt88";
    let _ = verify_password(password, DUMMY_HASH.into()).await?;
    Ok(())
}

fn code_digest(state: &AppState, code: &str) -> Result<String, AppError> {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(state.config.session_credential_key.as_slice())
            .map_err(|_| AppError::Internal)?;
    mac.update(b"sanser/guest-access-code/v1\0");
    mac.update(code.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// The password is shown in upper case; accept it typed in either case.
fn normalize_password(value: &str) -> Zeroizing<String> {
    Zeroizing::new(
        value
            .chars()
            .filter(|character| !character.is_whitespace())
            .map(|character| character.to_ascii_uppercase())
            .collect(),
    )
}

fn normalize_code(value: &str) -> Option<String> {
    let digits: String = value
        .chars()
        .filter(|character| !matches!(character, ' ' | '-'))
        .collect();
    (digits.len() == CODE_DIGITS as usize && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(digits)
}

fn generate_code() -> String {
    let value = OsRng.gen_range(0..10_u32.pow(CODE_DIGITS));
    format!("{value:0width$}", width = CODE_DIGITS as usize)
}

fn generate_password() -> String {
    (0..PASSWORD_LENGTH)
        .map(|_| char::from(PASSWORD_ALPHABET[OsRng.gen_range(0..PASSWORD_ALPHABET.len())]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_passwords_have_the_documented_shape() {
        for _ in 0..200 {
            let code = generate_code();
            assert_eq!(code.len(), 9);
            assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
            let password = generate_password();
            assert_eq!(password.len(), 8);
            assert!(
                password
                    .bytes()
                    .all(|byte| PASSWORD_ALPHABET.contains(&byte))
            );
        }
        assert_eq!(normalize_code("123 456-789").as_deref(), Some("123456789"));
        assert!(normalize_code("12345678").is_none());
        assert!(normalize_code("12345678a").is_none());
        assert!(normalize_code("1234567890").is_none());
    }
}
