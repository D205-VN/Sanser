use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, models::EventEnvelope, state::AppState, time::now_unix};

/// Persists a notification before broadcasting it. WebSocket clients can replay
/// the persisted row after a reconnect, so a short disconnect does not lose a
/// session request or state transition.
pub async fn publish(
    state: &AppState,
    user_id: &str,
    session_id: Option<&str>,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<EventEnvelope, AppError> {
    let event = EventEnvelope {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_owned(),
        session_id: session_id.map(str::to_owned),
        event_type: event_type.to_owned(),
        payload,
        created_at: now_unix(),
    };
    sqlx::query(
        "INSERT INTO connection_events \
         (id, user_id, session_id, event_type, payload_json, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&event.id)
    .bind(&event.user_id)
    .bind(&event.session_id)
    .bind(&event.event_type)
    .bind(event.payload.to_string())
    .bind(event.created_at)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;

    // No receiver is a valid state; the database row remains available for replay.
    let _ = state.events.send(event.clone());
    Ok(event)
}

pub async fn recent(
    state: &AppState,
    user_id: &str,
    since: Option<i64>,
    limit: i64,
) -> Result<Vec<EventEnvelope>, AppError> {
    let rows = if let Some(since) = since {
        sqlx::query(
            "SELECT id, user_id, session_id, event_type, payload_json, created_at \
             FROM connection_events WHERE user_id = $1 AND created_at >= $2 \
             ORDER BY created_at ASC, id ASC LIMIT $3",
        )
        .bind(user_id)
        .bind(since)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::from_db)?
    } else {
        let mut rows = sqlx::query(
            "SELECT id, user_id, session_id, event_type, payload_json, created_at \
             FROM connection_events WHERE user_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(AppError::from_db)?;
        rows.reverse();
        rows
    };

    rows.into_iter().map(row_to_event).collect()
}

fn row_to_event(row: sqlx::any::AnyRow) -> Result<EventEnvelope, AppError> {
    let payload_json: String = row.try_get("payload_json").map_err(AppError::from_db)?;
    let payload = serde_json::from_str(&payload_json).map_err(|error| {
        tracing::error!(error = %error, "stored connection event contains invalid JSON");
        AppError::Internal
    })?;
    Ok(EventEnvelope {
        id: row.try_get("id").map_err(AppError::from_db)?,
        user_id: row.try_get("user_id").map_err(AppError::from_db)?,
        session_id: row.try_get("session_id").map_err(AppError::from_db)?,
        event_type: row.try_get("event_type").map_err(AppError::from_db)?,
        payload,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
    })
}
