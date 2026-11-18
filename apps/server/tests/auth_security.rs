//! Two-step verification and guest access: every rejection path.
//! Runs against an isolated schema in TEST_DATABASE_URL, like tests/api.rs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use sanser_auth::TotpSecret;
use sanser_server::{
    build_router,
    config::{Config, PROTOCOL_VERSION, SANSER_VERSION},
    db,
    state::{AppState, RateLimiter},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "StrongPass123";

struct TestServer {
    app: Router,
    state: AppState,
    admin_pool: PgPool,
    schema: String,
    _permit: OwnedSemaphorePermit,
}

impl TestServer {
    /// `generous_limits` lifts the per-account code limiter so a test can walk
    /// through many rejection paths; the limiter itself has its own test.
    async fn new(generous_limits: bool) -> Self {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .expect("set TEST_DATABASE_URL to a dedicated test database");
        static LOCK: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let permit = LOCK
            .get_or_init(|| Arc::new(Semaphore::new(1)))
            .clone()
            .acquire_owned()
            .await
            .unwrap();
        let mut admin_url = url::Url::parse(&database_url).unwrap();
        let pairs: Vec<(String, String)> = admin_url
            .query_pairs()
            .filter(|(key, _)| key != "channel_binding")
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        admin_url.set_query(None);
        admin_url.query_pairs_mut().extend_pairs(pairs);
        let admin_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(admin_url.as_str())
            .await
            .unwrap();
        let schema = format!("sanser_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
            .execute(&admin_pool)
            .await
            .unwrap();
        let mut scoped = url::Url::parse(&database_url).unwrap();
        scoped
            .query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let mut config = Config::test(scoped.into()).unwrap();
        config.login_rate_limit_per_ten_minutes = 1_000;
        let pool = db::connect(&config).await.unwrap();
        let mut state = AppState::new(config, pool);
        if generous_limits {
            state.two_factor_limiter = RateLimiter::new(1_000, Duration::from_secs(600));
        }
        Self {
            app: build_router(state.clone()),
            state,
            admin_pool,
            schema,
            _permit: permit,
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = match body {
            Some(value) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&value).unwrap())
            }
            None => Body::empty(),
        };
        let response = self
            .app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, value)
    }

    async fn post(&self, path: &str, body: Value, token: Option<&str>) -> (StatusCode, Value) {
        self.call(Method::POST, path, Some(body), token).await
    }

    async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        self.call(Method::GET, path, None, Some(token)).await
    }

    async fn register(&self, email: &str) -> Value {
        let (status, body) = self
            .post(
                "/api/v2/auth/register",
                json!({"email": email, "password": PASSWORD, "deviceName": "Test", "platform": "test"}),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body
    }

    async fn login(&self, email: &str) -> (StatusCode, Value) {
        self.post(
            "/api/v2/auth/login",
            json!({"email": email, "password": PASSWORD, "deviceName": "Test", "platform": "test"}),
            None,
        )
        .await
    }

    async fn close(self) {
        self.state.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA \"{}\" CASCADE", self.schema))
            .execute(&self.admin_pool)
            .await
            .unwrap();
        self.admin_pool.close().await;
    }
}

fn token(value: &Value) -> String {
    value["accessToken"]
        .as_str()
        .expect("access token")
        .to_owned()
}

fn decode_base32(value: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = Vec::new();
    let (mut buffer, mut bits) = (0_u32, 0_u32);
    for byte in value.bytes() {
        let index = ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .unwrap();
        buffer = (buffer << 5) | u32::try_from(index).unwrap();
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push(u8::try_from((buffer >> bits) & 0xff).unwrap());
        }
    }
    output
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn code(secret: &TotpSecret, offset_steps: i64) -> String {
    format!("{:06}", secret.code_at_step(now() / 30 + offset_steps))
}

/// Lets a test use the current step again without waiting 30 seconds.
async fn forget_used_step(server: &TestServer, user_id: &str) {
    sqlx::query("UPDATE user_two_factor SET last_used_step = NULL WHERE user_id = $1")
        .bind(user_id)
        .execute(&server.state.pool)
        .await
        .unwrap();
}

