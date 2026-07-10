use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::{
    auth::{
        AuthContext, audit, hash_password, revoke_all_user_sessions, revoke_auth_session,
        validate_password, verify_password,
    },
    error::{ApiJson, AppError},
    models::Account,
    state::AppState,
    time::now_unix,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSession {
    id: String,
    device_name: String,
    platform: String,
    created_at: i64,
    last_seen_at: i64,
    expires_at: i64,
    current: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

pub async fn get_account(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Account>, AppError> {
    let row = sqlx::query("SELECT id, email, display_name, created_at FROM users WHERE id = $1")
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::Unauthorized)?;
    Ok(Json(Account {
        id: row.try_get("id").map_err(AppError::from_db)?,
        email: row.try_get("email").map_err(AppError::from_db)?,
        display_name: row.try_get("display_name").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
    }))
}

pub async fn list_login_sessions(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<LoginSession>>, AppError> {
    let rows = sqlx::query(
        "SELECT id, device_name, platform, created_at, last_seen_at, expires_at \
         FROM auth_sessions WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > $2 \
         ORDER BY last_seen_at DESC LIMIT 100",
    )
    .bind(&auth.user_id)
    .bind(now_unix())
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let sessions = rows
        .into_iter()
        .map(|row| {
            let id: String = row.try_get("id").map_err(AppError::from_db)?;
            Ok(LoginSession {
                current: id == auth.auth_session_id,
                id,
                device_name: row.try_get("device_name").map_err(AppError::from_db)?,
                platform: row.try_get("platform").map_err(AppError::from_db)?,
                created_at: row.try_get("created_at").map_err(AppError::from_db)?,
                last_seen_at: row.try_get("last_seen_at").map_err(AppError::from_db)?,
                expires_at: row.try_get("expires_at").map_err(AppError::from_db)?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(Json(sessions))
}

pub async fn revoke_login_session(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(session_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let owned = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM auth_sessions WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(&session_id)
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if owned == 0 {
        return Err(AppError::NotFound);
    }
    revoke_auth_session(&state.pool, &session_id).await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.session.revoke",
        Some("auth_session"),
        Some(&session_id),
        serde_json::json!({"current": session_id == auth.auth_session_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn change_password(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    validate_password(&request.new_password)?;
    if request.current_password == request.new_password {
        return Err(AppError::Validation(
            "newPassword must differ from currentPassword".into(),
        ));
    }
    let hash = sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE id = $1")
        .bind(&auth.user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::Unauthorized)?;
    if !verify_password(request.current_password, hash).await? {
        return Err(AppError::Unauthorized);
    }
    let new_hash = hash_password(request.new_password).await?;
    sqlx::query("UPDATE users SET password_hash = $1, updated_at = $2 WHERE id = $3")
        .bind(new_hash)
        .bind(now_unix())
        .bind(&auth.user_id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    revoke_all_user_sessions(&state.pool, &auth.user_id).await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.password.change",
        Some("user"),
        Some(&auth.user_id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
