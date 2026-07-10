use std::time::Duration;

use sqlx::{AnyPool, any::AnyPoolOptions};

use crate::{config::Config, error::AppError};

pub async fn connect(config: &Config) -> Result<AnyPool, AppError> {
    sqlx::any::install_default_drivers();

    if let Some(path) = &config.sqlite_path {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                tracing::error!(error = %error, path = %parent.display(), "failed to create SQLite directory");
                AppError::Internal
            })?;
        }
    }

    let pool = AnyPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Some(Duration::from_secs(300)))
        .connect(&config.database_url)
        .await
        .map_err(AppError::from_db)?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "database migration failed");
            AppError::Internal
        })?;

    Ok(pool)
}

pub async fn ready(pool: &AnyPool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT CAST(1 AS BIGINT)")
        .fetch_one(pool)
        .await
        .is_ok()
}

pub async fn cleanup(pool: &AnyPool, now: i64, offline_before: i64, pending_before: i64) {
    let operations = [
        sqlx::query("DELETE FROM access_tokens WHERE expires_at < $1 OR revoked_at < $2")
            .bind(now)
            .bind(now - 7 * 86_400)
            .execute(pool)
            .await,
        sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < $1 OR revoked_at < $2")
            .bind(now)
            .bind(now - 7 * 86_400)
            .execute(pool)
            .await,
        sqlx::query("DELETE FROM password_reset_tokens WHERE expires_at < $1 OR used_at < $2")
            .bind(now)
            .bind(now - 86_400)
            .execute(pool)
            .await,
        sqlx::query("UPDATE devices SET online = 0, streaming = 0, updated_at = $1 WHERE online = 1 AND last_seen_at < $2")
            .bind(now)
            .bind(offline_before)
            .execute(pool)
            .await,
        sqlx::query("UPDATE connection_sessions SET state = 'expired', ended_at = $1, updated_at = $2 WHERE state = 'pending' AND created_at < $3")
            .bind(now)
            .bind(now)
            .bind(pending_before)
            .execute(pool)
            .await,
        sqlx::query("DELETE FROM connection_events WHERE created_at < $1")
            .bind(now - 7 * 86_400)
            .execute(pool)
            .await,
        sqlx::query("DELETE FROM audit_logs WHERE created_at < $1")
            .bind(now - 90 * 86_400)
            .execute(pool)
            .await,
        sqlx::query(
            "DELETE FROM auth_sessions WHERE expires_at < $1 OR \
             (revoked_at IS NOT NULL AND revoked_at < $2)",
        )
        .bind(now - 7 * 86_400)
        .bind(now - 30 * 86_400)
        .execute(pool)
        .await,
        sqlx::query(
            "DELETE FROM connection_sessions WHERE ended_at IS NOT NULL AND ended_at < $1",
        )
        .bind(now - 30 * 86_400)
        .execute(pool)
        .await,
    ];

    for result in operations {
        if let Err(error) = result {
            tracing::warn!(error = %error, "database retention operation failed");
        }
    }
}
