//! Account access log: who connected to which computer, when, and how.
//!
//! The log is a read-only view over `connection_sessions`, which already
//! records requester, host, timestamps, access mode and end reason for every
//! request. Retention is enforced by [`prune`].
use axum::{
    Json,
    extract::{Query, State},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use crate::{
    auth::AuthContext,
    error::AppError,
    models::{HistoryEntry, Page, SessionAccess},
    state::AppState,
    time::now_unix,
};

/// Sessions are kept for 90 days after they end…
pub const HISTORY_RETENTION_SECONDS: i64 = 90 * 86_400;
/// …and at most this many ended sessions are kept per account.
pub const HISTORY_MAX_PER_ACCOUNT: i64 = 1_000;
const MAX_FILTER_DEVICES: usize = 8;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<u16>,
    /// Comma-separated device IDs; restricts the log to sessions involving them.
    #[serde(default)]
    device_ids: Option<String>,
    /// `any` (default), `incoming` (the devices were the host) or `outgoing`.
    #[serde(default)]
    direction: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct HistoryCursor {
    c: i64,
    i: String,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Page<HistoryEntry>>, AppError> {
    let limit = i64::from(query.limit.unwrap_or(50).clamp(1, 100));
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let device_ids = query
        .device_ids
        .as_deref()
        .map(parse_device_ids)
        .transpose()?;
    let direction = match query.direction.as_deref().map(str::trim) {
        None | Some("" | "any") => "any",
        Some("incoming") => "incoming",
        Some("outgoing") => "outgoing",
        Some(_) => {
            return Err(AppError::Validation(
                "direction must be any, incoming, or outgoing".into(),
            ));
        }
    };
    let rows = sqlx::query(
        "SELECT session.id, session.requester_device_id, session.host_device_id, session.state, \
                session.access_mode, session.sharing_id, session.selected_transport, \
                session.created_at, session.accepted_at, session.ended_at, \
                session.disconnect_reason, session.relay_used_at, \
                requester.name AS requester_name, requester.platform AS requester_platform, \
                requester.computer_id AS requester_computer_id, \
                host.name AS host_name, host.platform AS host_platform, \
                host.computer_id AS host_computer_id \
         FROM connection_sessions AS session \
         JOIN devices AS requester ON requester.id = session.requester_device_id \
              AND requester.user_id = session.user_id \
         JOIN devices AS host ON host.id = session.host_device_id AND host.user_id = session.user_id \
         WHERE session.user_id = $1 \
           AND COALESCE(session.ended_at, session.created_at) >= $2 \
           AND ($3::TEXT[] IS NULL OR CASE $4 \
                WHEN 'incoming' THEN session.host_device_id = ANY($3) \
                WHEN 'outgoing' THEN session.requester_device_id = ANY($3) \
                ELSE session.host_device_id = ANY($3) OR session.requester_device_id = ANY($3) END) \
           AND ($5::BIGINT IS NULL OR (session.created_at, session.id) < ($5, $6)) \
         ORDER BY session.created_at DESC, session.id DESC LIMIT $7",
    )
    .bind(&auth.user_id)
    .bind(now_unix() - HISTORY_RETENTION_SECONDS)
    .bind(&device_ids)
    .bind(direction)
    .bind(cursor.as_ref().map(|cursor| cursor.c))
    .bind(cursor.as_ref().map_or("", |cursor| cursor.i.as_str()))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?;

    let page_size = usize::try_from(limit).unwrap_or(100);
    let has_more = rows.len() > page_size;
    let items = rows
        .into_iter()
        .take(page_size)
        .map(row_to_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if has_more {
        items.last().map(encode_cursor).transpose()?
    } else {
        None
    };
    Ok(Json(Page { items, next_cursor }))
}

/// Deletes ended sessions older than the retention window, then trims each
/// account to its newest [`HISTORY_MAX_PER_ACCOUNT`] ended sessions. Active
/// sessions are never removed here.
pub async fn prune(pool: &PgPool, now: i64) -> Result<u64, sqlx::Error> {
    let expired =
        sqlx::query("DELETE FROM connection_sessions WHERE ended_at IS NOT NULL AND ended_at < $1")
            .bind(now - HISTORY_RETENTION_SECONDS)
            .execute(pool)
            .await?
            .rows_affected();
    let excess = sqlx::query(
        "DELETE FROM connection_sessions AS session USING (\
            SELECT id FROM (\
              SELECT id, row_number() OVER (PARTITION BY user_id ORDER BY created_at DESC, id DESC) AS position \
              FROM connection_sessions WHERE ended_at IS NOT NULL\
            ) AS ranked WHERE ranked.position > $1\
         ) AS excess WHERE session.id = excess.id",
    )
    .bind(HISTORY_MAX_PER_ACCOUNT)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(expired + excess)
}

fn row_to_entry(row: sqlx::postgres::PgRow) -> Result<HistoryEntry, AppError> {
    let get_str = |name: &str| row.try_get::<String, _>(name).map_err(AppError::from_db);
    let get_opt_str = |name: &str| {
        row.try_get::<Option<String>, _>(name)
            .map_err(AppError::from_db)
    };
    let get_opt_i64 = |name: &str| {
        row.try_get::<Option<i64>, _>(name)
            .map_err(AppError::from_db)
    };
    let state = get_str("state")?;
    let accepted_at = get_opt_i64("accepted_at")?;
    let ended_at = get_opt_i64("ended_at")?;
    let route = route_label(
        get_opt_i64("relay_used_at")?,
        accepted_at,
        get_opt_str("selected_transport")?.as_deref(),
    );
    let end_reason = get_opt_str("disconnect_reason")?.or_else(|| {
        matches!(state.as_str(), "rejected" | "expired" | "disconnected").then(|| state.clone())
    });
    Ok(HistoryEntry {
        id: get_str("id")?,
        requester_device_id: get_str("requester_device_id")?,
        requester_name: get_str("requester_name")?,
        requester_platform: get_str("requester_platform")?,
        requester_computer_id: get_opt_str("requester_computer_id")?,
        host_device_id: get_str("host_device_id")?,
        host_name: get_str("host_name")?,
        host_platform: get_str("host_platform")?,
        host_computer_id: get_opt_str("host_computer_id")?,
        access_mode: if get_str("access_mode")? == "view-only" {
            SessionAccess::ViewOnly
        } else {
            SessionAccess::Control
        },
        route,
        shared_view: get_opt_str("sharing_id")?.is_some(),
        started_at: row.try_get("created_at").map_err(AppError::from_db)?,
        connected_at: accepted_at,
        ended_at,
        duration_seconds: duration(accepted_at, ended_at),
        end_reason,
        state,
    })
}

/// Relay is recorded by the server when a peer opens the session relay. An
/// accepted session that never touched the relay used the direct path.
pub(crate) fn route_label(
    relay_used_at: Option<i64>,
    accepted_at: Option<i64>,
    selected_transport: Option<&str>,
) -> Option<&'static str> {
    if relay_used_at.is_some() {
        Some("relay")
    } else if accepted_at.is_some() && selected_transport.is_some() {
        Some("direct")
    } else {
        None
    }
}

pub(crate) fn duration(accepted_at: Option<i64>, ended_at: Option<i64>) -> Option<i64> {
    Some((ended_at? - accepted_at?).max(0))
}

fn parse_device_ids(value: &str) -> Result<Vec<String>, AppError> {
    let ids = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            uuid::Uuid::parse_str(part)
                .map(|id| id.to_string())
                .map_err(|_| AppError::Validation("deviceIds must be UUIDs".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty() || ids.len() > MAX_FILTER_DEVICES {
        return Err(AppError::Validation(format!(
            "deviceIds must list between 1 and {MAX_FILTER_DEVICES} devices"
        )));
    }
    Ok(ids)
}

fn encode_cursor(entry: &HistoryEntry) -> Result<String, AppError> {
    let payload = serde_json::to_vec(&HistoryCursor {
        c: entry.started_at,
        i: entry.id.clone(),
    })
    .map_err(|_| AppError::Internal)?;
    Ok(URL_SAFE_NO_PAD.encode(payload))
}

fn decode_cursor(value: &str) -> Result<HistoryCursor, AppError> {
    let invalid = || AppError::Validation("cursor is invalid".into());
    if value.len() > 256 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn route_reflects_relay_use_and_media_start() {
        assert_eq!(route_label(Some(5), Some(1), Some("native")), Some("relay"));
        assert_eq!(route_label(None, Some(1), Some("native")), Some("direct"));
        assert_eq!(route_label(None, None, None), None);
        assert_eq!(route_label(None, Some(1), None), None);
    }

    #[test]
    fn duration_needs_both_ends_and_is_never_negative() {
        assert_eq!(duration(Some(10), Some(70)), Some(60));
        assert_eq!(duration(Some(10), None), None);
        assert_eq!(duration(None, Some(10)), None);
        assert_eq!(duration(Some(10), Some(5)), Some(0));
    }

    #[test]
    fn cursor_round_trips_and_rejects_garbage() {
        let entry = HistoryEntry {
            id: "b".into(),
            requester_device_id: String::new(),
            requester_name: String::new(),
            requester_platform: String::new(),
            requester_computer_id: None,
            host_device_id: String::new(),
            host_name: String::new(),
            host_platform: String::new(),
            host_computer_id: None,
            state: "disconnected".into(),
            access_mode: SessionAccess::Control,
            route: None,
            shared_view: false,
            started_at: 42,
            connected_at: None,
            ended_at: None,
            duration_seconds: None,
            end_reason: None,
        };
        let cursor = decode_cursor(&encode_cursor(&entry).unwrap()).unwrap();
        assert_eq!((cursor.c, cursor.i.as_str()), (42, "b"));
        assert!(decode_cursor("not base64!").is_err());
        assert!(decode_cursor(&"A".repeat(300)).is_err());
    }

    #[test]
    fn device_filter_requires_bounded_uuids() {
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(parse_device_ids(&format!(" {id} ,")).unwrap(), vec![id]);
        assert!(parse_device_ids("").is_err());
        assert!(parse_device_ids("abc").is_err());
        let many = (0..9)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_device_ids(&many).is_err());
    }
}