/// Enables 2FA and returns (secret, recovery codes).
async fn enable_two_factor(server: &TestServer, access: &str) -> (TotpSecret, Vec<String>) {
    let (status, enrolled) = server
        .post(
            "/api/v2/account/two-factor/enroll",
            json!({"password": PASSWORD}),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enrolled}");
    let secret = TotpSecret::from_bytes(decode_base32(enrolled["secret"].as_str().unwrap()));
    assert!(
        enrolled["otpauthUri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Sanser:")
    );
    let (status, confirmed) = server
        .post(
            "/api/v2/account/two-factor/confirm",
            json!({"code": code(&secret, 0)}),
            Some(access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    let codes = confirmed["recoveryCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect();
    (secret, codes)
}

async fn challenge(server: &TestServer, email: &str) -> String {
    let (status, body) = server.login(email).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"]["code"], "two_factor_required");
    assert!(body.get("accessToken").is_none());
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Update Sanser")
    );
    body["error"]["challengeToken"].as_str().unwrap().to_owned()
}

async fn complete(
    server: &TestServer,
    challenge: &str,
    field: &str,
    value: &str,
) -> (StatusCode, Value) {
    server
        .post(
            "/api/v2/auth/login/two-factor",
            json!({"challengeToken": challenge, field: value}),
            None,
        )
        .await
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn two_factor_enrollment_rejects_bad_password_bad_code_and_double_enable() {
    let server = TestServer::new(true).await;
    let auth = server.register("enroll@example.test").await;
    let access = token(&auth);
    let other_device = token(&server.login("enroll@example.test").await.1);
    let other_refresh = server.login("enroll@example.test").await.1["refreshToken"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, body) = server
        .post(
            "/api/v2/account/two-factor/enroll",
            json!({"password": "WrongPass123"}),
            Some(&access),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (status, _) = server
        .post(
            "/api/v2/account/two-factor/confirm",
            json!({"code": "123456"}),
            Some(&access),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "confirm before enroll");

    let (status, enrolled) = server
        .post(
            "/api/v2/account/two-factor/enroll",
            json!({"password": PASSWORD}),
            Some(&access),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let secret = TotpSecret::from_bytes(decode_base32(enrolled["secret"].as_str().unwrap()));
    // Stored only encrypted.
    let stored: String = sqlx::query_scalar("SELECT secret_envelope FROM user_two_factor")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert!(!stored.contains(enrolled["secret"].as_str().unwrap()));
    for bad in ["000000", "12345", "abcdef", &code(&secret, -3)] {
        let (status, body) = server
            .post(
                "/api/v2/account/two-factor/confirm",
                json!({"code": bad}),
                Some(&access),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {body}");
    }
    // Not enabled yet: login still returns tokens directly.
    assert_eq!(server.login("enroll@example.test").await.0, StatusCode::OK);
    let (status, confirmed) = server
        .post(
            "/api/v2/account/two-factor/confirm",
            json!({"code": code(&secret, 0)}),
            Some(&access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    let codes = confirmed["recoveryCodes"].as_array().unwrap();
    assert_eq!(codes.len(), 10);
    let hashes: Vec<String> = sqlx::query_scalar("SELECT code_hash FROM two_factor_recovery_codes")
        .fetch_all(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(hashes.len(), 10);
    assert!(hashes.iter().all(|hash| hash.starts_with("$argon2id$")));

    // Enabling signs out password-only sessions, including their refresh tokens.
    assert_eq!(
        server.get("/api/v2/account", &other_device).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _) = server
        .post(
            "/api/v2/auth/refresh",
            json!({"refreshToken": other_refresh}),
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "refresh must not bypass 2FA"
    );
    assert_eq!(
        server.get("/api/v2/account", &access).await.0,
        StatusCode::OK
    );

    let (status, _) = server
        .post(
            "/api/v2/account/two-factor/enroll",
            json!({"password": PASSWORD}),
            Some(&access),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "already enabled");
    let (status, state) = server.get("/api/v2/account/two-factor", &access).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state["enabled"], true);
    assert_eq!(state["recoveryCodesRemaining"], 10);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn two_factor_login_rejects_bad_expired_reused_and_cross_account_codes() {
    let server = TestServer::new(true).await;
    let alice = server.register("alice-2fa@example.test").await;
    let bob = server.register("bob-2fa@example.test").await;
    let alice_id = alice["account"]["id"].as_str().unwrap().to_owned();
    let (alice_secret, alice_codes) = enable_two_factor(&server, &token(&alice)).await;
    let (bob_secret, bob_codes) = enable_two_factor(&server, &token(&bob)).await;
    forget_used_step(&server, &alice_id).await;

    let pending = challenge(&server, "alice-2fa@example.test").await;
    // Malformed and unknown challenge tokens.
    for bad in [
        "",
        "sn2_c_short",
        "sn2_a_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert_eq!(
            complete(&server, bad, "code", &code(&alice_secret, 0))
                .await
                .0,
            StatusCode::UNAUTHORIZED,
            "{bad}"
        );
    }
    let unknown = format!("sn2_c_{}", "A".repeat(43));
    assert_eq!(
        complete(&server, &unknown, "code", &code(&alice_secret, 0))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // Neither or both factors.
    let (status, _) = server
        .post(
            "/api/v2/auth/login/two-factor",
            json!({"challengeToken": pending}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = server
        .post(
            "/api/v2/auth/login/two-factor",
            json!({"challengeToken": pending, "code": "123456", "recoveryCode": alice_codes[0]}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // Wrong account: Bob's code and Bob's recovery code do not open Alice's challenge.
    assert_eq!(
        complete(&server, &pending, "code", &code(&bob_secret, 0))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        complete(&server, &pending, "recoveryCode", &bob_codes[0])
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Stale code outside the drift window.
    assert_eq!(
        complete(&server, &pending, "code", &code(&alice_secret, -3))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Success.
    let (status, tokens) = complete(&server, &pending, "code", &code(&alice_secret, 0)).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(
        server.get("/api/v2/account", &token(&tokens)).await.0,
        StatusCode::OK
    );
    // The challenge is single use.
    assert_eq!(
        complete(&server, &pending, "code", &code(&alice_secret, 1))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // The same TOTP code cannot be replayed on a fresh challenge.
    let second = challenge(&server, "alice-2fa@example.test").await;
    assert_eq!(
        complete(&server, &second, "code", &code(&alice_secret, 0))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Recovery codes work once, in any case and with or without the dash.
    let typed = alice_codes[0].to_ascii_lowercase().replace('-', "");
    let (status, _) = complete(&server, &second, "recoveryCode", &typed).await;
    assert_eq!(status, StatusCode::OK);
    let third = challenge(&server, "alice-2fa@example.test").await;
    assert_eq!(
        complete(&server, &third, "recoveryCode", &alice_codes[0])
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (_, state) = server
        .get("/api/v2/account/two-factor", &token(&tokens))
        .await;
    assert_eq!(state["recoveryCodesRemaining"], 9);

    // Expired challenge.
    sqlx::query("UPDATE two_factor_challenges SET expires_at = $1 WHERE consumed_at IS NULL")
        .bind(now() - 1)
        .execute(&server.state.pool)
        .await
        .unwrap();
    forget_used_step(&server, &alice_id).await;
    assert_eq!(
        complete(&server, &third, "code", &code(&alice_secret, 0))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    // Wrong password never yields a challenge.
    let (status, body) = server
        .post(
            "/api/v2/auth/login",
            json!({"email": "alice-2fa@example.test", "password": "WrongPass123"}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    assert!(body["error"].get("challengeToken").is_none());
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn two_factor_challenge_and_account_limits_stop_brute_force() {
    let server = TestServer::new(true).await;
    let auth = server.register("brute@example.test").await;
    let user_id = auth["account"]["id"].as_str().unwrap().to_owned();
    let (secret, _) = enable_two_factor(&server, &token(&auth)).await;
    forget_used_step(&server, &user_id).await;
    let pending = challenge(&server, "brute@example.test").await;
    for guess in 0..5 {
        let (status, _) = complete(&server, &pending, "code", &format!("{guess:06}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    // The sixth attempt is refused even with the right code.
    assert_eq!(
        complete(&server, &pending, "code", &code(&secret, 0))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    server.close().await;

    // With the default per-account limit, fresh challenges do not help either.
    let server = TestServer::new(false).await;
    let auth = server.register("brute2@example.test").await;
    let user_id = auth["account"]["id"].as_str().unwrap().to_owned();
    let (secret, _) = enable_two_factor(&server, &token(&auth)).await;
    forget_used_step(&server, &user_id).await;
    let mut limited = false;
    for _ in 0..4 {
        let pending = challenge(&server, "brute2@example.test").await;
        for guess in 0..5 {
            if complete(&server, &pending, "code", &format!("{guess:06}"))
                .await
                .0
                == StatusCode::TOO_MANY_REQUESTS
            {
                limited = true;
            }
        }
    }
    assert!(limited, "per-account limiter must trip");
    let pending = challenge(&server, "brute2@example.test").await;
    assert_eq!(
        complete(&server, &pending, "code", &code(&secret, 0))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn disabling_two_factor_requires_password_and_a_valid_code() {
    let server = TestServer::new(true).await;
    let auth = server.register("disable@example.test").await;
    let access = token(&auth);
    let user_id = auth["account"]["id"].as_str().unwrap().to_owned();
    let (secret, codes) = enable_two_factor(&server, &access).await;
    forget_used_step(&server, &user_id).await;
    let path = "/api/v2/account/two-factor/disable";
    for body in [
        json!({"password": PASSWORD}),
        json!({"password": "WrongPass123", "code": code(&secret, 0)}),
        json!({"password": PASSWORD, "code": "000000"}),
        json!({"password": PASSWORD, "recoveryCode": "AAAAA-AAAAA"}),
    ] {
        let (status, response) = server.post(path, body.clone(), Some(&access)).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}: {response}"
        );
    }
    assert_eq!(
        server
            .post(path, json!({"code": code(&secret, 0)}), Some(&access))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Regenerating recovery codes invalidates the old ones.
    let (status, fresh) = server
        .post(
            "/api/v2/account/two-factor/recovery-codes",
            json!({"password": PASSWORD, "recoveryCode": codes[1]}),
            Some(&access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fresh}");
    let (status, _) = server
        .post(
            path,
            json!({"password": PASSWORD, "recoveryCode": codes[2]}),
            Some(&access),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "old recovery code"
    );
    let new_code = fresh["recoveryCodes"][0].as_str().unwrap();
    assert_eq!(
        server
            .post(
                path,
                json!({"password": PASSWORD, "recoveryCode": new_code}),
                Some(&access)
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(server.login("disable@example.test").await.0, StatusCode::OK);
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM two_factor_recovery_codes")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    // Disabling twice is a conflict, not a silent success.
    assert_eq!(
        server
            .post(
                path,
                json!({"password": PASSWORD, "code": code(&secret, 1)}),
                Some(&access)
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    server.close().await;
}

struct GuestFixture {
    host_access: String,
    host_device: String,
}

async fn host_fixture(server: &TestServer, email: &str) -> GuestFixture {
    let auth = server.register(email).await;
    let host_access = token(&auth);
    let host_device = Uuid::new_v4().to_string();
    let (status, body) = server
        .post(
            "/api/v2/devices/register",
            json!({
                "id": host_device, "name": "Office PC", "platform": "windows-x86_64",
                "deviceRole": "host", "routeAddress": "192.168.1.30",
                "nativeTransport": true, "webrtc": true, "codecs": ["h264"]
            }),
            Some(&host_access),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    GuestFixture {
        host_access,
        host_device,
    }
}

async fn create_grant(server: &TestServer, fixture: &GuestFixture) -> Value {
    let (status, grant) = server
        .post(
            "/api/v2/guest/grants",
            json!({"hostDeviceId": fixture.host_device}),
            Some(&fixture.host_access),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    grant
}

fn redeem_body(grant: &Value, password: &str) -> Value {
    json!({
        "accessCode": grant["accessCode"], "password": password, "guestName": "Helper",
        "device": {
            "platform": "macos-aarch64", "version": SANSER_VERSION, "protocolVersion": PROTOCOL_VERSION,
            "codecs": ["h264"], "nativeTransport": true, "webRtc": true, "routeAddress": "192.168.1.20"
        }
    })
}

async fn redeem(server: &TestServer, grant: &Value, password: &str) -> (StatusCode, Value) {
    server
        .post("/api/v2/guest/redeem", redeem_body(grant, password), None)
        .await
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn guest_codes_reject_wrong_unknown_expired_reused_and_revoked() {
    let server = TestServer::new(true).await;
    let fixture = host_fixture(&server, "guest-host@example.test").await;
    let grant = create_grant(&server, &fixture).await;
    let password = grant["password"].as_str().unwrap().to_owned();
    assert_eq!(grant["accessCode"].as_str().unwrap().len(), 11);

    // Unknown code, malformed code, wrong password: one indistinct answer.
    let mut unknown = grant.clone();
    let digits = grant["accessCode"].as_str().unwrap().replace(' ', "");
    let flipped = (digits.parse::<u64>().unwrap() + 1) % 1_000_000_000;
    unknown["accessCode"] = json!(format!("{flipped:09}"));
    let (status, unknown_body) = redeem(&server, &unknown, &password).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut malformed = grant.clone();
    malformed["accessCode"] = json!("12345");
    assert_eq!(
        redeem(&server, &malformed, &password).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, wrong_body) = redeem(&server, &grant, "WRONGPWD").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unknown_body, wrong_body,
        "responses must not reveal which part was wrong"
    );

    // Lower-case password with spaces is accepted; the code is single use.
    let typed = format!(" {} ", password.to_ascii_lowercase());
    let (status, redeemed) = redeem(&server, &grant, &typed).await;
    assert_eq!(status, StatusCode::CREATED, "{redeemed}");
    assert_eq!(redeemed["session"]["accessMode"], "view-only");
    assert_eq!(redeemed["session"]["state"], "pending");
    assert_eq!(redeemed["hostName"], "Office PC");
    assert_eq!(
        redeem(&server, &grant, &password).await.0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "reused"
    );

    // Expired.
    let session_id = redeemed["session"]["id"].as_str().unwrap().to_owned();
    server
        .post(
            &format!("/api/v2/sessions/{session_id}/reject"),
            json!({}),
            Some(&fixture.host_access),
        )
        .await;
    let expired = create_grant(&server, &fixture).await;
    sqlx::query("UPDATE guest_access_grants SET expires_at = $1 WHERE id = $2")
        .bind(now() - 1)
        .bind(expired["id"].as_str().unwrap())
        .execute(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(
        redeem(&server, &expired, expired["password"].as_str().unwrap())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Revoked by the host, and replaced by a newer code.
    let revoked = create_grant(&server, &fixture).await;
    let (status, _) = server
        .post(
            &format!(
                "/api/v2/guest/grants/{}/revoke",
                revoked["id"].as_str().unwrap()
            ),
            json!({}),
            Some(&fixture.host_access),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        redeem(&server, &revoked, revoked["password"].as_str().unwrap())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let replaced = create_grant(&server, &fixture).await;
    let newest = create_grant(&server, &fixture).await;
    assert_eq!(
        redeem(&server, &replaced, replaced["password"].as_str().unwrap())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a newer code replaces the unused one"
    );
    let (status, listed) = server
        .get(
            &format!("/api/v2/guest/grants?hostDeviceId={}", fixture.host_device),
            &fixture.host_access,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], newest["id"]);
    assert!(listed[0].get("password").is_none() && listed[0].get("accessCode").is_none());
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn guest_password_guessing_burns_the_code_and_redeem_is_rate_limited() {
    let server = TestServer::new(true).await;
    let fixture = host_fixture(&server, "guest-brute@example.test").await;
    let grant = create_grant(&server, &fixture).await;
    for _ in 0..5 {
        assert_eq!(
            redeem(&server, &grant, "WRONGPWD").await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        redeem(&server, &grant, grant["password"].as_str().unwrap())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "five wrong passwords revoke the code"
    );
    // Per-address limit (20 in 10 minutes); 6 were used above.
    let mut statuses = Vec::new();
    for _ in 0..15 {
        statuses.push(redeem(&server, &grant, "WRONGPWD").await.0);
    }
    assert_eq!(statuses.last(), Some(&StatusCode::TOO_MANY_REQUESTS));
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn guest_token_is_scoped_to_one_session_and_needs_an_explicit_accept() {
    let server = TestServer::new(true).await;
    let fixture = host_fixture(&server, "guest-scope@example.test").await;
    let other = host_fixture(&server, "guest-other@example.test").await;

    // Offline host and non-host devices cannot issue codes; others cannot see them.
    let client_device = Uuid::new_v4().to_string();
    server
        .post(
            "/api/v2/devices/register",
            json!({"id": client_device, "name": "Laptop", "platform": "macos-aarch64", "deviceRole": "client", "nativeTransport": true}),
            Some(&fixture.host_access),
        )
        .await;
    assert_eq!(
        server
            .post(
                "/api/v2/guest/grants",
                json!({"hostDeviceId": client_device}),
                Some(&fixture.host_access)
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        server
            .post(
                "/api/v2/guest/grants",
                json!({"hostDeviceId": fixture.host_device}),
                Some(&other.host_access)
            )
            .await
            .0,
        StatusCode::NOT_FOUND,
        "wrong account"
    );
    assert_eq!(
        server
            .get(
                &format!("/api/v2/guest/grants?hostDeviceId={}", fixture.host_device),
                &other.host_access
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let grant = create_grant(&server, &fixture).await;
    assert_eq!(
        server
            .post(
                &format!(
                    "/api/v2/guest/grants/{}/revoke",
                    grant["id"].as_str().unwrap()
                ),
                json!({}),
                Some(&other.host_access)
            )
            .await
            .0,
        StatusCode::NOT_FOUND,
        "another account cannot revoke"
    );
    let (status, redeemed) = redeem(&server, &grant, grant["password"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::CREATED, "{redeemed}");
    let guest = token(&redeemed);
    let session_id = redeemed["session"]["id"].as_str().unwrap().to_owned();
    let guest_device = redeemed["deviceId"].as_str().unwrap().to_owned();

    // No account or device APIs.
    for path in [
        "/api/v2/account".to_owned(),
        "/api/v2/account/sessions".to_owned(),
        "/api/v2/account/two-factor".to_owned(),
        "/api/v2/devices".to_owned(),
        "/api/v2/sharing".to_owned(),
        format!(
            "/api/v2/sessions?hostDeviceId={}&state=active",
            fixture.host_device
        ),
        format!("/api/v2/guest/grants?hostDeviceId={}", fixture.host_device),
    ] {
        assert_eq!(
            server.get(&path, &guest).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    for (path, body) in [
        (
            "/api/v2/sessions",
            json!({"hostDeviceId": fixture.host_device, "requesterDeviceId": guest_device}),
        ),
        (
            "/api/v2/devices/register",
            json!({"name": "x", "platform": "macos-aarch64"}),
        ),
        (
            "/api/v2/guest/grants",
            json!({"hostDeviceId": fixture.host_device}),
        ),
        (
            "/api/v2/account/password",
            json!({"currentPassword": PASSWORD, "newPassword": "OtherPass1234"}),
        ),
    ] {
        assert_eq!(
            server.post(path, body, Some(&guest)).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        server
            .post(
                &format!("/api/v2/sessions/{session_id}/accept"),
                json!({"guestConfirmed": true, "accessMode": "control"}),
                Some(&guest)
            )
            .await
            .0,
        StatusCode::FORBIDDEN,
        "a guest cannot accept its own request"
    );
    // Only its own session and device.
    assert_eq!(
        server
            .get(&format!("/api/v2/sessions/{session_id}"), &guest)
            .await
            .0,
        StatusCode::OK
    );
    let other_session = Uuid::new_v4();
    assert_eq!(
        server
            .get(&format!("/api/v2/sessions/{other_session}"), &guest)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .post(
                "/api/v2/devices/heartbeat",
                json!({"deviceId": fixture.host_device}),
                Some(&guest)
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .post(
                "/api/v2/devices/heartbeat",
                json!({"deviceId": guest_device}),
                Some(&guest)
            )
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        server
            .get(
                &format!(
                    "/api/v2/sessions/{session_id}/credentials?deviceId={}",
                    fixture.host_device
                ),
                &guest
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // The guest device is invisible to the host account's device list and APIs.
    let (_, devices) = server.get("/api/v2/devices", &fixture.host_access).await;
    assert!(
        devices["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|device| device["id"] != guest_device.as_str())
    );
    assert_eq!(
        server
            .call(
                Method::DELETE,
                &format!("/api/v2/devices/{guest_device}"),
                None,
                Some(&fixture.host_access)
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // The host sees who is asking.
    let (_, queue) = server
        .get(
            &format!(
                "/api/v2/sessions?hostDeviceId={}&state=active",
                fixture.host_device
            ),
            &fixture.host_access,
        )
        .await;
    assert_eq!(queue["items"][0]["guest"]["name"], "Helper");

    // An accept without the guest confirmation (old host, auto-accept) is refused.
    let accept = format!("/api/v2/sessions/{session_id}/accept");
    assert_eq!(
        server
            .call(Method::POST, &accept, None, Some(&fixture.host_access))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        server
            .post(&accept, json!({}), Some(&fixture.host_access))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        server
            .post(
                &accept,
                json!({"guestConfirmed": true, "accessMode": "admin"}),
                Some(&fixture.host_access)
            )
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, accepted) = server
        .post(
            &accept,
            json!({"guestConfirmed": true}),
            Some(&fixture.host_access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(
        accepted["accessMode"], "view-only",
        "guests default to view-only"
    );

    // The guest ends the session; its token stops working at once.
    assert_eq!(
        server
            .post(
                &format!("/api/v2/sessions/{session_id}/disconnect"),
                json!({}),
                Some(&guest)
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        server
            .get(&format!("/api/v2/sessions/{session_id}"), &guest)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // Guest sign-ins never appear among the account's login sessions.
    let (_, logins) = server
        .get("/api/v2/account/sessions", &fixture.host_access)
        .await;
    assert_eq!(logins.as_array().unwrap().len(), 1);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn host_can_grant_control_and_revoke_a_running_guest_session() {
    let server = TestServer::new(true).await;
    let fixture = host_fixture(&server, "guest-control@example.test").await;
    let grant = create_grant(&server, &fixture).await;
    let (_, redeemed) = redeem(&server, &grant, grant["password"].as_str().unwrap()).await;
    let guest = token(&redeemed);
    let session_id = redeemed["session"]["id"].as_str().unwrap().to_owned();

    // A host busy with a guest cannot be redeemed again with another code.
    let second = create_grant(&server, &fixture).await;
    let (status, body) = redeem(&server, &second, second["password"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, accepted) = server
        .post(
            &format!("/api/v2/sessions/{session_id}/accept"),
            json!({"guestConfirmed": true, "accessMode": "control"}),
            Some(&fixture.host_access),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["accessMode"], "control");

    let (status, _) = server
        .post(
            &format!(
                "/api/v2/guest/grants/{}/revoke",
                grant["id"].as_str().unwrap()
            ),
            json!({}),
            Some(&fixture.host_access),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, ended) = server
        .get(
            &format!("/api/v2/sessions/{session_id}"),
            &fixture.host_access,
        )
        .await;
    assert_eq!(ended["state"], "disconnected");
    assert_eq!(ended["disconnectReason"], "guest_access_revoked");
    assert_eq!(
        server
            .get(&format!("/api/v2/sessions/{session_id}"), &guest)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );

    // Unanswered guest requests expire after two minutes, ending the token.
    let third = create_grant(&server, &fixture).await;
    let (status, pending) = redeem(&server, &third, third["password"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    let pending_session = pending["session"]["id"].as_str().unwrap().to_owned();
    sqlx::query("UPDATE connection_sessions SET created_at = $1 WHERE id = $2")
        .bind(now() - 3 * 60)
        .bind(&pending_session)
        .execute(&server.state.pool)
        .await
        .unwrap();
    sanser_server::routes::guest::cleanup(&server.state.pool, now()).await;
    assert_eq!(
        server
            .get(
                &format!("/api/v2/sessions/{pending_session}"),
                &token(&pending)
            )
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    server.close().await;
}
