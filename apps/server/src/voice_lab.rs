//! Experimental admission tokens, NOT a production replacement for native voice.
//! Gateway joins/rejoins use live DB authorization; the SFU listener must stay
//! private. Keep disabled until physical trials; see apps/voice-lab/README.md.
use std::{env, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;
use zeroize::Zeroizing;

mod administration;
pub mod gateway;
pub use administration::{close_retired_rooms, prune_closed_rooms, spawn_room_closer};

use crate::{
    auth::AuthContext,
    config::ConfigError,
    error::{ApiJson, AppError},
    state::AppState,
    time::now_unix,
};

// Intentionally no Debug: the API secret must never enter config/request logs.
#[derive(Clone)]
pub struct VoiceLabConfig {
    url: String,
    key: String,
    secret: Arc<Zeroizing<String>>,
    admin_client: reqwest::Client,
    gateway_url: Option<String>,
}

impl VoiceLabConfig {
    pub(crate) fn from_env() -> Result<Option<Self>, ConfigError> {
        match env::var("VOICE_LAB_EXPERIMENTAL_ENABLED")
            .as_deref()
            .unwrap_or("false")
        {
            "false" => Ok(None),
            "true" => Self::new(
                &env::var("VOICE_LAB_LIVEKIT_URL").unwrap_or_default(),
                &env::var("VOICE_LAB_LIVEKIT_API_KEY").unwrap_or_default(),
                env::var("VOICE_LAB_LIVEKIT_API_SECRET").unwrap_or_default(),
            )
            .and_then(|config| {
                config.with_gateway(&env::var("VOICE_LAB_GATEWAY_URL").unwrap_or_default())
            })
            .map(Some),
            _ => Err(ConfigError::Invalid(
                "VOICE_LAB_EXPERIMENTAL_ENABLED must be true or false".into(),
            )),
        }
    }

    pub fn new(url: &str, key: &str, secret: String) -> Result<Self, ConfigError> {
        // Wrap before validation so rejected secrets are also wiped on drop.
        let secret = Arc::new(Zeroizing::new(secret));
        let invalid = || {
            ConfigError::Invalid("Voice Lab requires a WSS endpoint (WS only on loopback), an API key and a secret of at least 32 bytes".into())
        };
        let parsed = url::Url::parse(url).map_err(|_| invalid())?;
        let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"));
        if !(parsed.scheme() == "wss" || (parsed.scheme() == "ws" && local))
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !matches!(parsed.path(), "" | "/")
            || key.is_empty()
            || key.len() > 256
            || !key
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || v == b'_' || v == b'-')
            || !(32..=4096).contains(&secret.len())
        {
            return Err(invalid());
        }
        Ok(Self {
            url: parsed.to_string(),
            key: key.into(),
            secret,
            gateway_url: None,
            admin_client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
                .connect_timeout(std::time::Duration::from_secs(2))
                .https_only(!local)
                .build()
                .map_err(|_| {
                    ConfigError::Invalid("Voice Lab HTTP client could not be initialized".into())
                })?,
        })
    }

    /// Signaling must be private to this API host; publish only the gateway.
    /// The SFU listener/firewall must also prevent external bypass (see docs).
    pub fn with_gateway(mut self, public: &str) -> Result<Self, ConfigError> {
        let invalid = || {
            ConfigError::Invalid("Voice Lab needs a WSS gateway and a private loopback WS upstream on a different origin".into())
        };
        let upstream = url::Url::parse(&self.url).map_err(|_| invalid())?;
        let gateway = url::Url::parse(public).map_err(|_| invalid())?;
        let local_gateway = gateway.host_str() == Some("127.0.0.1");
        if upstream.scheme() != "ws"
            || upstream.host_str() != Some("127.0.0.1")
            || !(gateway.scheme() == "wss" || (gateway.scheme() == "ws" && local_gateway))
            || gateway.host_str().is_none()
            || !gateway.username().is_empty()
            || gateway.password().is_some()
            || gateway.query().is_some()
            || gateway.fragment().is_some()
            || gateway.path() != "/"
            || gateway.origin() == upstream.origin()
        {
            return Err(invalid());
        }
        self.gateway_url = Some(gateway.to_string());
        Ok(self)
    }

    fn opaque(&self, domain: &str, parts: &[&str]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(domain.as_bytes());
        for part in parts {
            mac.update(&(part.len() as u64).to_be_bytes());
            mac.update(part.as_bytes());
        }
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    fn sfu_scope(&self) -> String {
        self.opaque("sanser-voice-lab-sfu-v1", &[&self.url, &self.key])
    }

    fn sign_claims(&self, claims: &serde_json::Value) -> Result<String, AppError> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).map_err(|_| AppError::Internal)?);
        let mut token = format!("{header}.{payload}");
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes())
            .map_err(|_| AppError::Internal)?;
        mac.update(token.as_bytes());
        token.push('.');
        token.push_str(&URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()));
        Ok(token)
    }

    fn issue(
        &self,
        user: &str,
        group: &str,
        device: &str,
        auth_session: &str,
        revision: i64,
        now: i64,
    ) -> Result<Credentials, AppError> {
        if revision < 0 {
            return Err(AppError::Internal);
        }
        let revision = revision.to_string();
        let scope = self.sfu_scope();
        let room = format!(
            "snvl_{}",
            self.opaque(
                "sanser-voice-lab-room-v3",
                &[user, group, &revision, &scope]
            )
        );
        let identity = format!(
            "snp_{}",
            self.opaque(
                "sanser-voice-lab-peer-v2",
                &[user, group, device, auth_session]
            )
        );
        let expires_at = now.checked_add(60).ok_or(AppError::Internal)?;
        let claims = serde_json::json!({
            "iss": self.key, "sub": identity, "iat": now, "nbf": now.saturating_sub(5),
            "exp": expires_at, "jti": Uuid::new_v4().to_string(),
            "video": {
                "room": room.clone(), "roomJoin": true, "canSubscribe": true,
                "canPublish": true, "canPublishSources": ["microphone"], "canPublishData": false,
                "canUpdateOwnMetadata": false, "roomAdmin": false, "roomCreate": false,
                "roomList": false, "roomRecord": false, "ingressAdmin": false
            }
        });
        // RFC 7519 HS256 envelope; no selectable algorithm or client-supplied claims.
        // Reuse the existing RustCrypto HMAC/SHA256 primitives; never log this payload.
        let token = self.sign_claims(&claims)?;
        Ok(Credentials {
            provider: "livekit-lab",
            url: self.gateway_url.as_ref().unwrap_or(&self.url).clone(),
            token,
            expires_at,
            room_revision: self.opaque(
                "sanser-voice-lab-revision-v2",
                &[user, group, &revision, &scope],
            ),
            room_name: room,
            identity,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialRequest {
    device_id: String,
}

// Do not derive Debug: this response carries a bearer credential.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    provider: &'static str,
    url: String,
    token: String,
    expires_at: i64,
    room_revision: String,
    #[serde(skip)]
    room_name: String,
    #[serde(skip)]
    identity: String,
}

pub async fn capabilities(
    State(state): State<AppState>,
    _auth: AuthContext,
) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"gateway":state.config.voice_lab.as_ref().is_some_and(|config|config.gateway_url.is_some())}),
    )
}

