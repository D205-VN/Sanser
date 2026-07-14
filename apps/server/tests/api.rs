use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use http_body_util::BodyExt;
use sanser_server::{
    build_router,
    config::{Config, NetworkMode},
    db,
    state::AppState,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::sync::{Arc, OnceLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

struct TestServer {
    app: Router,
    state: AppState,
    admin_pool: PgPool,
    schema: String,
    _database_permit: OwnedSemaphorePermit,
}

impl TestServer {
    async fn new() -> Option<Self> {
        Self::with_config(|_| {}).await
    }

    async fn with_config(update: impl FnOnce(&mut Config)) -> Option<Self> {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::TRACE)
            .try_init();
        // The suite is opt-in and intentionally never reads production DATABASE_URL.
        let database_url = std::env::var("TEST_DATABASE_URL").ok()?;
        static DATABASE_LOCK: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let database_permit = DATABASE_LOCK
            .get_or_init(|| Arc::new(Semaphore::new(1)))
            .clone()
            .acquire_owned()
            .await
            .expect("test database semaphore");
        let mut parsed = url::Url::parse(&database_url).expect("valid TEST_DATABASE_URL");
        assert!(
            !parsed.query_pairs().any(|(key, _)| key == "options"),
            "TEST_DATABASE_URL may not override PostgreSQL connection options"
        );
        Config::test(database_url.clone()).expect("TEST_DATABASE_URL must be a TLS Neon URL");

        let admin_database_url = without_channel_binding(&database_url);
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&admin_database_url)
            .await
            .expect("connect to TEST_DATABASE_URL");
        let schema = format!("sanser_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
            .execute(&admin_pool)
            .await
            .expect("create isolated test schema");
        parsed
            .query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));

        let mut config = Config::test(parsed.into()).expect("scoped test configuration");
        update(&mut config);
        let pool = db::connect(&config).await.expect("test database");
        let state = AppState::new(config, pool);
        let app = build_router(state.clone());
        Some(Self {
            app,
            state,
            admin_pool,
            schema,
            _database_permit: database_permit,
        })
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

    async fn close(self) {
        self.state.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA \"{}\" CASCADE", self.schema))
            .execute(&self.admin_pool)
            .await
            .expect("drop isolated test schema");
        self.admin_pool.close().await;
    }
}

fn without_channel_binding(database_url: &str) -> String {
    let mut parsed = url::Url::parse(database_url).expect("valid test database URL");
    let parameters = parsed
        .query_pairs()
        .filter(|(key, _)| key != "channel_binding")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    parsed.query_pairs_mut().extend_pairs(parameters);
    parsed.into()
}

#[tokio::test]
async fn auth_tokens_are_hashed_rotated_expired_and_revoked() {
    let Some(server) = TestServer::new().await else {
        return;
    };
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
    server.close().await;
}

