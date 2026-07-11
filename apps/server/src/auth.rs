use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, header, request::Parts},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use sanser_auth::{PasswordHashString, PasswordHasherService};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    error::AppError,
    models::{Account, AuthTokens},
    state::AppState,
    time::{expires_at, now_unix},
};

const ACCESS_PREFIX: &str = "sn2_a_";
const REFRESH_PREFIX: &str = "sn2_r_";

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub user_id: String,
    pub auth_session_id: String,
    pub access_digest: String,
}

impl FromRequestParts<AppState> for AuthContext {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or(AppError::Unauthorized)?;
        authenticate_access_token(state, token).await
    }
}

pub async fn authenticate_websocket(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthContext, AppError> {
    let token = bearer_token(headers)
        .or_else(|| websocket_protocol_token(headers))
        .ok_or(AppError::Unauthorized)?;
    authenticate_access_token(state, token).await
}

pub async fn authenticate_access_token(
    state: &AppState,
    token: &str,
) -> Result<AuthContext, AppError> {
    if !valid_token_shape(token, ACCESS_PREFIX) {
        return Err(AppError::Unauthorized);
    }
    let digest = token_digest(token);
    let now = now_unix();
    let row = sqlx::query(
        "SELECT s.user_id, a.auth_session_id \
         FROM access_tokens a \
         JOIN auth_sessions s ON s.id = a.auth_session_id \
         WHERE a.digest = $1 AND a.revoked_at IS NULL AND a.expires_at > $2 \
           AND s.revoked_at IS NULL AND s.expires_at > $3",
    )
    .bind(&digest)
    .bind(now)
    .bind(now)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::Unauthorized)?;

    Ok(AuthContext {
        user_id: row.try_get("user_id").map_err(AppError::from_db)?,
        auth_session_id: row.try_get("auth_session_id").map_err(AppError::from_db)?,
        access_digest: digest,
    })
}

pub async fn hash_password(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || {
        let password = Zeroizing::new(password);
        PasswordHasherService::default()
            .hash(&password)
            .map(|hash| hash.as_str().to_owned())
            .map_err(|error| {
                tracing::error!(error = %error, "password hashing failed");
                AppError::Internal
            })
    })
    .await
    .map_err(|error| {
        tracing::error!(error = %error, "password hashing task failed");
        AppError::Internal
    })?
}

pub async fn verify_password(password: String, encoded_hash: String) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || {
        let password = Zeroizing::new(password);
        let parsed = PasswordHashString::parse(encoded_hash).map_err(|error| {
            tracing::error!(error = %error, "stored password hash is invalid");
            AppError::Internal
        })?;
        Ok(PasswordHasherService::default().verify(&password, &parsed))
    })
    .await
    .map_err(|error| {
        tracing::error!(error = %error, "password verification task failed");
        AppError::Internal
    })?
}

pub async fn create_login_session(
    state: &AppState,
    account: Account,
    device_name: &str,
    platform: &str,
) -> Result<AuthTokens, AppError> {
    let auth_session_id = Uuid::new_v4().to_string();
    let access_token = new_token(ACCESS_PREFIX);
    let refresh_token = new_token(REFRESH_PREFIX);
    let access_digest = token_digest(&access_token);
    let refresh_digest = token_digest(&refresh_token);
    let now = now_unix();
    let access_expires_at = expires_at(state.config.access_token_ttl);
    let refresh_expires_at = expires_at(state.config.refresh_token_ttl);
    sqlx::query(
        "WITH inserted_session AS ( \
             INSERT INTO auth_sessions \
                 (id, user_id, device_name, platform, created_at, last_seen_at, expires_at, revoked_at) \
             VALUES ($1, $2, $3, $4, $5, $5, $6, NULL) \
             RETURNING id \
         ), inserted_access AS ( \
             INSERT INTO access_tokens \
                 (digest, auth_session_id, created_at, expires_at, revoked_at) \
             SELECT $7, id, $5, $8, NULL FROM inserted_session \
             RETURNING digest \
         ) \
         INSERT INTO refresh_tokens \
             (digest, auth_session_id, created_at, expires_at, revoked_at, rotated_at, replaced_by_digest) \
         SELECT $9, id, $5, $6, NULL, NULL, NULL FROM inserted_session",
    )
    .bind(&auth_session_id)
    .bind(&account.id)
    .bind(device_name)
    .bind(platform)
    .bind(now)
    .bind(refresh_expires_at)
    .bind(&access_digest)
    .bind(access_expires_at)
    .bind(&refresh_digest)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;

    Ok(AuthTokens {
        token_type: "Bearer",
        access_token,
        access_expires_at,
        refresh_token,
        refresh_expires_at,
        account,
        session_id: auth_session_id,
    })
}