pub async fn credentials(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(group): Path<String>,
    ApiJson(request): ApiJson<CredentialRequest>,
) -> Result<Response, AppError> {
    let config = state
        .config
        .voice_lab
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let group = Uuid::parse_str(&group)
        .map_err(|_| AppError::Validation("sharing UUID is required".into()))?
        .to_string();
    let device = Uuid::parse_str(&request.device_id)
        .map_err(|_| AppError::Validation("device UUID is required".into()))?
        .to_string();
    if !state
        .voice_lab_limiter
        .check(format!("{}:{device}", auth.user_id))
        .await
    {
        return Err(AppError::RateLimited);
    }
    // Lock the group first, then evaluate membership in a fresh statement.
    // A removal that commits while admission waits cannot authorize a token for
    // the new room using a stale membership snapshot. Removal/stop/ready paths
    // already use the group-first lock order. Triggered revisions cover cleanup.
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    let revision = sqlx::query_scalar::<_, i64>(
        "SELECT voice_revision FROM sharing_groups WHERE id=$1 AND user_id=$2 FOR SHARE",
    )
    .bind(&group)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    // Scope *every* participant/session to the account.
    // Account bearer authority is not proof of which physical computer runs the
    // browser: this follows Sanser's existing account-owned device policy.
    let row = sqlx::query(
        "SELECT g.state, h.online AS host_online, h.last_seen_at AS host_seen,
            d.online AS peer_online, d.last_seen_at AS peer_seen,
            (g.host_device_id = d.id OR EXISTS(
                SELECT 1 FROM sharing_members m JOIN connection_sessions s
                    ON s.id=m.session_id AND s.user_id=m.user_id
                WHERE m.sharing_id=g.id AND m.user_id=g.user_id
                    AND m.viewer_device_id=d.id AND m.state='accepted'
                    AND s.sharing_id=g.id AND s.host_device_id=g.host_device_id
                    AND s.requester_device_id=d.id AND s.state='accepted'
                    AND s.access_mode='view-only' AND s.ended_at IS NULL
            )) AS admitted
         FROM sharing_groups g JOIN devices h ON h.id=g.host_device_id AND h.user_id=g.user_id
         JOIN devices d ON d.id=$3 AND d.user_id=g.user_id
         WHERE g.id=$1 AND g.user_id=$2",
    )
    .bind(&group)
    .bind(&auth.user_id)
    .bind(&device)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    let now = now_unix();
    let fresh_after = now.saturating_sub(state.config.device_offline_after.as_secs() as i64);
    let open = matches!(
        row.try_get::<String, _>("state")
            .map_err(AppError::from_db)?
            .as_str(),
        "preparing" | "active"
    );
    let fresh = |online: bool, seen: Option<i64>| {
        online && seen.is_some_and(|v| v >= fresh_after && v <= now)
    };
    let host_fresh = fresh(
        row.try_get("host_online").map_err(AppError::from_db)?,
        row.try_get("host_seen").map_err(AppError::from_db)?,
    );
    let peer_fresh = fresh(
        row.try_get("peer_online").map_err(AppError::from_db)?,
        row.try_get("peer_seen").map_err(AppError::from_db)?,
    );
    if !open
        || !host_fresh
        || !peer_fresh
        || !row
            .try_get::<bool, _>("admitted")
            .map_err(AppError::from_db)?
    {
        return Err(AppError::Forbidden);
    }
    let grant = config.issue(
        &auth.user_id,
        &group,
        &device,
        &auth.auth_session_id,
        revision,
        now,
    )?;
    // Registration and admission share the same group lock/transaction. A
    // concurrent retirement cannot commit without seeing this issued room.
    sqlx::query("INSERT INTO voice_lab_rooms(room_name,sfu_scope,sharing_id,user_id,voice_revision,created_at) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(room_name) DO NOTHING")
        .bind(&grant.room_name).bind(config.sfu_scope()).bind(&group).bind(&auth.user_id).bind(revision).bind(now)
        .execute(&mut *tx).await.map_err(AppError::from_db)?;
    sqlx::query("INSERT INTO voice_lab_admissions(room_name,identity,user_id,device_id,auth_session_id,created_at) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(room_name,identity) DO NOTHING")
        .bind(&grant.room_name).bind(&grant.identity).bind(&auth.user_id).bind(&device).bind(&auth.auth_session_id).bind(now)
        .execute(&mut *tx).await.map_err(AppError::from_db)?;
    tx.commit().await.map_err(AppError::from_db)?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(grant),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> VoiceLabConfig {
        VoiceLabConfig::new("wss://voice.example.test", "test_key", "a".repeat(32)).unwrap()
    }

    #[test]
    fn rejects_insecure_endpoints_and_never_echoes_secrets() {
        for url in [
            "https://voice.test",
            "ws://voice.test",
            "wss://secret@voice.test",
            "wss://voice.test?token=secret",
            "wss://voice.test/path",
            "wss://voice.test#secret",
        ] {
            let error = VoiceLabConfig::new(url, "test", "secret".into())
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains(url));
        }
        assert!(VoiceLabConfig::new("ws://127.0.0.1:7880", "test", "a".repeat(32)).is_ok());
        assert!(VoiceLabConfig::new("wss://voice.test", "", "a".repeat(32)).is_err());
    }

    #[test]
    fn grant_is_short_lived_microphone_only_and_signature_verifies() {
        let c = config();
        let grant = c
            .issue("user", "group", "device", "login", 0, 1800000000)
            .unwrap();
        let parts: Vec<_> = grant.token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let payload: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(payload["exp"], 1800000060_i64);
        assert_eq!(payload["nbf"], 1799999995_i64);
        assert_eq!(
            payload["video"]["canPublishSources"],
            serde_json::json!(["microphone"])
        );
        assert_eq!(payload["video"]["canPublishData"], false);
        assert_eq!(payload["video"]["roomAdmin"], false);
        let mut mac = Hmac::<Sha256>::new_from_slice(c.secret.as_bytes()).unwrap();
        mac.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
        mac.verify_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap())
            .unwrap();
        assert!(!serde_json::to_string(&payload).unwrap().contains("device"));
        assert!(payload.get("metadata").is_none());
    }

    #[test]
    fn room_and_participant_are_opaque_stable_and_tenant_scoped() {
        let c = config();
        assert_eq!(
            c.opaque("room", &["a", "bc"]),
            c.opaque("room", &["a", "bc"])
        );
        assert_ne!(
            c.opaque("room", &["a", "bc"]),
            c.opaque("room", &["ab", "c"])
        );
        assert_ne!(
            c.opaque("room", &["user-a", "group"]),
            c.opaque("room", &["user-b", "group"])
        );
        assert_ne!(
            c.opaque("room", &["user", "group"]),
            c.opaque("peer", &["user", "group"])
        );
        assert_ne!(
            c.issue("u", "g", "d", "login", 0, 1).unwrap().token,
            c.issue("u", "g", "d", "login", 0, 1).unwrap().token
        );
    }

    #[test]
    fn revisions_change_room_without_changing_the_participant_or_exposing_ids() {
        let c = config();
        let payload = |grant: &Credentials| -> serde_json::Value {
            serde_json::from_slice(
                &URL_SAFE_NO_PAD
                    .decode(grant.token.split('.').nth(1).unwrap())
                    .unwrap(),
            )
            .unwrap()
        };
        let before = c.issue("user", "group", "device", "login", 0, 10).unwrap();
        let after = c.issue("user", "group", "device", "login", 1, 10).unwrap();
        assert_ne!(
            payload(&before)["video"]["room"],
            payload(&after)["video"]["room"]
        );
        assert_ne!(before.room_revision, after.room_revision);
        assert_eq!(payload(&before)["sub"], payload(&after)["sub"]);
        assert!(c.issue("user", "group", "device", "login", -1, 10).is_err());
    }

    #[tokio::test]
    async fn disabled_endpoint_never_queries_database() {
        let config = crate::config::Config::test(
            "postgres://test:test@127.0.0.1/test?sslmode=require".into(),
        )
        .unwrap();
        assert!(config.voice_lab.is_none());
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy(&config.database_url)
            .unwrap();
        let state = AppState::new(config, pool);
        let result = credentials(
            State(state),
            AuthContext {
                user_id: "u".into(),
                auth_session_id: "s".into(),
                access_digest: "d".into(),
            },
            Path(Uuid::new_v4().to_string()),
            ApiJson(CredentialRequest {
                device_id: Uuid::new_v4().to_string(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Unavailable)));
    }

    #[tokio::test]
    async fn router_requires_auth_and_cors_requires_explicit_lab_origin() {
        use axum::{
            body::Body,
            http::{HeaderValue, Request, StatusCode},
        };
        use tower::ServiceExt;
        for allowed in [false, true] {
            let mut config = crate::config::Config::test(
                "postgres://test:test@127.0.0.1/test?sslmode=require".into(),
            )
            .unwrap();
            if allowed {
                config
                    .allowed_origins
                    .push(HeaderValue::from_static("http://127.0.0.1:1431"));
            }
            let pool = sqlx::postgres::PgPoolOptions::new()
                .connect_lazy(&config.database_url)
                .unwrap();
            let app = crate::build_router(AppState::new(config, pool));
            let request = Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v2/sharing/{}/voice-lab/credentials",
                    Uuid::new_v4()
                ))
                .header(header::ORIGIN, "http://127.0.0.1:1431")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"deviceId":"{}"}}"#,
                    Uuid::new_v4()
                )))
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
                allowed
            );
        }
    }
}
