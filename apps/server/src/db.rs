use std::{str::FromStr, time::Duration};

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use crate::{config::Config, error::AppError};

pub async fn connect(config: &Config) -> Result<PgPool, AppError> {
    let connection_url = sqlx_connection_url(&config.database_url)?;
    let options = PgConnectOptions::from_str(&connection_url)
        .map_err(AppError::from_db)?
        .application_name("sanser-server/2.0.0")
        .statement_cache_capacity(256);
    tracing::info!("opening Neon PostgreSQL connection pool");
    let pool = PgPoolOptions::new()
        .max_connections(config.database_max_connections)
        .min_connections(config.database_min_connections)
        .acquire_timeout(config.database_acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(300)))
        .max_lifetime(Some(Duration::from_secs(1_800)))
        .test_before_acquire(false)
        .connect_with(options)
        .await
        .map_err(AppError::from_db)?;

    tracing::info!("Neon PostgreSQL pool is ready; applying migrations");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "database migration failed");
            AppError::Internal
        })?;
    tracing::info!("database migrations are current");

    Ok(pool)
}

fn sqlx_connection_url(database_url: &str) -> Result<String, AppError> {
    let mut parsed = url::Url::parse(database_url).map_err(|_| AppError::Internal)?;
    // Neon currently emits channel_binding=require in some generated URLs.
    // SQLx 0.8 does not parse that libpq option and otherwise warns with the
    // raw query parameter. TLS itself remains mandatory via sslmode=require.
    let parameters = parsed
        .query_pairs()
        .filter(|(key, _)| key != "channel_binding")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    if !parameters.is_empty() {
        parsed.query_pairs_mut().extend_pairs(parameters);
    }
    Ok(parsed.into())
}

pub async fn ready(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT CAST(1 AS BIGINT)")
        .fetch_one(pool)
        .await
        .is_ok()
}

pub async fn cleanup(pool: &PgPool, now: i64, offline_before: i64, pending_before: i64) {
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
        sqlx::query("UPDATE devices SET online = FALSE, streaming = FALSE, updated_at = $1 WHERE online = TRUE AND last_seen_at < $2")
            .bind(now)
            .bind(offline_before)
            .execute(pool)
            .await,
        sqlx::query(
            "UPDATE connection_sessions AS session SET state = 'disconnected', ended_at = $1, \
             updated_at = $1, disconnect_reason = 'device_offline' \
             WHERE session.state IN ('pending', 'accepted') AND EXISTS (\
               SELECT 1 FROM devices AS device WHERE device.user_id = session.user_id \
               AND device.id IN (session.requester_device_id, session.host_device_id) \
               AND device.online = FALSE\
             )",
        )
        .bind(now)
        .execute(pool)
        .await,
        sqlx::query(
            "UPDATE devices AS device SET streaming = FALSE, updated_at = $1 \
             WHERE device.streaming = TRUE AND NOT EXISTS (\
               SELECT 1 FROM connection_sessions AS session \
               WHERE session.user_id = device.user_id AND session.host_device_id = device.id \
               AND session.state = 'accepted'\
             )",
        )
        .bind(now)
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