pub async fn create_account_session(
    state: &AppState,
    account: Account,
    password_hash: String,
    device_name: &str,
    platform: &str,
) -> Result<AuthTokens, AppError> {
    let auth_session_id = Uuid::new_v4().to_string();
    let access_token = new_token(ACCESS_PREFIX);
    let refresh_token = new_token(REFRESH_PREFIX);
    let access_digest = token_digest(&access_token);
    let refresh_digest = token_digest(&refresh_token);
    let now = account.created_at;
    let access_expires_at = expires_at(state.config.access_token_ttl);
    let refresh_expires_at = expires_at(state.config.refresh_token_ttl);
    let inserted = sqlx::query(
        "WITH inserted_user AS ( \
             INSERT INTO users (id, email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) \
             RETURNING id \
         ), inserted_session AS ( \
             INSERT INTO auth_sessions \
                 (id, user_id, device_name, platform, created_at, last_seen_at, expires_at, revoked_at) \
             SELECT $6, id, $7, $8, $5, $5, $9, NULL FROM inserted_user \
             RETURNING id \
         ), inserted_access AS ( \
             INSERT INTO access_tokens \
                 (digest, auth_session_id, created_at, expires_at, revoked_at) \
             SELECT $10, id, $5, $11, NULL FROM inserted_session \
             RETURNING digest \
         ) \
         INSERT INTO refresh_tokens \
             (digest, auth_session_id, created_at, expires_at, revoked_at, rotated_at, replaced_by_digest) \
         SELECT $12, id, $5, $9, NULL, NULL, NULL FROM inserted_session",
    )
    .bind(&account.id)
    .bind(&account.email)
    .bind(&account.display_name)
    .bind(password_hash)
    .bind(now)
    .bind(&auth_session_id)
    .bind(device_name)
    .bind(platform)
    .bind(refresh_expires_at)
    .bind(&access_digest)
    .bind(access_expires_at)
    .bind(&refresh_digest)
    .execute(&state.pool)
    .await;

    if let Err(error) = inserted {
        if error
            .as_database_error()
            .and_then(|database_error| database_error.code())
            .is_some_and(|code| code == "23505")
        {
            return Err(AppError::Conflict(
                "an account with this email already exists".into(),
            ));
        }
        return Err(AppError::from_db(error));
    }

    Ok(AuthTokens {
        token_type: "Bearer",
        access_token,
        access_expires_at,
        refresh_token,
        refresh_expires_at,
        account,
        session_id: auth_session_id,
    })
}

