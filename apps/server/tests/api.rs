use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use sanser_server::{
    build_router,
    config::{Config, NetworkMode},
    db,
    state::AppState,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

struct TestServer {
    _directory: TempDir,
    app: Router,
    state: AppState,
}

impl TestServer {
    async fn new() -> Self {
        Self::with_config(|_| {}).await
    }

    async fn with_config(update: impl FnOnce(&mut Config)) -> Self {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database_path = directory.path().join("server.sqlite");
        let database_url = format!(
            "sqlite://{}?mode=rwc&busy_timeout=5000",
            database_path.display()
        );
        let mut config = Config::test(database_url);
        update(&mut config);
        let pool = db::connect(&config).await.expect("test database");
        let state = AppState::new(config, pool);
        let app = build_router(state.clone());
        Self {
            _directory: directory,
            app,
            state,
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        access_token: Option<&str>,
    ) -> (StatusCode, Value, http::HeaderMap) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = access_token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = if let Some(value) = body {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize request"))
        } else {
            Body::empty()
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("router response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("JSON response")
        };
        (status, value, headers)
    }

    async fn register(&self, email: &str) -> Value {
        let (status, body, _) = self
            .request(
                Method::POST,
                "/api/v2/auth/register",
                Some(json!({
                    "email": email,
                    "password": "StrongPass123",
                    "displayName": "Test User",
                    "deviceName": "Test runner",
                    "platform": "test"
                })),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }
}

#[tokio::test]
async fn auth_tokens_are_hashed_rotated_expired_and_revoked() {
    let server = TestServer::new().await;
    let registered = server.register("auth@example.test").await;
    let access = registered["accessToken"].as_str().expect("access token");
    let refresh = registered["refreshToken"].as_str().expect("refresh token");
    let auth_session_id = registered["sessionId"].as_str().expect("session id");

    let stored = sqlx::query_scalar::<_, String>("SELECT digest FROM access_tokens LIMIT 1")
        .fetch_one(&server.state.pool)
        .await
        .expect("stored token digest");
    assert_ne!(stored, access);
    assert!(!stored.contains(access));

    let (status, account, _) = server
        .request(Method::GET, "/api/v2/account", None, Some(access))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(account["email"], "auth@example.test");

    let (status, rotated, _) = server
        .request(
            Method::POST,
            "/api/v2/auth/refresh",
            Some(json!({"refreshToken": refresh})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let new_access = rotated["accessToken"].as_str().expect("new access token");
    assert_ne!(new_access, access);
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/account", None, Some(access))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    sqlx::query("UPDATE auth_sessions SET expires_at = 0 WHERE id = $1")
        .bind(auth_session_id)
        .execute(&server.state.pool)
        .await
        .expect("expire session");
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/account", None, Some(new_access))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    let login = server
        .request(
            Method::POST,
            "/api/v2/auth/login",
            Some(json!({
                "email":"auth@example.test",
                "password":"StrongPass123"
            })),
            None,
        )
        .await;
    assert_eq!(login.0, StatusCode::OK, "{}", login.1);
    let login_access = login.1["accessToken"].as_str().expect("login access");
    assert_eq!(
        server
            .request(
                Method::POST,
                "/api/v2/auth/logout",
                Some(json!({})),
                Some(login_access)
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/account", None, Some(login_access))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn devices_and_session_state_machine_enforce_ownership() {
    let server = TestServer::new().await;
    let auth = server.register("devices@example.test").await;
    let access = auth["accessToken"].as_str().expect("access token");
    let requester_id = Uuid::new_v4().to_string();
    let host_id = Uuid::new_v4().to_string();

    for (id, name, route) in [
        (&requester_id, "Mac client", "192.168.1.20"),
        (&host_id, "Windows host", "192.168.1.30"),
    ] {
        let (status, body, _) = server
            .request(
                Method::POST,
                "/api/v2/devices/register",
                Some(json!({
                    "id": id,
                    "name": name,
                    "platform": "test",
                    "routeAddress": route,
                    "nativeTransport": true,
                    "webrtc": true,
                    "codecs": ["h264", "hevc"]
                })),
                Some(access),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let heartbeat = server
        .request(
            Method::POST,
            "/api/v2/devices/heartbeat",
            Some(json!({
                "deviceId":host_id,
                "streaming":false,
                "routeAddress":"192.168.1.30",
                "networkQuality":"excellent",
                "latencyMs":3
            })),
            Some(access),
        )
        .await;
    assert_eq!(heartbeat.0, StatusCode::OK, "{}", heartbeat.1);
    assert_eq!(heartbeat.1["latencyMs"], 3);

    let created = server
        .request(
            Method::POST,
            "/api/v2/sessions",
            Some(json!({
                "requesterDeviceId": requester_id,
                "hostDeviceId": host_id,
                "networkMode":"direct",
                "qualityProfile":"competitive",
                "requestedCodec":"hevc"
            })),
            Some(access),
        )
        .await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);
    let session_id = created.1["id"].as_str().expect("session id");
    assert_eq!(created.1["state"], "pending");

    let accepted = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{session_id}/accept"),
            None,
            Some(access),
        )
        .await;
    assert_eq!(accepted.0, StatusCode::OK, "{}", accepted.1);
    assert_eq!(accepted.1["state"], "accepted");
    assert_eq!(accepted.1["selectedTransport"], "snv2");

    let other = server.register("other@example.test").await;
    let other_access = other["accessToken"].as_str().expect("other access");
    assert_eq!(
        server
            .request(
                Method::GET,
                &format!("/api/v2/sessions/{session_id}"),
                None,
                Some(other_access)
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        server
            .request(
                Method::POST,
                &format!("/api/v2/sessions/{session_id}/disconnect"),
                None,
                Some(access)
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let disconnected = server
        .request(
            Method::GET,
            &format!("/api/v2/sessions/{session_id}"),
            None,
            Some(access),
        )
        .await;
    assert_eq!(disconnected.1["state"], "disconnected");
}

#[tokio::test]
async fn ice_uses_authenticated_short_lived_turn_credentials() {
    let server = TestServer::with_config(|config| {
        config.network_mode = NetworkMode::Relay;
        config.turn_urls = vec!["turn:relay.example.test:3478?transport=udp".into()];
        config.turn_shared_secret = Some("test-shared-secret".into());
    })
    .await;
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/network/ice", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let auth = server.register("ice@example.test").await;
    let access = auth["accessToken"].as_str().expect("access token");
    let response = server
        .request(Method::GET, "/api/v2/network/ice", None, Some(access))
        .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.1);
    assert_eq!(response.1["iceTransportPolicy"], "relay");
    assert!(response.1["expiresAt"].as_i64().is_some());
    let username = response.1["iceServers"][0]["username"]
        .as_str()
        .expect("temporary TURN username");
    assert!(username.contains(':'));
    assert!(
        !response.1["iceServers"][0]["credential"]
            .as_str()
            .expect("TURN credential")
            .is_empty()
    );
}

#[tokio::test]
async fn request_guards_return_structured_errors_and_request_ids() {
    let server = TestServer::new().await;
    let health = server
        .request(Method::GET, "/api/v2/health", None, None)
        .await;
    assert_eq!(health.0, StatusCode::OK);
    assert_eq!(health.1["version"], "2.0.0");
    assert!(health.2.contains_key("x-request-id"));
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/readiness", None, None)
            .await
            .0,
        StatusCode::OK
    );

    let invalid = server
        .request(
            Method::POST,
            "/api/v2/auth/register",
            Some(json!({"email":"bad"})),
            None,
        )
        .await;
    assert_eq!(invalid.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.1["error"]["code"], "validation_error");

    let limited = TestServer::with_config(|config| config.general_rate_limit_per_minute = 1).await;
    assert_eq!(
        limited
            .request(Method::GET, "/api/v2/health", None, None)
            .await
            .0,
        StatusCode::OK
    );
    let rate_limited = limited
        .request(Method::GET, "/api/v2/health", None, None)
        .await;
    assert_eq!(rate_limited.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(rate_limited.1["error"]["code"], "rate_limited");
}

#[tokio::test]
async fn migration_and_cleanup_work_on_sqlite() {
    let server = TestServer::new().await;
    let migrations = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&server.state.pool)
        .await
        .expect("migration metadata");
    assert_eq!(migrations, 1);
    assert!(db::ready(&server.state.pool).await);

    let auth = server.register("cleanup@example.test").await;
    let access = auth["accessToken"].as_str().expect("access token");
    let device_id = Uuid::new_v4().to_string();
    assert_eq!(
        server
            .request(
                Method::POST,
                "/api/v2/devices/register",
                Some(json!({
                    "id": device_id,
                    "name":"Old device",
                    "platform":"test"
                })),
                Some(access)
            )
            .await
            .0,
        StatusCode::CREATED
    );
    sqlx::query("UPDATE devices SET last_seen_at = 1 WHERE id = $1")
        .bind(&device_id)
        .execute(&server.state.pool)
        .await
        .expect("age device");
    db::cleanup(&server.state.pool, 10_000, 9_000, 9_000).await;
    let online = sqlx::query_scalar::<_, i64>("SELECT online FROM devices WHERE id = $1")
        .bind(device_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("device state");
    assert_eq!(online, 0);
}
