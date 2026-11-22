//! Email sign-in codes and guest access: every rejection path.
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
use sanser_server::{
    build_router,
    config::{Config, PROTOCOL_VERSION, SANSER_VERSION},
    db,
    mailer::{Mailer, OutgoingEmail},
    state::{AppState, RateLimiter},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "StrongPass123";

type Outbox = Arc<tokio::sync::Mutex<Vec<OutgoingEmail>>>;

struct TestServer {
    app: Router,
    state: AppState,
    admin_pool: PgPool,
    schema: String,
    _permit: OwnedSemaphorePermit,
}

impl TestServer {
    /// With an in-memory mailer, so sign-in from a new computer and sign-up
    /// need the emailed code.
    async fn with_email() -> (Self, Outbox) {
        let (mailer, outbox) = Mailer::memory();
        let mut server = Self::new(true).await;
        server.state.mailer = mailer;
        server.app = build_router(server.state.clone());
        (server, outbox)
    }

    /// `generous_limits` lifts the per-address email limiter so a test can
    /// send many codes.
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
            state.email_limiter = RateLimiter::new(1_000, Duration::from_secs(600));
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

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

const INSTALLATION: &str = "6f1c2a7e-4b2d-4c1e-9a55-0d7e3f8b9c10";
const OTHER_INSTALLATION: &str = "a3e9c4d2-1f7b-4e6a-8c2d-5b9f0e1a7c34";

fn login_body(email: &str, installation: &str) -> Value {
    json!({"email": email, "password": PASSWORD, "deviceName": "Test", "platform": "test", "installationId": installation})
}

fn signup_body(email: &str, installation: &str) -> Value {
    json!({"email": email, "password": PASSWORD, "deviceName": "Test", "platform": "test", "installationId": installation})
}

/// The 6-digit code in the newest email to `to`.
async fn last_code(outbox: &Outbox, to: &str) -> String {
    let sent = outbox.lock().await;
    let email = sent
        .iter()
        .rev()
        .find(|email| email.to == to)
        .expect("an email was sent");
    email.subject.rsplit(' ').next().unwrap().to_owned()
}

fn challenge_of(body: &Value) -> String {
    assert_eq!(
        body["error"]["code"], "email_verification_required",
        "{body}"
    );
    body["error"]["challengeToken"].as_str().unwrap().to_owned()
}

async fn verify(server: &TestServer, challenge: &str, code: &str) -> (StatusCode, Value) {
    server
        .post(
            "/api/v2/auth/email/verify",
            json!({"challengeToken": challenge, "code": code}),
            None,
        )
        .await
}

/// Signs up through the email code and returns the tokens.
async fn signup_verified(server: &TestServer, outbox: &Outbox, email: &str) -> Value {
    let (status, body) = server
        .post(
            "/api/v2/auth/register",
            signup_body(email, INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let challenge = challenge_of(&body);
    let code = last_code(outbox, email).await;
    let (status, tokens) = verify(server, &challenge, &code).await;
    assert_eq!(status, StatusCode::CREATED, "{tokens}");
    tokens
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn signup_creates_the_account_only_after_the_emailed_code() {
    let (server, outbox) = TestServer::with_email().await;
    let (status, body) = server
        .post(
            "/api/v2/auth/register",
            signup_body("new@example.com", INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"]["emailHint"], "ne****@example.com");
    let challenge = challenge_of(&body);
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(users, 0, "no account exists before the code is entered");
    // Someone else cannot squat the address: a second sign-up just gets its
    // own challenge, and the first code does not complete it.
    let (_, second) = server
        .post(
            "/api/v2/auth/register",
            signup_body("new@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    let second_challenge = challenge_of(&second);
    let second_code = last_code(&outbox, "new@example.com").await;
    {
        let sent = outbox.lock().await;
        let first_code = sent[0].subject.rsplit(' ').next().unwrap().to_owned();
        drop(sent);
        if first_code != second_code {
            let (status, _) = verify(&server, &second_challenge, &first_code).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        }
    }
    let (status, _) = verify(&server, &challenge, "000000x").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let first_code = outbox.lock().await[0]
        .subject
        .rsplit(' ')
        .next()
        .unwrap()
        .to_owned();
    let (status, tokens) = verify(&server, &challenge, &first_code).await;
    assert_eq!(status, StatusCode::CREATED, "{tokens}");
    assert_eq!(tokens["account"]["email"], "new@example.com");
    let verified: Option<i64> = sqlx::query_scalar("SELECT email_verified_at FROM users")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert!(verified.is_some());
    // The code is single use, and the other sign-up can no longer finish.
    let (status, _) = verify(&server, &challenge, &first_code).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = verify(&server, &second_challenge, &second_code).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // An existing address is refused before any email is sent.
    let sent_before = outbox.lock().await.len();
    let (status, _) = server
        .post(
            "/api/v2/auth/register",
            signup_body("new@example.com", INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(outbox.lock().await.len(), sent_before);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn known_computer_signs_in_from_any_network_and_a_new_one_needs_the_code() {
    let (server, outbox) = TestServer::with_email().await;
    signup_verified(&server, &outbox, "owner@example.com").await;
    let sent = outbox.lock().await.len();
    // The computer that signed up is trusted: password alone.
    let (status, body) = server
        .post(
            "/api/v2/auth/login",
            login_body("owner@example.com", INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        outbox.lock().await.len(),
        sent,
        "no email for a known computer"
    );
    // A different computer, or a client without an installation ID, gets a code.
    let (status, body) = server
        .post(
            "/api/v2/auth/login",
            login_body("owner@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = challenge_of(&body);
    assert!(
        body["error"]["challengeToken"]
            .as_str()
            .unwrap()
            .starts_with("sn2_e_")
    );
    let mail = outbox.lock().await.last().cloned().unwrap();
    assert_eq!(mail.to, "owner@example.com");
    assert!(mail.body.contains("Test (test)"));
    let (status, body) = server.login("owner@example.com").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    challenge_of(&body);
    // A wrong password never sends an email.
    let before = outbox.lock().await.len();
    let (status, _) = server
        .post(
            "/api/v2/auth/login",
            json!({"email": "owner@example.com", "password": "WrongPass123", "installationId": OTHER_INSTALLATION}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(outbox.lock().await.len(), before);
    // The right code signs in and trusts the new computer.
    let code = {
        let sent = outbox.lock().await;
        sent.iter()
            .rev()
            .nth(1)
            .unwrap()
            .subject
            .rsplit(' ')
            .next()
            .unwrap()
            .to_owned()
    };
    let (status, tokens) = verify(&server, &challenge, &code).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let (status, _) = server
        .post(
            "/api/v2/auth/login",
            login_body("owner@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn email_codes_reject_guessing_expiry_cross_challenge_and_fast_resend() {
    let (server, outbox) = TestServer::with_email().await;
    signup_verified(&server, &outbox, "alice@example.com").await;
    signup_verified(&server, &outbox, "bob@example.com").await;
    let (_, alice) = server
        .post(
            "/api/v2/auth/login",
            login_body("alice@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    let alice_challenge = challenge_of(&alice);
    let alice_code = last_code(&outbox, "alice@example.com").await;
    let (_, bob) = server
        .post(
            "/api/v2/auth/login",
            login_body("bob@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    let bob_challenge = challenge_of(&bob);
    // Alice's code does not open Bob's challenge (unless the codes collide).
    let bob_code = last_code(&outbox, "bob@example.com").await;
    if alice_code != bob_code {
        let (status, _) = verify(&server, &bob_challenge, &alice_code).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    // Unknown and malformed challenges.
    for challenge in [
        "sn2_e_unknownunknownunknownunknownunknown",
        "not-a-token",
        "",
    ] {
        let (status, _) = verify(&server, challenge, &alice_code).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // Resend is refused within a minute, and a resent code replaces the old one.
    let (status, _) = server
        .post(
            "/api/v2/auth/email/resend",
            json!({"challengeToken": alice_challenge}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    sqlx::query("UPDATE email_challenges SET last_sent_at = last_sent_at - 120")
        .execute(&server.state.pool)
        .await
        .unwrap();
    let (status, body) = server
        .post(
            "/api/v2/auth/email/resend",
            json!({"challengeToken": alice_challenge}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["emailHint"], "al****@example.com");
    let resent = last_code(&outbox, "alice@example.com").await;
    let mut guesses_left = 5;
    if resent != alice_code {
        let (status, _) = verify(&server, &alice_challenge, &alice_code).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "the replaced code is dead"
        );
        guesses_left -= 1;
    }
    // Five wrong guesses in all burn the challenge, even with the right code after.
    let wrong = if resent == "111111" {
        "222222"
    } else {
        "111111"
    };
    for _ in 0..guesses_left {
        let (status, _) = verify(&server, &alice_challenge, wrong).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (status, _) = verify(&server, &alice_challenge, &resent).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // An expired code is refused.
    sqlx::query("UPDATE email_challenges SET expires_at = $1 WHERE consumed_at IS NULL")
        .bind(now() - 1)
        .execute(&server.state.pool)
        .await
        .unwrap();
    let (status, _) = verify(&server, &bob_challenge, &bob_code).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn signing_out_another_computer_forgets_it() {
    let (server, outbox) = TestServer::with_email().await;
    let first = signup_verified(&server, &outbox, "owner@example.com").await;
    let (_, body) = server
        .post(
            "/api/v2/auth/login",
            login_body("owner@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    let challenge = challenge_of(&body);
    let code = last_code(&outbox, "owner@example.com").await;
    let (status, second) = verify(&server, &challenge, &code).await;
    assert_eq!(status, StatusCode::OK);
    let second_session = second["sessionId"].as_str().unwrap();
    let (status, _) = server
        .call(
            Method::DELETE,
            &format!("/api/v2/account/sessions/{second_session}"),
            None,
            Some(&token(&first)),
        )
        .await;
    assert!(status.is_success(), "{status}");
    let (status, body) = server
        .post(
            "/api/v2/auth/login",
            login_body("owner@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    challenge_of(&body);
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn forgot_password_resets_with_the_emailed_code_and_signs_out_everywhere() {
    const NEW_PASSWORD: &str = "BrandNewPass456";
    let (server, outbox) = TestServer::with_email().await;
    let first = signup_verified(&server, &outbox, "owner@example.com").await;
    let forgot = |email: &'static str| {
        server.post(
            "/api/v2/auth/password/forgot",
            json!({"email": email, "deviceName": "Phone", "platform": "test", "installationId": OTHER_INSTALLATION}),
            None,
        )
    };
    // An unknown address gets the same answer, but no email and no usable code.
    let sent_before = outbox.lock().await.len();
    let (status, decoy) = forgot("nobody@example.com").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{decoy}");
    assert_eq!(decoy["emailHint"], "no****@example.com");
    assert_eq!(
        outbox.lock().await.len(),
        sent_before,
        "no email for an unknown address"
    );
    let decoy_challenge = decoy["challengeToken"].as_str().unwrap();
    for code in ["000000", "123456"] {
        let (status, _) = server
            .post(
                "/api/v2/auth/password/reset",
                json!({"challengeToken": decoy_challenge, "code": code, "newPassword": NEW_PASSWORD}),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (status, _) = server
        .post(
            "/api/v2/auth/password/check",
            json!({"challengeToken": decoy_challenge, "code": "123456"}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, body) = forgot("owner@example.com").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let challenge = body["challengeToken"].as_str().unwrap().to_owned();
    let email = outbox.lock().await.last().cloned().unwrap();
    assert!(
        email.subject.contains("đặt lại mật khẩu"),
        "{}",
        email.subject
    );
    let code = last_code(&outbox, "owner@example.com").await;
    // A reset code never works as a sign-in code.
    let (status, _) = verify(&server, &challenge, &code).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The new password must follow the password rules.
    let (status, _) = server
        .post(
            "/api/v2/auth/password/reset",
            json!({"challengeToken": challenge, "code": code, "newPassword": "short"}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let wrong = if code == "000000" { "111111" } else { "000000" };
    // The code is checked on its own before the new password is asked for.
    let check = |code: &str| {
        server.post(
            "/api/v2/auth/password/check",
            json!({"challengeToken": challenge, "code": code}),
            None,
        )
    };
    let (status, _) = check(wrong).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = check(&code).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = server
        .post(
            "/api/v2/auth/password/reset",
            json!({"challengeToken": challenge, "code": wrong, "newPassword": NEW_PASSWORD}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, tokens) = server
        .post(
            "/api/v2/auth/password/reset",
            json!({"challengeToken": challenge, "code": code, "newPassword": NEW_PASSWORD}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(tokens["account"]["email"], "owner@example.com");
    // Used once only.
    let (status, _) = server
        .post(
            "/api/v2/auth/password/reset",
            json!({"challengeToken": challenge, "code": code, "newPassword": NEW_PASSWORD}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The old session is signed out; the new one works.
    let (status, _) = server.get("/api/v2/account", &token(&first)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = server.get("/api/v2/account", &token(&tokens)).await;
    assert_eq!(status, StatusCode::OK);
    // The old password no longer works; the new one does, on the phone that
    // reset it, without another code.
    let (status, _) = server
        .post(
            "/api/v2/auth/login",
            login_body("owner@example.com", INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = server
        .post(
            "/api/v2/auth/login",
            json!({"email": "owner@example.com", "password": NEW_PASSWORD, "deviceName": "Phone", "platform": "test", "installationId": OTHER_INSTALLATION}),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    server.close().await;
}

#[tokio::test]
#[ignore = "requires a dedicated TEST_DATABASE_URL; run explicitly with --ignored"]
async fn without_smtp_sign_in_needs_only_the_password_and_trusts_the_computer() {
    let server = TestServer::new(false).await;
    server.register("plain@example.com").await;
    let (status, body) = server
        .post(
            "/api/v2/auth/login",
            login_body("plain@example.com", OTHER_INSTALLATION),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let trusted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM trusted_devices")
        .fetch_one(&server.state.pool)
        .await
        .unwrap();
    assert_eq!(trusted, 1);
    server.close().await;
}

// --- Guest access ----------------------------------------------------------

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
