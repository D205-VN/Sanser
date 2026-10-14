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

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn computer_registration_is_stable_per_account_and_role() {
    let server = TestServer::new().await.expect("test server");
    let auth = server.register("computer-identity@example.test").await;
    let access = auth["accessToken"].as_str().expect("access token");
    let computer = Uuid::new_v4().to_string();
    let body = |role: &str| {
        json!({ "id": Uuid::new_v4().to_string(), "computerId": computer,
        "deviceRole": role, "platform": "windows-x86_64", "name": "Studio PC" })
    };
    let (first, second) = tokio::join!(
        server.request(
            Method::POST,
            "/api/v2/devices/register",
            Some(body("host")),
            Some(access)
        ),
        server.request(
            Method::POST,
            "/api/v2/devices/register",
            Some(body("host")),
            Some(access)
        )
    );
    assert!(first.0.is_success(), "{}", first.1);
    assert!(second.0.is_success(), "{}", second.1);
    assert_eq!(first.1["id"], second.1["id"]);
    let id = first.1["id"].as_str().expect("host ID");
    let renamed = server
        .request(
            Method::PATCH,
            &format!("/api/v2/devices/{id}"),
            Some(json!({"name":"My office desktop"})),
            Some(access),
        )
        .await;
    assert!(renamed.0.is_success(), "{}", renamed.1);
    let again = server
        .request(
            Method::POST,
            "/api/v2/devices/register",
            Some(body("host")),
            Some(access),
        )
        .await;
    assert_eq!(again.1["id"], id);
    assert_eq!(again.1["name"], "My office desktop");
    let client = server
        .request(
            Method::POST,
            "/api/v2/devices/register",
            Some(body("client")),
            Some(access),
        )
        .await;
    assert!(client.0.is_success(), "{}", client.1);
    assert_ne!(client.1["id"], id);
    assert_eq!(client.1["computerId"], computer);
    let other = server.register("other-computer-account@example.test").await;
    let other_access = other["accessToken"].as_str().expect("other token");
    let foreign = server
        .request(
            Method::POST,
            "/api/v2/devices/register",
            Some(body("host")),
            Some(other_access),
        )
        .await;
    assert!(foreign.0.is_success(), "{}", foreign.1);
    assert_ne!(foreign.1["id"], id);
    server.close().await;
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
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("set TEST_DATABASE_URL to a dedicated Neon test database before running ignored API tests");
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
        Config::test(database_url.clone())
            .expect("TEST_DATABASE_URL must be a dedicated Neon or loopback test URL");

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
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn authentication_expires_after_seven_idle_days_and_activity_renews_it() {
    let Some(server) = TestServer::new().await else {
        return;
    };
    let login = server.register("idle-session@example.test").await;
    let id = login["sessionId"].as_str().expect("session ID");
    let access = login["accessToken"].as_str().expect("access token");
    let refresh = login["refreshToken"].as_str().expect("refresh token");
    let now = chrono::Utc::now().timestamp();
    sqlx::query("UPDATE auth_sessions SET last_seen_at = $1 WHERE id = $2")
        .bind(now - 6 * 24 * 60 * 60)
        .bind(id)
        .execute(&server.state.pool)
        .await
        .expect("set last activity");
    let (status, _, _) = server
        .request(Method::GET, "/api/v2/account", None, Some(access))
        .await;
    assert_eq!(status, StatusCode::OK);
    let last_seen: i64 = sqlx::query_scalar("SELECT last_seen_at FROM auth_sessions WHERE id = $1")
        .bind(id)
        .fetch_one(&server.state.pool)
        .await
        .expect("last activity");
    assert!(last_seen >= now);
    sqlx::query("UPDATE auth_sessions SET last_seen_at = $1 WHERE id = $2")
        .bind(now - sanser_server::auth::SESSION_IDLE_SECONDS)
        .bind(id)
        .execute(&server.state.pool)
        .await
        .expect("expire idle session");
    let (status, _, _) = server
        .request(Method::GET, "/api/v2/account", None, Some(access))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = server
        .request(
            Method::POST,
            "/api/v2/auth/refresh",
            Some(json!({"refreshToken": refresh})),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
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
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
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
                    "platform": if id==&host_id {"windows-x86_64"} else {"macos-aarch64"},
                    "deviceRole": if id==&host_id {"host"} else {"client"},
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

    sqlx::query("UPDATE connection_sessions SET requester_ready_at = 1 WHERE id = $1")
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
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
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
    assert_eq!(response.1["iceTransportPolicy"], "all");
    assert!(response.1["expiresAt"].as_i64().is_some());
    let turn = response.1["iceServers"]
        .as_array()
        .expect("ICE servers")
        .iter()
        .find(|server| {
            server["urls"].as_array().is_some_and(|urls| {
                urls.iter()
                    .any(|url| url.as_str().is_some_and(|url| url.starts_with("turn:")))
            })
        })
        .expect("TURN entry");
    let username = turn["username"].as_str().expect("temporary TURN username");
    assert!(username.contains(':'));
    assert!(
        !turn["credential"]
            .as_str()
            .expect("TURN credential")
            .is_empty()
    );
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
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

    server.close().await;
    let Some(limited) =
        TestServer::with_config(|config| config.general_rate_limit_per_minute = 1).await
    else {
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
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
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

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn sharing_capacity_consent_ownership_and_stop_are_transactional() {
    let server = TestServer::new().await.expect("test server");
    let auth = server.register("sharing-owner@example.test").await;
    let access = auth["accessToken"].as_str().expect("token");
    let foreign = server.register("sharing-foreign@example.test").await;
    let foreign_access = foreign["accessToken"].as_str().expect("foreign token");
    let mut peers = Vec::new();
    for index in 0..5 {
        let id = Uuid::new_v4().to_string();
        let (status, body, _) = server.request(Method::POST, "/api/v2/devices/register", Some(json!({
            "id":id,"name":format!("Sharing peer {index}"),"computerId":Uuid::new_v4().to_string(),
            "deviceRole":if index == 0 {"host"} else {"client"},"platform":if index == 0 {"windows-x86_64"} else {"macos-aarch64"},
            "sanserVersion":"2.1.22","crossPlatform":true,"nativeTransport":true,"codecs":["h264"],"routeAddress":"192.168.1.20"
        })),Some(access)).await;
        assert!(status.is_success(), "{body}");
        let (status, body, _) = server
            .request(
                Method::POST,
                "/api/v2/devices/heartbeat",
                Some(json!({"deviceId":id,"streaming":false})),
                Some(access),
            )
            .await;
        assert!(status.is_success(), "{body}");
        peers.push(id);
    }
    let (status, group, _) = server
        .request(
            Method::POST,
            "/api/v2/sharing",
            Some(json!({"hostDeviceId":peers[0]})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{group}");
    let sharing = group["id"].as_str().expect("sharing ID");
    let path = format!("/api/v2/sharing/{sharing}");
    let (status, _, _) = server
        .request(Method::GET, &path, None, Some(foreign_access))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let invite_path = format!("{path}/invite");
    let (a, b, c, d) = tokio::join!(
        server.request(
            Method::POST,
            &invite_path,
            Some(json!({"viewerDeviceId":peers[1]})),
            Some(access)
        ),
        server.request(
            Method::POST,
            &invite_path,
            Some(json!({"viewerDeviceId":peers[2]})),
            Some(access)
        ),
        server.request(
            Method::POST,
            &invite_path,
            Some(json!({"viewerDeviceId":peers[3]})),
            Some(access)
        ),
        server.request(
            Method::POST,
            &invite_path,
            Some(json!({"viewerDeviceId":peers[4]})),
            Some(access)
        )
    );
    let responses = [a, b, c, d];
    assert_eq!(responses.iter().filter(|r| r.0.is_success()).count(), 3);
    assert_eq!(
        responses
            .iter()
            .filter(|r| r.0 == StatusCode::CONFLICT)
            .count(),
        1
    );
    let (_, invited, _) = server.request(Method::GET, &path, None, Some(access)).await;
    let members = invited["members"].as_array().expect("members");
    assert_eq!(members.len(), 3);
    let member = &members[0];
    let chosen_viewer = member["viewerDeviceId"].as_str().expect("chosen viewer");
    let wrong_viewer = members[1]["viewerDeviceId"].as_str().expect("other viewer");
    assert_eq!(member["accessMode"], "view-only");
    let member_id = member["id"].as_str().expect("member ID");
    let respond_path = format!("{path}/members/{member_id}/respond");
    let (status, _, _) = server
        .request(
            Method::POST,
            &respond_path,
            Some(json!({"viewerDeviceId":wrong_viewer,"accept":true})),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, accepted, _) = server
        .request(
            Method::POST,
            &respond_path,
            Some(json!({"viewerDeviceId":chosen_viewer,"accept":true})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{accepted}");
    assert_eq!(accepted["state"], "active");
    let media_id = accepted["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == member_id)
        .unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, ready, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-ready"),
            Some(json!({"deviceId":chosen_viewer})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{ready}");
    let epoch = ready["requesterReadyAt"].as_i64().unwrap();
    let (status, credential, _) = server
        .request(
            Method::GET,
            &format!(
                "/api/v2/sessions/{media_id}/credentials?deviceId={}",
                peers[0]
            ),
            None,
            Some(access),
        )
        .await;
    assert!(status.is_success());
    assert_eq!(credential["wireProtocol"], "snv2");
    assert_eq!(credential["accessMode"], "view-only");
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-started"),
            Some(json!({"deviceId":wrong_viewer,"requesterReadyAt":epoch})),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-started"),
            Some(json!({"deviceId":chosen_viewer,"requesterReadyAt":epoch})),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mut sessions = vec![media_id.clone()];
    for another in members.iter().skip(1) {
        let (status, response, _) = server
            .request(
                Method::POST,
                &format!("{path}/members/{}/respond", another["id"].as_str().unwrap()),
                Some(json!({"viewerDeviceId":another["viewerDeviceId"],"accept":true})),
                Some(access),
            )
            .await;
        assert!(status.is_success(), "{response}");
        let sid = response["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == another["id"])
            .unwrap()["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!sessions.contains(&sid));
        sessions.push(sid);
    }
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!("{path}/members/{member_id}/remove"),
            None,
            Some(access),
        )
        .await;
    assert!(status.is_success());
    let (_, removed, _) = server
        .request(
            Method::GET,
            &format!("/api/v2/sessions/{media_id}"),
            None,
            Some(access),
        )
        .await;
    assert_eq!(removed["state"], "disconnected");
    for sid in sessions.iter().skip(1) {
        let (_, survivor, _) = server
            .request(
                Method::GET,
                &format!("/api/v2/sessions/{sid}"),
                None,
                Some(access),
            )
            .await;
        assert_eq!(survivor["state"], "accepted");
    }
    let (_, stopped, _) = server
        .request(Method::POST, &format!("{path}/stop"), None, Some(access))
        .await;
    assert_eq!(stopped["state"], "stopped");
    assert!(
        stopped["members"]
            .as_array()
            .expect("members")
            .iter()
            .all(|m| m["state"] == "removed")
    );
    let (status, restarted, _) = server
        .request(
            Method::POST,
            "/api/v2/sharing",
            Some(json!({"hostDeviceId":peers[0]})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{restarted}");
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!(
                "/api/v2/sharing/{}/invite",
                restarted["id"].as_str().expect("new ID")
            ),
            Some(json!({"viewerDeviceId":peers[1]})),
            Some(access),
        )
        .await;
    assert!(status.is_success());
    server.close().await;
}
