//! Two-step verification (TOTP) for account sign-in. See docs/two-factor.md.

use axum::{Json, extract::State, http::StatusCode};
use sanser_auth::{
    SecretBox, TotpSecret, generate_recovery_code, normalize_recovery_code, normalize_totp_code,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    auth::{
        AuthContext, audit, create_login_session, hash_password, new_token, token_digest,
        valid_token_shape, verify_password,
    },
    error::{ApiJson, AppError},
    models::{Account, AuthTokens},
    state::AppState,
    time::now_unix,
};

const ISSUER: &str = "Sanser";
const CHALLENGE_PREFIX: &str = "sn2_c_";
pub const CHALLENGE_TTL_SECONDS: i64 = 5 * 60;
pub const CHALLENGE_MAX_ATTEMPTS: i32 = 5;
pub const RECOVERY_CODE_COUNT: usize = 10;
/// Unconfirmed enrollments are discarded after this long.
const ENROLLMENT_TTL_SECONDS: i64 = 15 * 60;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorStatus {
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled_at: Option<i64>,
    recovery_codes_remaining: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollRequest {
    password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollResponse {
    secret: String,
    otpauth_uri: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmRequest {
    code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodesResponse {
    recovery_codes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReauthenticateRequest {
    password: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    recovery_code: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChallengeRequest {
    challenge_token: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    recovery_code: Option<String>,
}

/// Which second factor was presented.
enum SecondFactor<'a> {
    Totp(&'a str),
    Recovery(&'a str),
}

impl<'a> SecondFactor<'a> {
    fn from_parts(code: Option<&'a str>, recovery: Option<&'a str>) -> Result<Self, AppError> {
        match (
            code.filter(|value| !value.trim().is_empty()),
            recovery.filter(|value| !value.trim().is_empty()),
        ) {
            (Some(code), None) => Ok(Self::Totp(code)),
            (None, Some(recovery)) => Ok(Self::Recovery(recovery)),
            _ => Err(AppError::Validation(
                "provide either code or recoveryCode".into(),
            )),
        }
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::Totp(_) => "totp",
            Self::Recovery(_) => "recovery_code",
        }
    }
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<TwoFactorStatus>, AppError> {
    let enabled_at = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT enabled_at FROM user_two_factor WHERE user_id = $1",
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .flatten();
    let remaining = if enabled_at.is_some() {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM two_factor_recovery_codes WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(&auth.user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(AppError::from_db)?
    } else {
        0
    };
    Ok(Json(TwoFactorStatus {
        enabled: enabled_at.is_some(),
        enabled_at,
        recovery_codes_remaining: remaining,
    }))
}

pub async fn enroll(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<EnrollRequest>,
) -> Result<Json<EnrollResponse>, AppError> {
    limit(&state, &auth.user_id).await?;
    let (email, password_hash) = account_credentials(&state, &auth.user_id).await?;
    if !verify_password(request.password, password_hash).await? {
        return Err(incorrect_password());
    }
    if is_enabled(&state, &auth.user_id).await? {
        return Err(AppError::Conflict(
            "two-step verification is already turned on".into(),
        ));
    }
    let secret = TotpSecret::generate();
    let envelope = sealing_box(&state)?
        .seal(&secret, &auth.user_id)
        .map_err(|_| AppError::Internal)?;
    let now = now_unix();
    // Replaces any earlier unconfirmed enrollment; an enabled row is left alone.
    let written = sqlx::query(
        "INSERT INTO user_two_factor (user_id, secret_envelope, enabled_at, last_used_step, created_at, updated_at) \
         VALUES ($1, $2, NULL, NULL, $3, $3) \
         ON CONFLICT (user_id) DO UPDATE SET secret_envelope = EXCLUDED.secret_envelope, \
             last_used_step = NULL, created_at = EXCLUDED.created_at, updated_at = EXCLUDED.updated_at \
         WHERE user_two_factor.enabled_at IS NULL",
    )
    .bind(&auth.user_id)
    .bind(&envelope)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if written.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "two-step verification is already turned on".into(),
        ));
    }
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.two_factor.enroll",
        Some("user"),
        Some(&auth.user_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(EnrollResponse {
        secret: secret.to_base32().to_string(),
        otpauth_uri: secret.otpauth_uri(ISSUER, &email).to_string(),
    }))
}

pub async fn confirm(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<ConfirmRequest>,
) -> Result<Json<RecoveryCodesResponse>, AppError> {
    limit(&state, &auth.user_id).await?;
    let row = sqlx::query(
        "SELECT secret_envelope, enabled_at, created_at FROM user_two_factor WHERE user_id = $1",
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or_else(|| AppError::Conflict("start two-step verification setup first".into()))?;
    let enabled_at: Option<i64> = row.try_get("enabled_at").map_err(AppError::from_db)?;
    if enabled_at.is_some() {
        return Err(AppError::Conflict(
            "two-step verification is already turned on".into(),
        ));
    }
    let created_at: i64 = row.try_get("created_at").map_err(AppError::from_db)?;
    let now = now_unix();
    if now - created_at > ENROLLMENT_TTL_SECONDS {
        return Err(AppError::Conflict(
            "two-step verification setup expired; start again".into(),
        ));
    }
    let envelope: String = row.try_get("secret_envelope").map_err(AppError::from_db)?;
    let secret = open_secret(&state, &envelope, &auth.user_id)?;
    let Some(step) = secret.verify(&request.code, now, None) else {
        audit_failure(&state, &auth.user_id, "confirm", "totp").await;
        return Err(invalid_code());
    };

    let codes = new_recovery_codes();
    let hashes = hash_recovery_codes(&codes).await?;
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    let enabled = sqlx::query(
        "UPDATE user_two_factor SET enabled_at = $1, last_used_step = $2, updated_at = $1 \
         WHERE user_id = $3 AND enabled_at IS NULL AND secret_envelope = $4",
    )
    .bind(now)
    .bind(step)
    .bind(&auth.user_id)
    .bind(&envelope)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    if enabled.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "two-step verification setup changed; start again".into(),
        ));
    }
    replace_recovery_codes(&mut tx, &auth.user_id, &hashes, now).await?;
    // Sign-ins made with only a password must not keep refreshing forever.
    revoke_other_account_sessions(&mut tx, &auth.user_id, &auth.auth_session_id, now).await?;
    tx.commit().await.map_err(AppError::from_db)?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.two_factor.enable",
        Some("user"),
        Some(&auth.user_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: codes.iter().map(|code| code.to_string()).collect(),
    }))
}

pub async fn disable(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<ReauthenticateRequest>,
) -> Result<StatusCode, AppError> {
    let factor =
        SecondFactor::from_parts(request.code.as_deref(), request.recovery_code.as_deref())?;
    limit(&state, &auth.user_id).await?;
    reauthenticate(&state, &auth.user_id, request.password, &factor, "disable").await?;
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    for statement in [
        "DELETE FROM two_factor_recovery_codes WHERE user_id = $1",
        "DELETE FROM two_factor_challenges WHERE user_id = $1",
        "DELETE FROM user_two_factor WHERE user_id = $1",
    ] {
        sqlx::query(statement)
            .bind(&auth.user_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::from_db)?;
    }
    tx.commit().await.map_err(AppError::from_db)?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.two_factor.disable",
        Some("user"),
        Some(&auth.user_id),
        serde_json::json!({"factor": factor.kind()}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<ReauthenticateRequest>,
) -> Result<Json<RecoveryCodesResponse>, AppError> {
    let factor =
        SecondFactor::from_parts(request.code.as_deref(), request.recovery_code.as_deref())?;
    limit(&state, &auth.user_id).await?;
    reauthenticate(
        &state,
        &auth.user_id,
        request.password,
        &factor,
        "recovery_codes",
    )
    .await?;
    let codes = new_recovery_codes();
    let hashes = hash_recovery_codes(&codes).await?;
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    replace_recovery_codes(&mut tx, &auth.user_id, &hashes, now_unix()).await?;
    tx.commit().await.map_err(AppError::from_db)?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.two_factor.recovery_codes",
        Some("user"),
        Some(&auth.user_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: codes.iter().map(|code| code.to_string()).collect(),
    }))
}