pub async fn rotate_refresh_token(
    state: &AppState,
    refresh_token: &str,
) -> Result<AuthTokens, AppError> {
    if !valid_token_shape(refresh_token, REFRESH_PREFIX) {
        return Err(AppError::Unauthorized);
    }
    let old_digest = token_digest(refresh_token);
    let now = now_unix();
    let mut transaction = state.pool.begin().await.map_err(AppError::from_db)?;
    let row = sqlx::query(
        "SELECT r.auth_session_id, r.expires_at, r.revoked_at, r.rotated_at, \
                s.user_id, s.revoked_at AS session_revoked_at, s.expires_at AS session_expires_at, \
                u.email, u.display_name, u.created_at \
         FROM refresh_tokens r \
         JOIN auth_sessions s ON s.id = r.auth_session_id \
         JOIN users u ON u.id = s.user_id \
         WHERE r.digest = $1 \
         FOR UPDATE OF r, s",
    )
    .bind(&old_digest)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::Unauthorized)?;

    let auth_session_id: String = row.try_get("auth_session_id").map_err(AppError::from_db)?;
    let token_expiry: i64 = row.try_get("expires_at").map_err(AppError::from_db)?;
    let token_revoked: Option<i64> = row.try_get("revoked_at").map_err(AppError::from_db)?;
    let rotated_at: Option<i64> = row.try_get("rotated_at").map_err(AppError::from_db)?;
    let session_revoked: Option<i64> = row
        .try_get("session_revoked_at")
        .map_err(AppError::from_db)?;
    let session_expiry: i64 = row
        .try_get("session_expires_at")
        .map_err(AppError::from_db)?;

    if token_revoked.is_some() || rotated_at.is_some() {
        revoke_session_in_transaction(&mut transaction, &auth_session_id, now).await?;
        transaction.commit().await.map_err(AppError::from_db)?;
        tracing::warn!(auth_session_id = %auth_session_id, "refresh token reuse detected; session revoked");
        return Err(AppError::Unauthorized);
    }
    if token_expiry <= now || session_revoked.is_some() || session_expiry <= now {
        return Err(AppError::Unauthorized);
    }

    let new_access = new_token(ACCESS_PREFIX);
    let new_refresh = new_token(REFRESH_PREFIX);
    let new_access_digest = token_digest(&new_access);
    let new_refresh_digest = token_digest(&new_refresh);
    let access_expires_at = expires_at(state.config.access_token_ttl);
    let refresh_expires_at = expires_at(state.config.refresh_token_ttl);

    let rotated = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = $1, rotated_at = $2, replaced_by_digest = $3 \
         WHERE digest = $4 AND revoked_at IS NULL AND rotated_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(&new_refresh_digest)
    .bind(&old_digest)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    if rotated.rows_affected() != 1 {
        return Err(AppError::Unauthorized);
    }

    sqlx::query(
        "UPDATE access_tokens SET revoked_at = $1 \
         WHERE auth_session_id = $2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(&auth_session_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query("UPDATE auth_sessions SET last_seen_at = $1, expires_at = $2 WHERE id = $3")
        .bind(now)
        .bind(refresh_expires_at)
        .bind(&auth_session_id)
        .execute(&mut *transaction)
        .await
        .map_err(AppError::from_db)?;
    insert_access_token(
        &mut transaction,
        &new_access_digest,
        &auth_session_id,
        now,
        access_expires_at,
    )
    .await?;
    insert_refresh_token(
        &mut transaction,
        &new_refresh_digest,
        &auth_session_id,
        now,
        refresh_expires_at,
    )
    .await?;

    let account = Account {
        id: row.try_get("user_id").map_err(AppError::from_db)?,
        email: row.try_get("email").map_err(AppError::from_db)?,
        display_name: row.try_get("display_name").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
    };
    transaction.commit().await.map_err(AppError::from_db)?;

    Ok(AuthTokens {
        token_type: "Bearer",
        access_token: new_access,
        access_expires_at,
        refresh_token: new_refresh,
        refresh_expires_at,
        account,
        session_id: auth_session_id,
    })
}

pub async fn revoke_auth_session(pool: &PgPool, auth_session_id: &str) -> Result<(), AppError> {
    let now = now_unix();
    let mut transaction = pool.begin().await.map_err(AppError::from_db)?;
    revoke_session_in_transaction(&mut transaction, auth_session_id, now).await?;
    transaction.commit().await.map_err(AppError::from_db)
}

