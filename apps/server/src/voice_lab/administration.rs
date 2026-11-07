//! Close only registered, retired rooms. Network calls never hold DB locks.
//! Successful DeleteRoom closes current peers, not cached/refreshed join tokens.
use super::*;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Default, Debug)]
pub struct ClosureBatch {
    pub closed: usize,
    pub deferred: usize,
    pub pruned: usize,
}

/// Retain closed rounds for 30 days. Never remove pending closure work, active
/// rounds or leased rows. Admissions cascade away, so even a refreshed token
/// cannot recreate admission after retention. Bounded claims support workers.
pub async fn prune_closed_rooms(state: &AppState) -> Result<usize, AppError> {
    let cutoff = now_unix().saturating_sub(30 * 24 * 60 * 60);
    let result = sqlx::query(
        "WITH expired AS (
        SELECT room_name FROM voice_lab_rooms
        WHERE retired_at IS NOT NULL AND closed_at < $1
          AND lease_until=0 AND lease_id IS NULL
        ORDER BY closed_at,room_name LIMIT 100 FOR UPDATE SKIP LOCKED
    ) DELETE FROM voice_lab_rooms r USING expired WHERE r.room_name=expired.room_name",
    )
    .bind(cutoff)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(result.rows_affected() as usize)
}

impl VoiceLabConfig {
    async fn delete_room(&self, room: &str) -> Result<(), AppError> {
        let suffix = room.strip_prefix("snvl_").ok_or(AppError::Internal)?;
        if suffix.len() != 43
            || !suffix
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(AppError::Internal);
        }
        let mut endpoint = url::Url::parse(&self.url).map_err(|_| AppError::Internal)?;
        let scheme = if endpoint.scheme() == "wss" {
            "https"
        } else {
            "http"
        };
        endpoint
            .set_scheme(scheme)
            .map_err(|_| AppError::Internal)?;
        endpoint.set_path("/twirp/livekit.RoomService/DeleteRoom");
        let now = now_unix();
        // This privileged token stays on the server. Do not use a client grant
        // for administration, or include this token in response/error/Debug.
        let token = Zeroizing::new(self.sign_claims(&serde_json::json!({
            "iss":self.key,"sub":"sanser-voice-lab-admin","iat":now,
            "nbf":now.saturating_sub(5),"exp":now.saturating_add(30),
            "video":{"room":room,"roomCreate":true,"roomJoin":false,
                "canPublish":false,"canSubscribe":false,"canPublishData":false}
        }))?);
        let mut response = self
            .admin_client
            .post(endpoint)
            .bearer_auth(token.as_str())
            .json(&serde_json::json!({"room":room}))
            .send()
            .await
            .map_err(|_| AppError::Unavailable)?;
        let status = response.status();
        // Bound the body even on errors. A proxy redirect/HTML404 must not be
        // mistaken for an authenticated Twirp acknowledgement.
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
            if bytes.len().saturating_add(chunk.len()) > 4096 {
                return Err(AppError::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| AppError::Unavailable)?;
        if (status.is_success() && body.as_object().is_some_and(|v| v.is_empty()))
            || (status == reqwest::StatusCode::NOT_FOUND
                && body.get("code").and_then(|v| v.as_str()) == Some("not_found"))
        {
            Ok(())
        } else {
            Err(AppError::Unavailable)
        }
    }
}

pub async fn close_retired_rooms(state: &AppState) -> Result<ClosureBatch, AppError> {
    let Some(config) = state.config.voice_lab.as_ref() else {
        return Ok(ClosureBatch::default());
    };
    let now = now_unix();
    // Reconcile login revocation/expiry and presence as well as explicit group
    // changes. Retire the round server-side even when a modified client ignores
    // polling or keeps its signaling socket open. Existing trigger queues close.
    sqlx::query("WITH invalid AS (
        SELECT DISTINCT g.id FROM sharing_groups g
        JOIN voice_lab_rooms r ON r.sharing_id=g.id AND r.user_id=g.user_id AND r.voice_revision=g.voice_revision
        JOIN voice_lab_admissions a ON a.room_name=r.room_name
        WHERE r.sfu_scope=$1 AND r.retired_at IS NULL AND r.closed_at IS NULL
          AND g.state IN ('preparing','active')
          AND NOT sanser_voice_admission_valid(a.room_name,a.identity,$1,$2,$3,$4)
        ORDER BY g.id LIMIT 32
    ) UPDATE sharing_groups g SET voice_revision=g.voice_revision+1 FROM invalid WHERE g.id=invalid.id")
        .bind(config.sfu_scope()).bind(now)
        .bind(now.saturating_sub(state.config.device_offline_after.as_secs() as i64))
        .bind(now.saturating_sub(crate::auth::SESSION_IDLE_SECONDS-1))
        .execute(&state.pool).await.map_err(AppError::from_db)?;
    let lease = Uuid::new_v4().to_string();
    // Durable, bounded claim with SKIP LOCKED; multiple API instances can safely
    // share the queue. Crashed workers release their claims after 30 seconds.
    let rows = sqlx::query(
        "WITH pending AS (
        SELECT room_name FROM voice_lab_rooms WHERE sfu_scope=$1
            AND retired_at IS NOT NULL AND closed_at IS NULL
            AND next_attempt_at <= $2 AND lease_until <= $2
        ORDER BY next_attempt_at,retired_at,room_name LIMIT 4 FOR UPDATE SKIP LOCKED
    ) UPDATE voice_lab_rooms r SET lease_until=$2+30,lease_id=$3,attempts=LEAST(r.attempts+1,20)
        FROM pending WHERE r.room_name=pending.room_name RETURNING r.room_name,r.attempts",
    )
    .bind(config.sfu_scope())
    .bind(now)
    .bind(&lease)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let mut batch = ClosureBatch::default();
    for row in rows {
        let room: String = row.try_get("room_name").map_err(AppError::from_db)?;
        let attempts: i32 = row.try_get("attempts").map_err(AppError::from_db)?;
        if config.delete_room(&room).await.is_ok() {
            let result = sqlx::query("UPDATE voice_lab_rooms SET closed_at=$1,lease_until=0,lease_id=NULL WHERE room_name=$2 AND lease_id=$3 AND closed_at IS NULL")
                .bind(now_unix()).bind(&room).bind(&lease).execute(&state.pool).await.map_err(AppError::from_db)?;
            batch.closed += result.rows_affected() as usize;
        } else {
            let delay = (1_i64 << attempts.clamp(0, 5)).min(30);
            sqlx::query("UPDATE voice_lab_rooms SET next_attempt_at=$1,lease_until=0,lease_id=NULL WHERE room_name=$2 AND lease_id=$3 AND closed_at IS NULL")
                .bind(now_unix().saturating_add(delay)).bind(&room).bind(&lease).execute(&state.pool).await.map_err(AppError::from_db)?;
            batch.deferred += 1;
        }
    }
    batch.pruned = prune_closed_rooms(state).await?;
    Ok(batch)
}