/// Called by password login after the password was verified. Returns the
/// challenge as an error so clients that predate two-step verification show
/// its message instead of misreading a success response.
pub async fn require_second_step(
    state: &AppState,
    account: &Account,
    device_name: &str,
    platform: &str,
    installation_id: Option<&str>,
) -> Result<(), AppError> {
    if !is_enabled(state, &account.id).await? {
        return Ok(());
    }
    let token = new_token(CHALLENGE_PREFIX);
    let now = now_unix();
    let expires_at = now + CHALLENGE_TTL_SECONDS;
    sqlx::query(
        "INSERT INTO two_factor_challenges \
         (digest, user_id, device_name, platform, installation_id, attempts, created_at, expires_at, consumed_at) \
         VALUES ($1, $2, $3, $4, $5, 0, $6, $7, NULL)",
    )
    .bind(token_digest(&token))
    .bind(&account.id)
    .bind(device_name)
    .bind(platform)
    .bind(installation_id)
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    audit(
        &state.pool,
        Some(&account.id),
        "auth.two_factor.challenge",
        Some("user"),
        Some(&account.id),
        serde_json::json!({"platform": platform}),
    )
    .await;
    Err(AppError::TwoFactorRequired {
        challenge_token: token,
        expires_at,
    })
}

pub async fn complete_login(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ChallengeRequest>,
) -> Result<Json<AuthTokens>, AppError> {
    let factor =
        SecondFactor::from_parts(request.code.as_deref(), request.recovery_code.as_deref())?;
    let token = request.challenge_token.trim();
    if !valid_token_shape(token, CHALLENGE_PREFIX) {
        return Err(AppError::Unauthorized);
    }
    let now = now_unix();
    // Count the attempt before checking the code, atomically, so parallel
    // guesses cannot exceed the per-challenge limit.
    let row = sqlx::query(
        "UPDATE two_factor_challenges SET attempts = attempts + 1 \
         WHERE digest = $1 AND consumed_at IS NULL AND expires_at > $2 AND attempts < $3 \
         RETURNING user_id, device_name, platform, installation_id",
    )
    .bind(token_digest(token))
    .bind(now)
    .bind(CHALLENGE_MAX_ATTEMPTS)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::Unauthorized)?;
    let user_id: String = row.try_get("user_id").map_err(AppError::from_db)?;
    limit(&state, &user_id).await?;
    if !verify_second_factor(&state, &user_id, &factor).await? {
        audit_failure(&state, &user_id, "login", factor.kind()).await;
        return Err(invalid_code());
    }
    let consumed = sqlx::query(
        "UPDATE two_factor_challenges SET consumed_at = $1 WHERE digest = $2 AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(token_digest(token))
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if consumed.rows_affected() != 1 {
        return Err(AppError::Unauthorized);
    }
    let account_row =
        sqlx::query("SELECT id, email, display_name, created_at FROM users WHERE id = $1")
            .bind(&user_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(AppError::from_db)?
            .ok_or(AppError::Unauthorized)?;
    let account = Account {
        id: account_row.try_get("id").map_err(AppError::from_db)?,
        email: account_row.try_get("email").map_err(AppError::from_db)?,
        display_name: account_row
            .try_get("display_name")
            .map_err(AppError::from_db)?,
        created_at: account_row
            .try_get("created_at")
            .map_err(AppError::from_db)?,
    };
    let device_name: String = row.try_get("device_name").map_err(AppError::from_db)?;
    let platform: String = row.try_get("platform").map_err(AppError::from_db)?;
    let installation_id: Option<String> =
        row.try_get("installation_id").map_err(AppError::from_db)?;
    let tokens = create_login_session(
        &state,
        account,
        &device_name,
        &platform,
        installation_id.as_deref(),
    )
    .await?;
    audit(
        &state.pool,
        Some(&user_id),
        "auth.login",
        Some("auth_session"),
        Some(&tokens.session_id),
        serde_json::json!({"platform": platform, "twoFactor": factor.kind()}),
    )
    .await;
    Ok(Json(tokens))
}

/// Removes challenges and abandoned enrollments. Called from the periodic cleanup.
pub async fn cleanup(pool: &sqlx::PgPool, now: i64) {
    for (statement, before) in [
        (
            "DELETE FROM two_factor_challenges WHERE expires_at < $1",
            now - 86_400,
        ),
        (
            "DELETE FROM user_two_factor WHERE enabled_at IS NULL AND created_at < $1",
            now - ENROLLMENT_TTL_SECONDS,
        ),
    ] {
        if let Err(error) = sqlx::query(statement).bind(before).execute(pool).await {
            tracing::warn!(error = %error, "two-step verification cleanup failed");
        }
    }
}

async fn reauthenticate(
    state: &AppState,
    user_id: &str,
    password: String,
    factor: &SecondFactor<'_>,
    action: &str,
) -> Result<(), AppError> {
    let (_, password_hash) = account_credentials(state, user_id).await?;
    if !verify_password(password, password_hash).await? {
        return Err(incorrect_password());
    }
    if !is_enabled(state, user_id).await? {
        return Err(AppError::Conflict(
            "two-step verification is not turned on".into(),
        ));
    }
    if !verify_second_factor(state, user_id, factor).await? {
        audit_failure(state, user_id, action, factor.kind()).await;
        return Err(invalid_code());
    }
    Ok(())
}

/// Verifies and consumes one second factor for an account with 2FA enabled.
async fn verify_second_factor(
    state: &AppState,
    user_id: &str,
    factor: &SecondFactor<'_>,
) -> Result<bool, AppError> {
    let row = sqlx::query(
        "SELECT secret_envelope, last_used_step FROM user_two_factor \
         WHERE user_id = $1 AND enabled_at IS NOT NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let Some(row) = row else { return Ok(false) };
    match factor {
        SecondFactor::Totp(code) => {
            if normalize_totp_code(code).is_none() {
                return Ok(false);
            }
            let envelope: String = row.try_get("secret_envelope").map_err(AppError::from_db)?;
            let last_used: Option<i64> =
                row.try_get("last_used_step").map_err(AppError::from_db)?;
            let secret = open_secret(state, &envelope, user_id)?;
            let Some(step) = secret.verify(code, now_unix(), last_used) else {
                return Ok(false);
            };
            // Conditional write: a concurrent use of the same code loses.
            let updated = sqlx::query(
                "UPDATE user_two_factor SET last_used_step = $1, updated_at = $2 \
                 WHERE user_id = $3 AND enabled_at IS NOT NULL \
                   AND (last_used_step IS NULL OR last_used_step < $1)",
            )
            .bind(step)
            .bind(now_unix())
            .bind(user_id)
            .execute(&state.pool)
            .await
            .map_err(AppError::from_db)?;
            Ok(updated.rows_affected() == 1)
        }
        SecondFactor::Recovery(code) => {
            let Some(normalized) = normalize_recovery_code(code) else {
                return Ok(false);
            };
            let candidates = sqlx::query(
                "SELECT id, code_hash FROM two_factor_recovery_codes \
                 WHERE user_id = $1 AND used_at IS NULL",
            )
            .bind(user_id)
            .fetch_all(&state.pool)
            .await
            .map_err(AppError::from_db)?;
            let mut matched = None;
            for candidate in candidates {
                let hash: String = candidate.try_get("code_hash").map_err(AppError::from_db)?;
                if verify_password(normalized.clone(), hash).await? && matched.is_none() {
                    matched = Some(
                        candidate
                            .try_get::<String, _>("id")
                            .map_err(AppError::from_db)?,
                    );
                }
            }
            let Some(id) = matched else { return Ok(false) };
            let used = sqlx::query(
                "UPDATE two_factor_recovery_codes SET used_at = $1 \
                 WHERE id = $2 AND user_id = $3 AND used_at IS NULL",
            )
            .bind(now_unix())
            .bind(&id)
            .bind(user_id)
            .execute(&state.pool)
            .await
            .map_err(AppError::from_db)?;
            Ok(used.rows_affected() == 1)
        }
    }
}

async fn is_enabled(state: &AppState, user_id: &str) -> Result<bool, AppError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM user_two_factor WHERE user_id = $1 AND enabled_at IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::from_db)
}