#[tokio::test]
async fn devices_and_session_state_machine_enforce_ownership() {
    let Some(server) = TestServer::new().await else {
        return;
    };
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

    let session_request = json!({
        "requesterDeviceId": requester_id,
        "hostDeviceId": host_id,
        "networkMode":"direct",
        "qualityProfile":"competitive",
        "requestedCodec":"hevc"
    });
    let (first_create, second_create) = tokio::join!(
        server.request(
            Method::POST,
            "/api/v2/sessions",
            Some(session_request.clone()),
            Some(access),
        ),
        server.request(
            Method::POST,
            "/api/v2/sessions",
            Some(session_request),
            Some(access),
        )
    );
    let (created, rejected_overlap) = if first_create.0 == StatusCode::CREATED {
        (first_create, second_create)
    } else {
        (second_create, first_create)
    };
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);
    assert_eq!(
        rejected_overlap.0,
        StatusCode::CONFLICT,
        "{}",
        rejected_overlap.1
    );
    let session_id = created.1["id"].as_str().expect("session id");
    assert_eq!(created.1["state"], "pending");
    let host_queue = server
        .request(
            Method::GET,
            &format!("/api/v2/sessions?hostDeviceId={host_id}&state=active"),
            None,
            Some(access),
        )
        .await;
    assert_eq!(host_queue.0, StatusCode::OK, "{}", host_queue.1);
    assert_eq!(host_queue.1["items"][0]["id"], session_id);
    assert_eq!(host_queue.1["items"][0]["requesterDeviceId"], requester_id);
    assert_eq!(
        server
            .request(
                Method::GET,
                &format!("/api/v2/sessions/{session_id}/credentials?deviceId={requester_id}"),
                None,
                Some(access),
            )
            .await
            .0,
        StatusCode::CONFLICT
    );

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
    assert_eq!(accepted.1["selectedTransport"], "native");
    assert_eq!(
        server
            .request(
                Method::GET,
                &format!("/api/v2/sessions/{session_id}/credentials?deviceId={requester_id}"),
                None,
                Some(access),
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        server
            .request(
                Method::POST,
                &format!("/api/v2/sessions/{session_id}/native-ready"),
                Some(json!({"deviceId": host_id})),
                Some(access),
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let requester_ready = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{session_id}/native-ready"),
            Some(json!({"deviceId": requester_id})),
            Some(access),
        )
        .await;
    assert_eq!(requester_ready.0, StatusCode::OK, "{}", requester_ready.1);
    assert!(requester_ready.1["requesterReadyAt"].as_i64().is_some());

    let credential_path =
        format!("/api/v2/sessions/{session_id}/credentials?deviceId={requester_id}");
    assert_eq!(
        server
            .request(Method::GET, &credential_path, None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server
            .request(
                Method::GET,
                &format!(
                    "/api/v2/sessions/{session_id}/credentials?deviceId={}",
                    Uuid::new_v4()
                ),
                None,
                Some(access),
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let credentials = server
        .request(Method::GET, &credential_path, None, Some(access))
        .await;
    assert_eq!(credentials.0, StatusCode::OK, "{}", credentials.1);
    assert_eq!(
        credentials.2.get(header::CACHE_CONTROL),
        Some(&http::HeaderValue::from_static("no-store"))
    );
    assert_eq!(credentials.1["deviceId"], requester_id);
    assert_eq!(credentials.1["peerDeviceId"], host_id);
    assert_eq!(credentials.1["peerRouteAddress"], "192.168.1.30");
    assert_eq!(credentials.1["basePort"], 50_000);
    assert_eq!(
        credentials.1["expiresAt"].as_i64(),
        requester_ready.1["requesterReadyAt"]
            .as_i64()
            .map(|value| value + 60)
    );
    let session_token = credentials.1["sessionToken"]
        .as_str()
        .expect("native session token");
    assert_eq!(session_token.len(), 43);
    assert!(
        !credentials
            .1
            .to_string()
            .contains(&STANDARD.encode([0xA5; 32]))
    );

    let host_credentials = server
        .request(
            Method::GET,
            &format!("/api/v2/sessions/{session_id}/credentials?deviceId={host_id}"),
            None,
            Some(access),
        )
        .await;
    assert_eq!(host_credentials.0, StatusCode::OK, "{}", host_credentials.1);
    assert_eq!(host_credentials.1["peerDeviceId"], requester_id);
    assert_eq!(host_credentials.1["peerRouteAddress"], "192.168.1.20");
    assert_eq!(host_credentials.1["sessionToken"], session_token);

    sqlx::query("UPDATE connection_sessions SET selected_transport = 'webrtc' WHERE id = $1")
        .bind(session_id)
        .execute(&server.state.pool)
        .await
        .expect("change selected transport");
    assert_eq!(
        server
            .request(Method::GET, &credential_path, None, Some(access))
            .await
            .0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE connection_sessions SET selected_transport = 'native' WHERE id = $1")
        .bind(session_id)
        .execute(&server.state.pool)
        .await
        .expect("restore selected transport");

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
            .request(Method::GET, &credential_path, None, Some(other_access))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    sqlx::query("UPDATE connection_sessions SET accepted_at = 1 WHERE id = $1")
        .bind(session_id)
        .execute(&server.state.pool)
        .await
        .expect("expire native credential");
    assert_eq!(
        server
            .request(Method::GET, &credential_path, None, Some(access))
            .await
            .0,
        StatusCode::CONFLICT
    );

    assert_eq!(
        server
            .request(
                Method::POST,
                "/api/v2/devices/offline",
                Some(json!({"deviceId": host_id})),
                Some(other_access),
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let offline = server
        .request(
            Method::POST,
            "/api/v2/devices/offline",
            Some(json!({"deviceId": host_id})),
            Some(access),
        )
        .await;
    assert_eq!(offline.0, StatusCode::OK, "{}", offline.1);
    assert_eq!(offline.1["online"], false);
    assert_eq!(offline.1["streaming"], false);
    assert!(offline.1["lastSeenAt"].as_i64().is_some());
    let disconnected = server
        .request(
            Method::GET,
            &format!("/api/v2/sessions/{session_id}"),
            None,
            Some(access),
        )
        .await;
    assert_eq!(disconnected.1["state"], "disconnected");
    assert_eq!(disconnected.1["disconnectReason"], "device_offline");
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
    let presence_payload = sqlx::query_scalar::<_, String>(
        "SELECT payload_json FROM connection_events WHERE user_id = $1 AND event_type = 'device.presence' \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(auth["account"]["id"].as_str().expect("account id"))
    .fetch_one(&server.state.pool)
    .await
    .expect("offline presence event");
    let presence: Value = serde_json::from_str(&presence_payload).expect("presence JSON");
    assert_eq!(presence["deviceId"], host_id);
    assert_eq!(presence["online"], false);
    server.close().await;
}

#[tokio::test]
async fn ice_uses_authenticated_short_lived_turn_credentials() {
    let Some(server) = TestServer::with_config(|config| {
        config.network_mode = NetworkMode::Auto;
        config.turn_urls = vec!["turn:relay.example.test:3478?transport=udp".into()];
        config.turn_shared_secret = Some("test-shared-secret".into());
    })
    .await
    else {
        return;
    };
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
    server.close().await;
}

#[tokio::test]
async fn request_guards_return_structured_errors_and_request_ids() {
    let Some(server) = TestServer::new().await else {
        return;
    };
    let health = server
        .request(Method::GET, "/api/v2/health", None, None)
        .await;
    assert_eq!(health.0, StatusCode::OK);
    assert_eq!(health.1["version"], env!("CARGO_PKG_VERSION"));
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

    let Some(limited) =
        TestServer::with_config(|config| config.general_rate_limit_per_minute = 1).await
    else {
        server.close().await;
        return;
    };
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
    limited.close().await;
    server.close().await;
}

#[tokio::test]
async fn migration_and_cleanup_work_on_neon_postgres() {
    let Some(server) = TestServer::new().await else {
        return;
    };
    let migrations = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&server.state.pool)
        .await
        .expect("migration metadata");
    assert_eq!(migrations, 3);
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
    let online = sqlx::query_scalar::<_, bool>("SELECT online FROM devices WHERE id = $1")
        .bind(device_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("device state");
    assert!(!online);
    server.close().await;
}