pub fn spawn_room_closer(
    state: AppState,
    stop: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    state.config.voice_lab.as_ref()?;
    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                _ = interval.tick() => {
                    // Cancellation also interrupts an in-flight HTTP request.
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        result = close_retired_rooms(&state) => {
                            match result {
                                Ok(batch) if batch.deferred > 0 => tracing::warn!(count=batch.deferred,"Voice Lab room closure deferred"),
                                Err(_) => tracing::warn!("Voice Lab room closure queue unavailable"),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct Mock {
        mode: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<(serde_json::Value, serde_json::Value)>>>,
    }
    async fn delete(
        State(mock): State<Mock>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Response {
        mock.calls.fetch_add(1, Ordering::SeqCst);
        let token = headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap();
        let parts: Vec<_> = token.split('.').collect();
        let mut mac = Hmac::<Sha256>::new_from_slice("a".repeat(32).as_bytes()).unwrap();
        mac.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
        mac.verify_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap())
            .unwrap();
        let claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        mock.seen.lock().await.push((body, claims));
        match mock.mode.load(Ordering::SeqCst) {
            0 => Json(serde_json::json!({})).into_response(),
            1 => (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"code":"permission_denied"})),
            )
                .into_response(),
            2 => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"code":"unavailable"})),
            )
                .into_response(),
            3 => (
                StatusCode::TEMPORARY_REDIRECT,
                [(header::LOCATION, "/trap")],
                "",
            )
                .into_response(),
            4 => Json(serde_json::json!({"padding":"x".repeat(8192)})).into_response(),
            5 => (StatusCode::NOT_FOUND, "<html>proxy</html>").into_response(),
            6 => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"code":"not_found"})),
            )
                .into_response(),
            7 => (StatusCode::OK, "not a Twirp response").into_response(),
            _ => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Json(serde_json::json!({})).into_response()
            }
        }
    }

    #[tokio::test]
    async fn administration_is_server_only_bounded_and_never_follows_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mock = Mock::default();
        let trap = mock.calls.clone();
        let app = Router::new()
            .route("/twirp/livekit.RoomService/DeleteRoom", post(delete))
            .route(
                "/trap",
                post(move || async move {
                    trap.fetch_add(1000, Ordering::SeqCst);
                    Json(serde_json::json!({}))
                }),
            )
            .with_state(mock.clone());
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(stopping.cancelled_owned())
                .await
                .unwrap();
        });
        let config =
            VoiceLabConfig::new(&format!("ws://{address}"), "test_key", "a".repeat(32)).unwrap();
        let room = format!("snvl_{}", "r".repeat(43));
        for mode in 0..=7 {
            mock.mode.store(mode, Ordering::SeqCst);
            assert_eq!(
                config.delete_room(&room).await.is_ok(),
                matches!(mode, 0 | 6),
                "mode {mode}"
            );
        }
        assert_eq!(mock.calls.load(Ordering::SeqCst), 8);
        let seen = mock.seen.lock().await;
        assert_eq!(seen[0].0, serde_json::json!({"room":room}));
        let claims = &seen[0].1;
        assert_eq!(claims["video"]["roomCreate"], true);
        assert_eq!(claims["video"]["roomJoin"], false);
        assert_eq!(claims["video"]["canPublish"], false);
        assert_eq!(claims["video"]["room"], room);
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            30
        );
        drop(seen);
        assert!(config.delete_room("arbitrary-room").await.is_err());
        assert_eq!(mock.calls.load(Ordering::SeqCst), 8);
        mock.mode.store(8, Ordering::SeqCst);
        let start = std::time::Instant::now();
        assert!(config.delete_room(&room).await.is_err());
        assert!(start.elapsed() < Duration::from_secs(4));
        stop.cancel();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn disabled_room_closer_never_reads_database_or_starts_a_worker() {
        let config = crate::config::Config::test(
            "postgres://user:pass@127.0.0.1/test?sslmode=disable".into(),
        )
        .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy(&config.database_url)
            .unwrap();
        let state = AppState::new(config, pool);
        assert_eq!(close_retired_rooms(&state).await.unwrap().closed, 0);
        assert!(spawn_room_closer(state, CancellationToken::new()).is_none());
    }
}