async fn account_credentials(
    state: &AppState,
    user_id: &str,
) -> Result<(String, String), AppError> {
    let row = sqlx::query("SELECT email, password_hash FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::Unauthorized)?;
    Ok((
        row.try_get("email").map_err(AppError::from_db)?,
        row.try_get("password_hash").map_err(AppError::from_db)?,
    ))
}

async fn limit(state: &AppState, user_id: &str) -> Result<(), AppError> {
    if state
        .two_factor_limiter
        .check(format!("two-factor:{user_id}"))
        .await
    {
        Ok(())
    } else {
        Err(AppError::RateLimited)
    }
}

/// Not 401: an authenticated client would treat 401 as an expired access
/// token and silently retry, spending another password attempt.
fn incorrect_password() -> AppError {
    AppError::Validation("The password is incorrect.".into())
}

fn invalid_code() -> AppError {
    AppError::Validation("The verification code is incorrect or was already used.".into())
}

async fn audit_failure(state: &AppState, user_id: &str, action: &str, factor: &str) {
    audit(
        &state.pool,
        Some(user_id),
        "auth.two_factor.failure",
        Some("user"),
        Some(user_id),
        serde_json::json!({"action": action, "factor": factor}),
    )
    .await;
}

fn sealing_box(state: &AppState) -> Result<SecretBox, AppError> {
    match &state.config.two_factor_key {
        Some(key) => SecretBox::derive(key, "k"),
        None => SecretBox::derive(&state.config.session_credential_key, "d"),
    }
    .map_err(|_| AppError::Internal)
}

fn open_secret(state: &AppState, envelope: &str, user_id: &str) -> Result<TotpSecret, AppError> {
    let opener = match SecretBox::envelope_key_id(envelope) {
        Some("k") => state
            .config
            .two_factor_key
            .as_ref()
            .map(|key| SecretBox::derive(key, "k")),
        Some("d") => Some(SecretBox::derive(&state.config.session_credential_key, "d")),
        _ => None,
    };
    let Some(Ok(opener)) = opener else {
        tracing::error!("two-step verification secret uses an unavailable key");
        return Err(AppError::Internal);
    };
    opener.open(envelope, user_id).map_err(|_| {
        tracing::error!("two-step verification secret could not be decrypted");
        AppError::Internal
    })
}

fn new_recovery_codes() -> Vec<Zeroizing<String>> {
    (0..RECOVERY_CODE_COUNT)
        .map(|_| generate_recovery_code())
        .collect()
}

async fn hash_recovery_codes(codes: &[Zeroizing<String>]) -> Result<Vec<String>, AppError> {
    let mut hashes = Vec::with_capacity(codes.len());
    for code in codes {
        let normalized = normalize_recovery_code(code).ok_or(AppError::Internal)?;
        hashes.push(hash_password(normalized).await?);
    }
    Ok(hashes)
}

async fn replace_recovery_codes(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
    hashes: &[String],
    now: i64,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM two_factor_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::from_db)?;
    for hash in hashes {
        sqlx::query(
            "INSERT INTO two_factor_recovery_codes (id, user_id, code_hash, created_at, used_at) \
             VALUES ($1, $2, $3, $4, NULL)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_id)
        .bind(hash)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::from_db)?;
    }
    Ok(())
}

async fn revoke_other_account_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: &str,
    keep_session_id: &str,
    now: i64,
) -> Result<(), AppError> {
    for statement in [
        "UPDATE access_tokens SET revoked_at = $1 WHERE revoked_at IS NULL AND auth_session_id IN \
         (SELECT id FROM auth_sessions WHERE user_id = $2 AND id <> $3 AND scope = 'account')",
        "UPDATE refresh_tokens SET revoked_at = $1 WHERE revoked_at IS NULL AND auth_session_id IN \
         (SELECT id FROM auth_sessions WHERE user_id = $2 AND id <> $3 AND scope = 'account')",
        "UPDATE auth_sessions SET revoked_at = $1 WHERE user_id = $2 AND id <> $3 \
         AND scope = 'account' AND revoked_at IS NULL",
    ] {
        sqlx::query(statement)
            .bind(now)
            .bind(user_id)
            .bind(keep_session_id)
            .execute(&mut **tx)
            .await
            .map_err(AppError::from_db)?;
    }
    Ok(())
}