pub async fn revoke_all_user_sessions(pool: &PgPool, user_id: &str) -> Result<(), AppError> {
    let now = now_unix();
    let mut transaction = pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE auth_sessions SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE access_tokens SET revoked_at = $1 WHERE revoked_at IS NULL AND auth_session_id IN \
         (SELECT id FROM auth_sessions WHERE user_id = $2)",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = $1 WHERE revoked_at IS NULL AND auth_session_id IN \
         (SELECT id FROM auth_sessions WHERE user_id = $2)",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    transaction.commit().await.map_err(AppError::from_db)
}

pub fn normalize_email(value: &str) -> Result<String, AppError> {
    let email = value.trim().to_ascii_lowercase();
    let valid = email.len() <= 254
        && email.len() >= 5
        && !email.contains(char::is_whitespace)
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
        });
    if valid {
        Ok(email)
    } else {
        Err(AppError::Validation("email address is invalid".into()))
    }
}

pub fn validate_password(value: &str) -> Result<(), AppError> {
    let valid = (10..=128).contains(&value.chars().count())
        && value
            .chars()
            .any(|character| character.is_ascii_lowercase())
        && value
            .chars()
            .any(|character| character.is_ascii_uppercase())
        && value.chars().any(|character| character.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(AppError::Validation(
            "password must be 10–128 characters and contain upper-case, lower-case, and numeric characters"
                .into(),
        ))
    }
}

pub fn clean_label(value: &str, field: &str, max_length: usize) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > max_length
        || value.chars().any(|character| character.is_control())
    {
        return Err(AppError::Validation(format!(
            "{field} must contain 1–{max_length} printable characters"
        )));
    }
    Ok(value.to_owned())
}

pub async fn audit(
    pool: &PgPool,
    user_id: Option<&str>,
    action: &str,
    target_type: Option<&str>,
    target_id: Option<&str>,
    metadata: serde_json::Value,
) {
    let result = sqlx::query(
        "INSERT INTO audit_logs \
         (id, user_id, action, target_type, target_id, metadata_json, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(user_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(metadata.to_string())
    .bind(now_unix())
    .execute(pool)
    .await;
    if let Err(error) = result {
        tracing::warn!(error = %error, action, "failed to write audit event");
    }
}

fn new_token(prefix: &str) -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn token_digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn valid_token_shape(token: &str, prefix: &str) -> bool {
    token.starts_with(prefix)
        && token.len() == prefix.len() + 43
        && token[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim)
}

fn websocket_protocol_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)?
        .to_str()
        .ok()?
        .split(',')
        .map(str::trim)
        .find_map(|protocol| protocol.strip_prefix("bearer."))
}

async fn insert_access_token(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    digest: &str,
    auth_session_id: &str,
    now: i64,
    expires_at: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO access_tokens \
         (digest, auth_session_id, created_at, expires_at, revoked_at) \
         VALUES ($1, $2, $3, $4, NULL)",
    )
    .bind(digest)
    .bind(auth_session_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut **transaction)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}

async fn insert_refresh_token(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    digest: &str,
    auth_session_id: &str,
    now: i64,
    expires_at: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO refresh_tokens \
         (digest, auth_session_id, created_at, expires_at, revoked_at, rotated_at, replaced_by_digest) \
         VALUES ($1, $2, $3, $4, NULL, NULL, NULL)",
    )
    .bind(digest)
    .bind(auth_session_id)
    .bind(now)
    .bind(expires_at)
    .execute(&mut **transaction)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}

async fn revoke_session_in_transaction(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    auth_session_id: &str,
    now: i64,
) -> Result<(), AppError> {
    sqlx::query("UPDATE auth_sessions SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL")
        .bind(now)
        .bind(auth_session_id)
        .execute(&mut **transaction)
        .await
        .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE access_tokens SET revoked_at = $1 WHERE auth_session_id = $2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(auth_session_id)
    .execute(&mut **transaction)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = $1 WHERE auth_session_id = $2 AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(auth_session_id)
    .execute(&mut **transaction)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}
