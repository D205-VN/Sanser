use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{
        AuthContext, audit, clean_label, create_account_session, create_login_session,
        hash_password, normalize_email, revoke_auth_session, rotate_refresh_token,
        validate_password, verify_password,
    },
    error::{ApiJson, AppError},
    models::Account,
    state::AppState,
    time::now_unix,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterRequest {
    email: String,
    password: String,
    display_name: String,
    #[serde(default = "default_device_name")]
    device_name: String,
    #[serde(default = "default_platform")]
    platform: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    email: String,
    password: String,
    #[serde(default = "default_device_name")]
    device_name: String,
    #[serde(default = "default_platform")]
    platform: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshRequest {
    refresh_token: String,
}

pub async fn register(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RegisterRequest>,
) -> Result<Response, AppError> {
    let email = normalize_email(&request.email)?;
    validate_password(&request.password)?;
    let display_name = clean_label(&request.display_name, "displayName", 80)?;
    let device_name = clean_label(&request.device_name, "deviceName", 120)?;
    let platform = clean_label(&request.platform, "platform", 40)?;

    let password_hash = hash_password(request.password).await?;
    let now = now_unix();
    let account = Account {
        id: Uuid::new_v4().to_string(),
        email,
        display_name,
        created_at: now,
    };
    let tokens = create_account_session(
        &state,
        account.clone(),
        password_hash,
        &device_name,
        &platform,
    )
    .await?;
    audit(
        &state.pool,
        Some(&account.id),
        "auth.register",
        Some("user"),
        Some(&account.id),
        serde_json::json!({"platform": platform}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(tokens)).into_response())
}

pub async fn login(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<Json<crate::models::AuthTokens>, AppError> {
    let email = normalize_email(&request.email)?;
    if !state.login_limiter.check(format!("login:{email}")).await {
        return Err(AppError::RateLimited);
    }
    let device_name = clean_label(&request.device_name, "deviceName", 120)?;
    let platform = clean_label(&request.platform, "platform", 40)?;
    let row = sqlx::query(
        "SELECT id, email, display_name, password_hash, created_at FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?;

    let Some(row) = row else {
        burn_invalid_password(request.password).await?;
        return Err(AppError::Unauthorized);
    };
    let password_hash: String = row.try_get("password_hash").map_err(AppError::from_db)?;
    if !verify_password(request.password, password_hash).await? {
        return Err(AppError::Unauthorized);
    }

    let account = Account {
        id: row.try_get("id").map_err(AppError::from_db)?,
        email: row.try_get("email").map_err(AppError::from_db)?,
        display_name: row.try_get("display_name").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
    };
    let tokens = create_login_session(&state, account.clone(), &device_name, &platform).await?;
    audit(
        &state.pool,
        Some(&account.id),
        "auth.login",
        Some("auth_session"),
        Some(&tokens.session_id),
        serde_json::json!({"platform": platform}),
    )
    .await;
    Ok(Json(tokens))
}

pub async fn refresh(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<RefreshRequest>,
) -> Result<Json<crate::models::AuthTokens>, AppError> {
    let tokens = rotate_refresh_token(&state, request.refresh_token.trim()).await?;
    audit(
        &state.pool,
        Some(&tokens.account.id),
        "auth.refresh",
        Some("auth_session"),
        Some(&tokens.session_id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(tokens))
}

pub async fn logout(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<StatusCode, AppError> {
    revoke_auth_session(&state.pool, &auth.auth_session_id).await?;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "auth.logout",
        Some("auth_session"),
        Some(&auth.auth_session_id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn burn_invalid_password(password: String) -> Result<(), AppError> {
    // A fixed valid hash keeps unknown-account login timing close to a real verification.
    const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$N2ZqRVJXY0xYZ0JYaFhnTQ$Y74gYub18WnWQ4EFVgyB7yxGyehPV20xuQiyPcYrt88";
    let _ = verify_password(password, DUMMY_HASH.into()).await?;
    Ok(())
}

fn default_device_name() -> String {
    "Sanser Desktop".into()
}

fn default_platform() -> String {
    "unknown".into()
}
