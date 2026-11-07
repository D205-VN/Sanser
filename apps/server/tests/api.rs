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
async fn voice_retention_is_bounded_and_never_erases_pending_or_live_rounds() {
    let server = TestServer::new().await.expect("test server");
    let now = chrono::Utc::now().timestamp();
    let old = now - 31 * 24 * 60 * 60;
    for index in 0..105 {
        // 101 expired closed, one active, one pending, one recently closed and
        // one leased closed row. Each room is independent of real accounts.
        let retired = if index == 101 { None } else { Some(old) };
        let closed = match index {
            101 | 102 => None,
            103 => Some(now),
            _ => Some(old),
        };
        sqlx::query("INSERT INTO voice_lab_rooms
            (room_name,sfu_scope,sharing_id,user_id,voice_revision,created_at,retired_at,closed_at,lease_until,lease_id)
            VALUES ($1,$2,$3,$4,0,$5,$6,$7,$8,$9)")
            .bind(format!("snvl_{index:043}"))
            .bind("s".repeat(43)).bind(Uuid::new_v4().to_string()).bind(Uuid::new_v4().to_string())
            .bind(old).bind(retired).bind(closed)
            .bind(if index == 104 { now + 30 } else { 0 })
            .bind(if index == 104 { Some("active-worker") } else { None })
            .execute(&server.state.pool).await.unwrap();
    }
    sqlx::query("INSERT INTO voice_lab_admissions (room_name,identity,user_id,device_id,auth_session_id,created_at)
        VALUES ($1,$2,'synthetic-user','synthetic-device','synthetic-login',$3)")
        .bind(format!("snvl_{:043}",0)).bind(format!("snp_{}","i".repeat(43))).bind(old)
        .execute(&server.state.pool).await.unwrap();
    assert_eq!(
        sanser_server::voice_lab::prune_closed_rooms(&server.state)
            .await
            .unwrap(),
        100
    );
    assert_eq!(
        sanser_server::voice_lab::prune_closed_rooms(&server.state)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sanser_server::voice_lab::prune_closed_rooms(&server.state)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM voice_lab_rooms")
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM voice_lab_admissions")
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        0
    );
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn password_login_replaces_only_the_same_installation_after_verification() {
    let server = TestServer::new().await.expect("test server");
    let installation = Uuid::new_v4().to_string();
    let other_installation = Uuid::new_v4().to_string();
    let registration = json!({
        "email": "installation@example.test", "password": "StrongPass123",
        "deviceName": "Identical computer name", "platform": "macos-aarch64",
        "installationId": installation
    });
    let (status, registered, _) = server
        .request(
            Method::POST,
            "/api/v2/auth/register",
            Some(registration),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{registered}");
    assert_eq!(registered["account"]["displayName"], "installation");
    let original_access = registered["accessToken"].as_str().expect("original access");
    let login = |id: &str, password: &str| {
        json!({
            "email": "installation@example.test", "password": password,
            "deviceName": "Identical computer name", "platform": "macos-aarch64",
            "installationId": id
        })
    };
    let invalid = server
        .request(
            Method::POST,
            "/api/v2/auth/login",
            Some(login(&installation, "incorrect-password")),
            None,
        )
        .await;
    assert_eq!(invalid.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/account", None, Some(original_access))
            .await
            .0,
        StatusCode::OK
    );
    let second = server
        .request(
            Method::POST,
            "/api/v2/auth/login",
            Some(login(&other_installation, "StrongPass123")),
            None,
        )
        .await;
    assert_eq!(second.0, StatusCode::OK, "{}", second.1);
    let second_access = second.1["accessToken"]
        .as_str()
        .expect("second computer access");
    // Two simultaneous logins must not leave two live sessions on the same installation.
    let (a, b) = tokio::join!(
        server.request(
            Method::POST,
            "/api/v2/auth/login",
            Some(login(&installation, "StrongPass123")),
            None
        ),
        server.request(
            Method::POST,
            "/api/v2/auth/login",
            Some(login(&installation, "StrongPass123")),
            None
        )
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/account", None, Some(original_access))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let refresh = server
        .request(
            Method::POST,
            "/api/v2/auth/refresh",
            Some(json!({"refreshToken": registered["refreshToken"]})),
            None,
        )
        .await;
    assert_eq!(refresh.0, StatusCode::UNAUTHORIZED);
    let listed = server
        .request(
            Method::GET,
            "/api/v2/account/sessions",
            None,
            Some(second_access),
        )
        .await;
    assert_eq!(listed.0, StatusCode::OK, "{}", listed.1);
    assert_eq!(listed.1.as_array().expect("sessions").len(), 2);
    assert!(
        listed
            .1
            .as_array()
            .expect("sessions")
            .iter()
            .all(|item| item["deviceName"] == "Identical computer name"
                && item["platform"] == "macos-aarch64")
    );
    // Installation IDs are scoped to the authenticated account, never globally.
    let foreign = server
        .request(
            Method::POST,
            "/api/v2/auth/register",
            Some(json!({
                "email": "other-installation-account@example.test", "password": "StrongPass123",
                "installationId": other_installation
            })),
            None,
        )
        .await;
    assert_eq!(foreign.0, StatusCode::CREATED, "{}", foreign.1);
    assert_eq!(
        server
            .request(Method::GET, "/api/v2/account", None, Some(second_access))
            .await
            .0,
        StatusCode::OK
    );
    // Legacy clients remain supported and are not merged by their generic name.
    for _ in 0..2 {
        let legacy = server
            .request(
                Method::POST,
                "/api/v2/auth/login",
                Some(json!({"email": "installation@example.test", "password": "StrongPass123"})),
                None,
            )
            .await;
        assert_eq!(legacy.0, StatusCode::OK, "{}", legacy.1);
    }
    let listed = server
        .request(
            Method::GET,
            "/api/v2/account/sessions",
            None,
            Some(second_access),
        )
        .await;
    assert_eq!(listed.1.as_array().expect("sessions").len(), 4);
    server.close().await;
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
async fn voice_lab_admission_is_account_membership_and_live_session_scoped() {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let server = TestServer::with_config(|config| {
        config.voice_lab = Some(
            sanser_server::voice_lab::VoiceLabConfig::new(
                "wss://voice.example.test",
                "test_key",
                "test-only-secret-00000000000000000000".into(),
            )
            .unwrap(),
        );
    })
    .await
    .unwrap();
    let auth = server.register("voice-lab@example.test").await;
    let access = auth["accessToken"].as_str().unwrap();
    let foreign = server.register("foreign-voice-lab@example.test").await;
    let foreign_access = foreign["accessToken"].as_str().unwrap();
    let mut devices = Vec::new();
    for role in ["host", "client", "client"] {
        let id = Uuid::new_v4().to_string();
        let (status, body, _) = server
            .request(
                Method::POST,
                "/api/v2/devices/register",
                Some(json!({
                    "id":id,"computerId":Uuid::new_v4().to_string(),"deviceRole":role,
                    "name":"Voice test","platform":"macos-aarch64","sanserVersion":"2.3.3",
                    "crossPlatform":true,"nativeTransport":true,"codecs":["h264"]
                })),
                Some(access),
            )
            .await;
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
        devices.push(id);
    }
    let (status, group, _) = server
        .request(
            Method::POST,
            "/api/v2/sharing",
            Some(json!({"hostDeviceId":devices[0]})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{group}");
    let group_id = group["id"].as_str().unwrap();
    let path = format!("/api/v2/sharing/{group_id}");
    let credentials = format!("{path}/voice-lab/credentials");
    let host_body = json!({"deviceId":devices[0]});
    assert_eq!(
        server
            .request(Method::POST, &credentials, Some(host_body.clone()), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        server
            .request(
                Method::POST,
                &credentials,
                Some(host_body.clone()),
                Some(foreign_access)
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .request(
                Method::POST,
                &credentials,
                Some(json!({"deviceId":devices[2]})),
                Some(access)
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        server
            .request(
                Method::POST,
                &credentials,
                Some(json!({"deviceId":devices[0],"room":"other-room"})),
                Some(access)
            )
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, host_grant, headers) = server
        .request(
            Method::POST,
            &credentials,
            Some(host_body.clone()),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{host_grant}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let decode = |grant: &Value| -> Value {
        let token = grant["token"].as_str().unwrap();
        serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(token.split('.').nth(1).unwrap())
                .unwrap(),
        )
        .unwrap()
    };
    let host_claims = decode(&host_grant);
    assert_eq!(
        host_claims["video"]["canPublishSources"],
        json!(["microphone"])
    );
    assert_eq!(
        host_claims["exp"].as_i64().unwrap() - host_claims["iat"].as_i64().unwrap(),
        60
    );
    assert!(!host_claims.to_string().contains(group_id));

    let (status, invited, _) = server
        .request(
            Method::POST,
            &format!("{path}/invite"),
            Some(json!({"viewerDeviceId":devices[1]})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{invited}");
    let member = invited["members"][0]["id"].as_str().unwrap();
    let viewer_body = json!({"deviceId":devices[1]});
    assert_eq!(
        server
            .request(
                Method::POST,
                &credentials,
                Some(viewer_body.clone()),
                Some(access)
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, accepted, _) = server
        .request(
            Method::POST,
            &format!("{path}/members/{member}/respond"),
            Some(json!({"viewerDeviceId":devices[1],"accept":true})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{accepted}");
    let session = accepted["members"][0]["sessionId"].as_str().unwrap();
    let (status, viewer_grant, _) = server
        .request(
            Method::POST,
            &credentials,
            Some(viewer_body.clone()),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{viewer_grant}");
    let viewer_claims = decode(&viewer_grant);
    assert_eq!(viewer_claims["video"]["room"], host_claims["video"]["room"]);
    assert_ne!(viewer_claims["sub"], host_claims["sub"]);
    assert_eq!(host_grant["roomRevision"], viewer_grant["roomRevision"]);
    // An accepted membership alone cannot keep a disconnected media session alive.
    sqlx::query("UPDATE connection_sessions SET state='disconnected' WHERE id=$1")
        .bind(session)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        server
            .request(
                Method::POST,
                &credentials,
                Some(viewer_body.clone()),
                Some(access)
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE connection_sessions SET state='accepted' WHERE id=$1")
        .bind(session)
        .execute(&server.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE devices SET last_seen_at=0 WHERE id=$1")
        .bind(&devices[1])
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        server
            .request(
                Method::POST,
                &credentials,
                Some(viewer_body.clone()),
                Some(access)
            )
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    server
        .request(
            Method::POST,
            "/api/v2/devices/heartbeat",
            Some(json!({"deviceId":devices[1],"streaming":false})),
            Some(access),
        )
        .await;
    // Admission must wait for a concurrent removal, then evaluate membership in
    // a fresh statement. It must not combine a new revision with old permission.
    {
        let mut removal = server.state.pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM sharing_groups WHERE id=$1 FOR UPDATE")
            .bind(group_id)
            .fetch_one(&mut *removal)
            .await
            .unwrap();
        sqlx::query("UPDATE sharing_members SET state='removed' WHERE id=$1")
            .bind(member)
            .execute(&mut *removal)
            .await
            .unwrap();
        let racing_grant = server.request(
            Method::POST,
            &credentials,
            Some(viewer_body.clone()),
            Some(access),
        );
        tokio::pin!(racing_grant);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut racing_grant)
                .await
                .is_err()
        );
        removal.commit().await.unwrap();
        assert_eq!(racing_grant.await.0, StatusCode::FORBIDDEN);
    }
    let (status, body, _) = server
        .request(
            Method::POST,
            &format!("{path}/members/{member}/remove"),
            None,
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{body}");
    assert_eq!(
        server
            .request(Method::POST, &credentials, Some(viewer_body), Some(access))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, new_host_grant, _) = server
        .request(
            Method::POST,
            &credentials,
            Some(host_body.clone()),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let new_host_claims = decode(&new_host_grant);
    assert_ne!(
        host_claims["video"]["room"], new_host_claims["video"]["room"],
        "a removed viewer's old token must not authorize the next voice room"
    );
    assert_ne!(host_grant["roomRevision"], new_host_grant["roomRevision"]);
    let revision =
        sqlx::query_scalar::<_, i64>("SELECT voice_revision FROM sharing_groups WHERE id=$1")
            .bind(group_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap();
    // Repeated removal is idempotent, and ordinary status updates do not change
    // voice rounds. Trigger tests also cover direct cleanup/delete operations.
    server
        .request(
            Method::POST,
            &format!("{path}/members/{member}/remove"),
            None,
            Some(access),
        )
        .await;
    sqlx::query("UPDATE sharing_groups SET updated_at=updated_at WHERE id=$1")
        .bind(group_id)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT voice_revision FROM sharing_groups WHERE id=$1")
            .bind(group_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        revision
    );
    sqlx::query("UPDATE sharing_members SET state='accepted' WHERE id=$1")
        .bind(member)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT voice_revision FROM sharing_groups WHERE id=$1")
            .bind(group_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        revision,
        "adding a newly accepted viewer must not interrupt the existing round"
    );
    sqlx::query("UPDATE sharing_members SET session_id=NULL WHERE id=$1")
        .bind(member)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT voice_revision FROM sharing_groups WHERE id=$1")
            .bind(group_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        revision + 1
    );
    sqlx::query("DELETE FROM sharing_members WHERE id=$1")
        .bind(member)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT voice_revision FROM sharing_groups WHERE id=$1")
            .bind(group_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        revision + 2
    );
    server
        .request(Method::POST, &format!("{path}/stop"), None, Some(access))
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT voice_revision FROM sharing_groups WHERE id=$1")
            .bind(group_id)
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        revision + 3
    );
    assert_eq!(
        server
            .request(Method::POST, &credentials, Some(host_body), Some(access))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn voice_lab_room_closures_survive_failure_restart_and_expired_worker_leases() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let upstream = Router::new().route(
        "/twirp/livekit.RoomService/DeleteRoom",
        axum::routing::post(move |axum::Json(_): axum::Json<Value>| {
            let seen = seen.clone();
            async move {
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(json!({"code":"unavailable"})),
                    )
                } else {
                    (StatusCode::OK, axum::Json(json!({})))
                }
            }
        }),
    );
    let stop = tokio_util::sync::CancellationToken::new();
    let stopping = stop.clone();
    let upstream_task = tokio::spawn(async move {
        axum::serve(listener, upstream)
            .with_graceful_shutdown(stopping.cancelled_owned())
            .await
            .unwrap();
    });
    let server = TestServer::with_config(|config| {
        config.voice_lab = Some(
            sanser_server::voice_lab::VoiceLabConfig::new(
                &format!("ws://{address}"),
                "test_key",
                "a".repeat(32),
            )
            .unwrap(),
        );
    })
    .await
    .unwrap();
    let auth = server.register("voice-closure@example.test").await;
    let access = auth["accessToken"].as_str().unwrap();
    let host = Uuid::new_v4().to_string();
    let (status,body,_)=server.request(Method::POST,"/api/v2/devices/register",Some(json!({
        "id":host,"computerId":Uuid::new_v4().to_string(),"deviceRole":"host","name":"Closure test","platform":"macos-aarch64",
        "sanserVersion":"2.3.3","crossPlatform":true,"nativeTransport":true,"codecs":["h264"]
    })),Some(access)).await;
    assert!(status.is_success(), "{body}");
    server
        .request(
            Method::POST,
            "/api/v2/devices/heartbeat",
            Some(json!({"deviceId":host,"streaming":false})),
            Some(access),
        )
        .await;
    let (status, group, _) = server
        .request(
            Method::POST,
            "/api/v2/sharing",
            Some(json!({"hostDeviceId":host})),
            Some(access),
        )
        .await;
    assert!(status.is_success(), "{group}");
    let group = group["id"].as_str().unwrap();
    let path = format!("/api/v2/sharing/{group}/voice-lab/credentials");
    let (status, _, _) = server
        .request(
            Method::POST,
            &path,
            Some(json!({"deviceId":host})),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let old_room = sqlx::query_scalar::<_, String>("SELECT room_name FROM voice_lab_rooms")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    // Reissuing the same round cannot enqueue duplicate rooms.
    server
        .request(
            Method::POST,
            &path,
            Some(json!({"deviceId":host})),
            Some(access),
        )
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM voice_lab_rooms")
            .fetch_one(&server.state.pool)
            .await
            .unwrap(),
        1
    );
    let active = sanser_server::voice_lab::close_retired_rooms(&server.state)
        .await
        .unwrap();
    assert_eq!(active.closed, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    sqlx::query("UPDATE sharing_groups SET voice_revision=voice_revision+1 WHERE id=$1")
        .bind(group)
        .execute(&server.state.pool)
        .await
        .unwrap();
    server
        .request(
            Method::POST,
            &path,
            Some(json!({"deviceId":host})),
            Some(access),
        )
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM voice_lab_rooms WHERE retired_at IS NULL"
        )
        .fetch_one(&server.state.pool)
        .await
        .unwrap(),
        1
    );
    let failed = sanser_server::voice_lab::close_retired_rooms(&server.state)
        .await
        .unwrap();
    assert_eq!(failed.deferred, 1);
    assert_eq!(failed.closed, 0);
    assert!(sqlx::query_scalar::<_,bool>("SELECT closed_at IS NULL AND next_attempt_at>0 AND lease_until=0 FROM voice_lab_rooms WHERE room_name=$1").bind(&old_room).fetch_one(&server.state.pool).await.unwrap());
    assert_eq!(
        sanser_server::voice_lab::close_retired_rooms(&server.state)
            .await
            .unwrap()
            .closed,
        0
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    sqlx::query("UPDATE voice_lab_rooms SET next_attempt_at=0 WHERE room_name=$1")
        .bind(&old_room)
        .execute(&server.state.pool)
        .await
        .unwrap();
    let restarted = AppState::new((*server.state.config).clone(), server.state.pool.clone());
    assert_eq!(
        sanser_server::voice_lab::close_retired_rooms(&restarted)
            .await
            .unwrap()
            .closed,
        1
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        sanser_server::voice_lab::close_retired_rooms(&restarted)
            .await
            .unwrap()
            .closed,
        0
    );
    // Simulate a worker dying with a claimed job. It remains unavailable until
    // its lease expires, then concurrent workers send one idempotent close.
    sqlx::query("UPDATE voice_lab_rooms SET closed_at=NULL,lease_until=9223372036854775807,lease_id='crashed' WHERE room_name=$1").bind(&old_room).execute(&server.state.pool).await.unwrap();
    assert_eq!(
        sanser_server::voice_lab::close_retired_rooms(&restarted)
            .await
            .unwrap()
            .closed,
        0
    );
    sqlx::query("UPDATE voice_lab_rooms SET lease_until=0 WHERE room_name=$1")
        .bind(&old_room)
        .execute(&server.state.pool)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        sanser_server::voice_lab::close_retired_rooms(&restarted),
        sanser_server::voice_lab::close_retired_rooms(&server.state)
    );
    assert_eq!(a.unwrap().closed + b.unwrap().closed, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    // Keep durable deletion information even when the group itself is deleted.
    sqlx::query("DELETE FROM sharing_groups WHERE id=$1")
        .bind(group)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM voice_lab_rooms WHERE retired_at IS NOT NULL"
        )
        .fetch_one(&server.state.pool)
        .await
        .unwrap(),
        2
    );
    let closed = sanser_server::voice_lab::close_retired_rooms(&restarted)
        .await
        .unwrap();
    assert_eq!(closed.closed, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    stop.cancel();
    upstream_task.await.unwrap();
    server.close().await;
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
    assert_eq!(
        migrations,
        i64::try_from(sqlx::migrate!("./migrations").iter().count()).expect("migration count")
    );
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
        assert_eq!(
            body["sanserVersion"], "2.1.22",
            "registration must retain the client version, not the server version"
        );
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
    server
        .request(
            Method::POST,
            "/api/v2/devices/offline",
            Some(json!({"deviceId":peers[4]})),
            Some(access),
        )
        .await;
    let (status, _, _) = server
        .request(
            Method::POST,
            &invite_path,
            Some(json!({"viewerDeviceId":peers[4]})),
            Some(access),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an offline registration must not receive a new invitation"
    );
    server
        .request(
            Method::POST,
            "/api/v2/devices/heartbeat",
            Some(json!({"deviceId":peers[4],"streaming":false})),
            Some(access),
        )
        .await;
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
    let streaming = sqlx::query_scalar::<_, bool>("SELECT streaming FROM devices WHERE id=$1")
        .bind(&peers[0])
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert!(
        streaming,
        "only an acknowledged decoded frame marks the host streaming"
    );
    let (status, refreshed, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-ready"),
            Some(json!({"deviceId":chosen_viewer})),
            Some(access),
        )
        .await;
    assert!(status.is_success());
    let next_epoch = refreshed["requesterReadyAt"].as_i64().unwrap();
    assert!(next_epoch > epoch);
    let streaming = sqlx::query_scalar::<_, bool>("SELECT streaming FROM devices WHERE id=$1")
        .bind(&peers[0])
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert!(
        !streaming,
        "a retried generation has not decoded a frame yet"
    );
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-started"),
            Some(json!({"deviceId":chosen_viewer,"requesterReadyAt":epoch})),
            Some(access),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "stale decoder reports cannot revive a previous generation"
    );
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-started"),
            Some(json!({"deviceId":chosen_viewer,"requesterReadyAt":next_epoch})),
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
    let streaming = sqlx::query_scalar::<_, bool>("SELECT streaming FROM devices WHERE id=$1")
        .bind(&peers[0])
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert!(
        !streaming,
        "accepted viewers without decoded frames do not count as watching"
    );
    let (status, _, _) = server
        .request(
            Method::POST,
            &format!("/api/v2/sessions/{media_id}/native-started"),
            Some(json!({"deviceId":chosen_viewer,"requesterReadyAt":next_epoch})),
            Some(access),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "removal invalidates subsequent first-frame reports"
    );
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

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn voice_gateway_denies_cached_admission_after_offline_logout_and_stop() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let upstream = Router::new()
        .route(
            "/rtc/validate",
            axum::routing::get(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    "success"
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/DeleteRoom",
            axum::routing::post(|| async { axum::Json(json!({})) }),
        );
    let stop = tokio_util::sync::CancellationToken::new();
    let stopping = stop.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, upstream)
            .with_graceful_shutdown(stopping.cancelled_owned())
            .await
            .unwrap();
    });
    let server = TestServer::with_config(|config| {
        config.voice_lab = Some(
            sanser_server::voice_lab::VoiceLabConfig::new(
                &format!("ws://{address}"),
                "test_key",
                "a".repeat(32),
            )
            .unwrap()
            .with_gateway("wss://gateway.example.test")
            .unwrap(),
        );
    })
    .await
    .unwrap();
    let auth = server.register("gateway@example.test").await;
    let access = auth["accessToken"].as_str().unwrap();
    let device = Uuid::new_v4().to_string();
    let registered=server.request(Method::POST,"/api/v2/devices/register",Some(json!({"id":device,"computerId":Uuid::new_v4().to_string(),"deviceRole":"host","name":"Synthetic","platform":"macos-aarch64","sanserVersion":"2.3.3","crossPlatform":true,"nativeTransport":true,"codecs":["h264"]})),Some(access)).await;
    assert!(registered.0.is_success());
    server
        .request(
            Method::POST,
            "/api/v2/devices/heartbeat",
            Some(json!({"deviceId":device,"streaming":false})),
            Some(access),
        )
        .await;
    let group = server
        .request(
            Method::POST,
            "/api/v2/sharing",
            Some(json!({"hostDeviceId":device})),
            Some(access),
        )
        .await
        .1;
    let id = group["id"].as_str().unwrap();
    let path = format!("/api/v2/sharing/{id}/voice-lab/credentials");
    let grant = server
        .request(
            Method::POST,
            &path,
            Some(json!({"deviceId":device})),
            Some(access),
        )
        .await
        .1;
    assert_eq!(grant["url"], "wss://gateway.example.test/");
    let token = grant["token"].as_str().unwrap();
    let probe = |token: &str| {
        let app = server.app.clone();
        let path = format!("/rtc/validate?access_token={token}");
        async move {
            app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(probe(token).await, StatusCode::OK);
    assert_eq!(
        probe(&format!("{token}&access_token={token}")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    sqlx::query("UPDATE devices SET online=FALSE WHERE id=$1")
        .bind(&device)
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(probe(token).await, StatusCode::FORBIDDEN);
    server
        .request(
            Method::POST,
            "/api/v2/devices/heartbeat",
            Some(json!({"deviceId":device,"streaming":false})),
            Some(access),
        )
        .await;
    assert_eq!(probe(token).await, StatusCode::OK);
    server
        .request(
            Method::POST,
            "/api/v2/auth/logout",
            Some(json!({"refreshToken":auth["refreshToken"]})),
            Some(access),
        )
        .await;
    assert_eq!(probe(token).await, StatusCode::FORBIDDEN);
    assert_eq!(
        sanser_server::voice_lab::close_retired_rooms(&server.state)
            .await
            .unwrap()
            .closed,
        1
    );
    let again = server
        .request(
            Method::POST,
            "/api/v2/auth/login",
            Some(json!({"email":"gateway@example.test","password":"StrongPass123"})),
            None,
        )
        .await
        .1;
    let new_access = again["accessToken"].as_str().unwrap();
    let next = server
        .request(
            Method::POST,
            &path,
            Some(json!({"deviceId":device})),
            Some(new_access),
        )
        .await;
    assert_eq!(next.0, StatusCode::OK);
    let next = next.1["token"].as_str().unwrap();
    assert_eq!(probe(next).await, StatusCode::OK);
    assert_eq!(probe(token).await, StatusCode::FORBIDDEN);
    server
        .request(
            Method::POST,
            &format!("/api/v2/sharing/{id}/stop"),
            Some(json!({})),
            Some(new_access),
        )
        .await;
    assert_eq!(probe(next).await, StatusCode::FORBIDDEN);
    assert_eq!(count.load(Ordering::SeqCst), 3);
    stop.cancel();
    task.await.unwrap();
    server.close().await;
}
