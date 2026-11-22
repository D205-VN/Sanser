//! "Forgot password": a 6-digit code emailed to the account address lets the
//! person choose a new password. See docs/email-verification.md.
//!
//! The answer to a request never reveals whether the address has an account:
//! an unknown address gets a decoy challenge that sends no email and accepts
//! no code. Setting the new password signs out every other computer and
//! phone, and signs this one in.

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{
        audit, clean_label, create_login_session, hash_password, new_token, normalize_email,
        revoke_all_user_sessions, token_digest, valid_token_shape, validate_password,
    },
    error::{ApiJson, AppError},
    models::AuthTokens,
    routes::email_verification::{
        CHALLENGE_MAX_ATTEMPTS, CHALLENGE_PREFIX, CHALLENGE_TTL_SECONDS, Origin, code_mac,
        code_matches, deliver, email_hint, expired, invalid_code, load_account, new_code,
        normalize_code, trust_device,
    },
    state::AppState,
    time::now_unix,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgotRequest {
    email: String,
    #[serde(default = "default_device_name")]
    device_name: String,
    #[serde(default = "default_platform")]
    platform: String,
    installation_id: Option<Uuid>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgotResponse {
    challenge_token: String,
    challenge_expires_at: i64,
    email_hint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckRequest {
    challenge_token: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetRequest {
    challenge_token: String,
    code: String,
    new_password: String,
}

/// Emails a reset code. 202 with a challenge whether or not the address has
/// an account; a new code is asked for with `/api/v2/auth/email/resend`.
pub async fn forgot(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ForgotRequest>,
) -> Result<(StatusCode, Json<ForgotResponse>), AppError> {
    let email = normalize_email(&request.email)?;
    let device_name = clean_label(&request.device_name, "deviceName", 120)?;
    let platform = clean_label(&request.platform, "platform", 40)?;
    if !state.mailer.enabled() {
        return Err(AppError::Unavailable);
    }
    if !state.email_limiter.check(format!("email:{email}")).await {
        return Err(AppError::RateLimited);
    }
    let user_id = sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    let installation_id = request.installation_id.as_ref().map(Uuid::to_string);
    let token = new_token(CHALLENGE_PREFIX);
    let digest = token_digest(&token);
    let code = new_code();
    let now = now_unix();
    let expires_at = now + CHALLENGE_TTL_SECONDS;
    sqlx::query(
        "INSERT INTO email_challenges \
         (digest, purpose, user_id, email, display_name, password_hash, code_mac, device_name, \
          platform, installation_id, attempts, sends, created_at, last_sent_at, expires_at, consumed_at) \
         VALUES ($1, 'reset', $2, $3, NULL, NULL, $4, $5, $6, $7, 0, 1, $8, $8, $9, NULL)",
    )
    .bind(&digest)
    .bind(user_id.as_deref())
    .bind(&email)
    .bind(code_mac(&state, &digest, &code))
    .bind(&device_name)
    .bind(&platform)
    .bind(installation_id.as_deref())
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if user_id.is_some() {
        let origin = Origin {
            device_name: &device_name,
            platform: &platform,
            installation_id: None,
        };
        if let Err(error) = deliver(&state, &email, &code, "reset", &origin).await {
            let _ = sqlx::query("DELETE FROM email_challenges WHERE digest = $1")
                .bind(&digest)
                .execute(&state.pool)
                .await;
            return Err(error);
        }
        audit(
            &state.pool,
            user_id.as_deref(),
            "auth.password_reset.email_code",
            Some("user"),
            user_id.as_deref(),
            serde_json::json!({"platform": platform}),
        )
        .await;
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(ForgotResponse {
            challenge_token: token,
            challenge_expires_at: expires_at,
            email_hint: email_hint(&email),
        }),
    ))
}

/// Checks the emailed code before the new password is asked for. 204 when it
/// is right; it stays valid for `reset`. Each check counts as an attempt.
pub async fn check(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CheckRequest>,
) -> Result<StatusCode, AppError> {
    attempt(&state, &request.challenge_token, &request.code).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sets the new password with the emailed code. Signs out every session of
/// the account, then signs in the computer or phone that asked.
pub async fn reset(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ResetRequest>,
) -> Result<Json<AuthTokens>, AppError> {
    validate_password(&request.new_password)?;
    let Attempt {
        digest,
        now,
        user_id,
        row,
    } = attempt(&state, &request.challenge_token, &request.code).await?;
    let consumed = sqlx::query(
        "UPDATE email_challenges SET consumed_at = $1 WHERE digest = $2 AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(&digest)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if consumed.rows_affected() != 1 {
        return Err(expired());
    }
    let password_hash = hash_password(request.new_password).await?;
    sqlx::query(
        "UPDATE users SET password_hash = $1, \
             email_verified_at = COALESCE(email_verified_at, $2) WHERE id = $3",
    )
    .bind(&password_hash)
    .bind(now)
    .bind(&user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    // Whoever knew the old password is signed out everywhere, and other
    // reset codes for this account stop working.
    revoke_all_user_sessions(&state.pool, &user_id).await?;
    sqlx::query(
        "UPDATE email_challenges SET consumed_at = $1 \
         WHERE user_id = $2 AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(&user_id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let device_name: String = row.try_get("device_name").map_err(AppError::from_db)?;
    let platform: String = row.try_get("platform").map_err(AppError::from_db)?;
    let installation_id: Option<String> =
        row.try_get("installation_id").map_err(AppError::from_db)?;
    let account = load_account(&state, &user_id).await?;
    let tokens = create_login_session(
        &state,
        account,
        &device_name,
        &platform,
        installation_id.as_deref(),
    )
    .await?;
    trust_device(&state, &user_id, installation_id.as_deref()).await?;
    audit(
        &state.pool,
        Some(&user_id),
        "auth.password_reset",
        Some("auth_session"),
        Some(&tokens.session_id),
        serde_json::json!({"platform": platform}),
    )
    .await;
    Ok(Json(tokens))
}

struct Attempt {
    digest: String,
    now: i64,
    user_id: String,
    row: sqlx::postgres::PgRow,
}

/// Counts one attempt at the code (atomically, as for sign-in codes) and
/// checks it. Five attempts per challenge, whether checked or used.
async fn attempt(state: &AppState, challenge_token: &str, code: &str) -> Result<Attempt, AppError> {
    let token = challenge_token.trim();
    if !valid_token_shape(token, CHALLENGE_PREFIX) {
        return Err(expired());
    }
    let Some(code) = normalize_code(code) else {
        return Err(invalid_code());
    };
    let digest = token_digest(token);
    let now = now_unix();
    // Count the attempt first, atomically, as for sign-in codes.
    let row = sqlx::query(
        "UPDATE email_challenges SET attempts = attempts + 1 \
         WHERE digest = $1 AND purpose = 'reset' AND consumed_at IS NULL AND expires_at > $2 \
           AND attempts < $3 \
         RETURNING user_id, code_mac, device_name, platform, installation_id",
    )
    .bind(&digest)
    .bind(now)
    .bind(CHALLENGE_MAX_ATTEMPTS)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or_else(expired)?;
    let user_id: Option<String> = row.try_get("user_id").map_err(AppError::from_db)?;
    let stored: String = row.try_get("code_mac").map_err(AppError::from_db)?;
    // A decoy (no account) never matches.
    let Some(user_id) = user_id.filter(|_| code_matches(state, &digest, &code, &stored)) else {
        return Err(invalid_code());
    };
    Ok(Attempt {
        digest,
        now,
        user_id,
        row,
    })
}

fn default_device_name() -> String {
    "Sanser Desktop".into()
}

fn default_platform() -> String {
    "unknown".into()
}
