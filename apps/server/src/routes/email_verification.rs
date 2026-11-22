//! Email codes for sign-in from a new computer and for sign-up.
//! See docs/email-verification.md.
//!
//! A computer (installation) that completed a sign-in to an account is
//! trusted for that account and signs in with the password alone, from any
//! network. Any other computer gets a challenge: a 6-digit code is emailed to
//! the account address and must be entered within 10 minutes.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use rand::{Rng, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{
        audit, create_account_session, create_login_session, new_token, token_digest,
        valid_token_shape,
    },
    error::{ApiJson, AppError},
    mailer::OutgoingEmail,
    models::{Account, AuthTokens},
    state::AppState,
    time::now_unix,
};

pub(crate) const CHALLENGE_PREFIX: &str = "sn2_e_";
pub const CHALLENGE_TTL_SECONDS: i64 = 10 * 60;
pub const CHALLENGE_MAX_ATTEMPTS: i32 = 5;
pub const RESEND_INTERVAL_SECONDS: i64 = 60;
pub const MAX_SENDS: i32 = 5;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyRequest {
    challenge_token: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResendRequest {
    challenge_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResendResponse {
    challenge_expires_at: i64,
    email_hint: String,
}

/// Where a sign-in or sign-up comes from.
pub struct Origin<'a> {
    pub device_name: &'a str,
    pub platform: &'a str,
    pub installation_id: Option<&'a str>,
}

/// A sign-up waiting for its email code.
pub struct PendingAccount {
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
}

/// Called after a correct password. Returns `Ok` when this computer may sign
/// in without an email code, otherwise the challenge error for the client.
pub async fn require_login_code(
    state: &AppState,
    account: &Account,
    origin: &Origin<'_>,
) -> Result<(), AppError> {
    if is_trusted(state, &account.id, origin.installation_id).await? {
        return Ok(());
    }
    if !state.mailer.enabled() {
        // Without a mail service the server cannot prove anything more than
        // the password. Remember the computer so it stays trusted once mail
        // is configured.
        trust_device(state, &account.id, origin.installation_id).await?;
        return Ok(());
    }
    Err(issue(state, Some(&account.id), &account.email, None, origin).await?)
}

/// Starts a sign-up. With a mail service it always returns the challenge
/// error; the account is created only when the emailed code is entered.
/// Returns `Ok(())` when sign-up should proceed directly.
pub async fn require_signup_code(
    state: &AppState,
    pending: &PendingAccount,
    origin: &Origin<'_>,
) -> Result<(), AppError> {
    if !state.mailer.enabled() {
        return Ok(());
    }
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)")
            .bind(&pending.email)
            .fetch_one(&state.pool)
            .await
            .map_err(AppError::from_db)?;
    if exists {
        return Err(AppError::Conflict(
            "an account with this email already exists".into(),
        ));
    }
    Err(issue(state, None, &pending.email, Some(pending), origin).await?)
}

/// Records that this computer completed a sign-in to the account.
pub async fn trust_device(
    state: &AppState,
    user_id: &str,
    installation_id: Option<&str>,
) -> Result<(), AppError> {
    let Some(installation_id) = installation_id else {
        return Ok(());
    };
    let now = now_unix();
    sqlx::query(
        "INSERT INTO trusted_devices (user_id, installation_id, created_at, last_used_at) \
         VALUES ($1, $2, $3, $3) \
         ON CONFLICT (user_id, installation_id) DO UPDATE SET last_used_at = EXCLUDED.last_used_at",
    )
    .bind(user_id)
    .bind(installation_id)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(())
}

/// Forgets a computer, so its next sign-in needs an email code again.
pub async fn forget_device(
    state: &AppState,
    user_id: &str,
    installation_id: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM trusted_devices WHERE user_id = $1 AND installation_id = $2")
        .bind(user_id)
        .bind(installation_id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    Ok(())
}

async fn is_trusted(
    state: &AppState,
    user_id: &str,
    installation_id: Option<&str>,
) -> Result<bool, AppError> {
    let Some(installation_id) = installation_id else {
        return Ok(false);
    };
    let updated = sqlx::query(
        "UPDATE trusted_devices SET last_used_at = $3 WHERE user_id = $1 AND installation_id = $2",
    )
    .bind(user_id)
    .bind(installation_id)
    .bind(now_unix())
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(updated.rows_affected() == 1)
}

async fn issue(
    state: &AppState,
    user_id: Option<&str>,
    email: &str,
    pending: Option<&PendingAccount>,
    origin: &Origin<'_>,
) -> Result<AppError, AppError> {
    // Bounds codes sent to one address, so sign-up cannot be used to flood
    // someone else's mailbox.
    if !state.email_limiter.check(format!("email:{email}")).await {
        return Err(AppError::RateLimited);
    }
    let token = new_token(CHALLENGE_PREFIX);
    let digest = token_digest(&token);
    let code = new_code();
    let now = now_unix();
    let expires_at = now + CHALLENGE_TTL_SECONDS;
    sqlx::query(
        "INSERT INTO email_challenges \
         (digest, purpose, user_id, email, display_name, password_hash, code_mac, device_name, \
          platform, installation_id, attempts, sends, created_at, last_sent_at, expires_at, consumed_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, 1, $11, $11, $12, NULL)",
    )
    .bind(&digest)
    .bind(if pending.is_some() { "register" } else { "login" })
    .bind(user_id)
    .bind(email)
    .bind(pending.map(|account| account.display_name.as_str()))
    .bind(pending.map(|account| account.password_hash.as_str()))
    .bind(code_mac(state, &digest, &code))
    .bind(origin.device_name)
    .bind(origin.platform)
    .bind(origin.installation_id)
    .bind(now)
    .bind(expires_at)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let purpose = if pending.is_some() {
        "register"
    } else {
        "login"
    };
    if let Err(error) = deliver(state, email, &code, purpose, origin).await {
        let _ = sqlx::query("DELETE FROM email_challenges WHERE digest = $1")
            .bind(&digest)
            .execute(&state.pool)
            .await;
        return Err(error);
    }
    audit(
        &state.pool,
        user_id,
        if pending.is_some() {
            "auth.register.email_code"
        } else {
            "auth.login.email_code"
        },
        Some("user"),
        user_id,
        serde_json::json!({"platform": origin.platform}),
    )
    .await;
    Ok(AppError::EmailVerificationRequired {
        challenge_token: token,
        expires_at,
        email_hint: email_hint(email),
    })
}

/// Completes a sign-in or sign-up with the emailed code.
pub async fn verify(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<VerifyRequest>,
) -> Result<Response, AppError> {
    let token = request.challenge_token.trim();
    if !valid_token_shape(token, CHALLENGE_PREFIX) {
        return Err(expired());
    }
    let Some(code) = normalize_code(&request.code) else {
        return Err(invalid_code());
    };
    let digest = token_digest(token);
    let now = now_unix();
    // Count the attempt before checking the code, atomically, so parallel
    // guesses cannot exceed the per-code limit.
    let row = sqlx::query(
        "UPDATE email_challenges SET attempts = attempts + 1 \
         WHERE digest = $1 AND consumed_at IS NULL AND expires_at > $2 AND attempts < $3 \
           AND purpose <> 'reset' \
         RETURNING purpose, user_id, email, display_name, password_hash, code_mac, \
                   device_name, platform, installation_id",
    )
    .bind(&digest)
    .bind(now)
    .bind(CHALLENGE_MAX_ATTEMPTS)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or_else(expired)?;
    let stored: String = row.try_get("code_mac").map_err(AppError::from_db)?;
    if !code_matches(&state, &digest, &code, &stored) {
        let user_id: Option<String> = row.try_get("user_id").map_err(AppError::from_db)?;
        audit(
            &state.pool,
            user_id.as_deref(),
            "auth.email_code.failure",
            Some("user"),
            user_id.as_deref(),
            serde_json::json!({}),
        )
        .await;
        return Err(invalid_code());
    }
    let consumed = sqlx::query(
        "UPDATE email_challenges SET consumed_at = $1 WHERE digest = $2 AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(&digest)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if consumed.rows_affected() != 1 {
        return Err(expired());
    }
    let purpose: String = row.try_get("purpose").map_err(AppError::from_db)?;
    let device_name: String = row.try_get("device_name").map_err(AppError::from_db)?;
    let platform: String = row.try_get("platform").map_err(AppError::from_db)?;
    let installation_id: Option<String> =
        row.try_get("installation_id").map_err(AppError::from_db)?;
    let (status, tokens) = if purpose == "register" {
        let email: String = row.try_get("email").map_err(AppError::from_db)?;
        let display_name: Option<String> =
            row.try_get("display_name").map_err(AppError::from_db)?;
        let password_hash: Option<String> =
            row.try_get("password_hash").map_err(AppError::from_db)?;
        let account = Account {
            id: Uuid::new_v4().to_string(),
            display_name: display_name.unwrap_or_else(|| email.clone()),
            email,
            created_at: now,
        };
        let tokens = create_account_session(
            &state,
            account,
            password_hash.ok_or(AppError::Internal)?,
            &device_name,
            &platform,
            installation_id.as_deref(),
        )
        .await?;
        audit(
            &state.pool,
            Some(&tokens.account.id),
            "auth.register",
            Some("user"),
            Some(&tokens.account.id),
            serde_json::json!({"platform": platform, "email": "verified"}),
        )
        .await;
        (StatusCode::CREATED, tokens)
    } else {
        let user_id: Option<String> = row.try_get("user_id").map_err(AppError::from_db)?;
        let account = load_account(&state, &user_id.ok_or(AppError::Internal)?).await?;
        let tokens = create_login_session(
            &state,
            account,
            &device_name,
            &platform,
            installation_id.as_deref(),
        )
        .await?;
        audit(
            &state.pool,
            Some(&tokens.account.id),
            "auth.login",
            Some("auth_session"),
            Some(&tokens.session_id),
            serde_json::json!({"platform": platform, "email": "verified"}),
        )
        .await;
        (StatusCode::OK, tokens)
    };
    sqlx::query(
        "UPDATE users SET email_verified_at = COALESCE(email_verified_at, $1) WHERE id = $2",
    )
    .bind(now)
    .bind(&tokens.account.id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    trust_device(&state, &tokens.account.id, installation_id.as_deref()).await?;
    Ok((status, Json::<AuthTokens>(tokens)).into_response())
}

/// Sends a fresh code for an open challenge, at most once a minute and five
/// times in all. The previous code stops working.
pub async fn resend(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ResendRequest>,
) -> Result<Json<ResendResponse>, AppError> {
    let token = request.challenge_token.trim();
    if !valid_token_shape(token, CHALLENGE_PREFIX) {
        return Err(expired());
    }
    let digest = token_digest(token);
    let now = now_unix();
    let code = new_code();
    let expires_at = now + CHALLENGE_TTL_SECONDS;
    let row = sqlx::query(
        "UPDATE email_challenges SET code_mac = $2, attempts = 0, sends = sends + 1, \
             last_sent_at = $3, expires_at = $4 \
         WHERE digest = $1 AND consumed_at IS NULL AND expires_at > $3 \
           AND sends < $5 AND last_sent_at <= $3 - $6 \
         RETURNING email, purpose, user_id, device_name, platform, installation_id",
    )
    .bind(&digest)
    .bind(code_mac(&state, &digest, &code))
    .bind(now)
    .bind(expires_at)
    .bind(MAX_SENDS)
    .bind(RESEND_INTERVAL_SECONDS)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let Some(row) = row else {
        return Err(AppError::Conflict(
            "Wait a minute before asking for another code. After five codes, sign in again.".into(),
        ));
    };
    let email: String = row.try_get("email").map_err(AppError::from_db)?;
    let purpose: String = row.try_get("purpose").map_err(AppError::from_db)?;
    let device_name: String = row.try_get("device_name").map_err(AppError::from_db)?;
    let platform: String = row.try_get("platform").map_err(AppError::from_db)?;
    let user_id: Option<String> = row.try_get("user_id").map_err(AppError::from_db)?;
    let origin = Origin {
        device_name: &device_name,
        platform: &platform,
        installation_id: None,
    };
    // A reset for an address without an account is a decoy: same answer,
    // no email.
    if purpose != "reset" || user_id.is_some() {
        deliver(&state, &email, &code, &purpose, &origin).await?;
    }
    Ok(Json(ResendResponse {
        challenge_expires_at: expires_at,
        email_hint: email_hint(&email),
    }))
}

/// Removes old challenges. Called from the periodic cleanup.
pub async fn cleanup(pool: &sqlx::PgPool, now: i64) {
    if let Err(error) = sqlx::query("DELETE FROM email_challenges WHERE expires_at < $1")
        .bind(now - 86_400)
        .execute(pool)
        .await
    {
        tracing::warn!(error = %error, "email verification cleanup failed");
    }
}

pub(crate) async fn deliver(
    state: &AppState,
    email: &str,
    code: &str,
    purpose: &str,
    origin: &Origin<'_>,
) -> Result<(), AppError> {
    let message = verification_email(email, code, purpose, origin);
    state.mailer.send(message).await.map_err(|error| {
        tracing::warn!(error = %error, "verification email was not sent");
        AppError::Unavailable
    })
}

/// `purpose` is the challenge purpose: "login", "register" or "reset".
pub fn verification_email(
    email: &str,
    code: &str,
    purpose: &str,
    origin: &Origin<'_>,
) -> OutgoingEmail {
    let (subject, purpose_vi, purpose_en) = match purpose {
        "register" => (
            format!("Mã xác nhận tạo tài khoản Sanser: {code}"),
            "tạo tài khoản Sanser",
            "create your Sanser account",
        ),
        "reset" => (
            format!("Mã đặt lại mật khẩu Sanser: {code}"),
            "đặt lại mật khẩu Sanser",
            "reset your Sanser password",
        ),
        _ => (
            format!("Mã đăng nhập Sanser: {code}"),
            "đăng nhập Sanser trên một máy mới",
            "sign in to Sanser on a new computer",
        ),
    };
    let body = format!(
        "Mã xác nhận để {purpose_vi} là:\n\n    {code}\n\n\
         Máy: {device} ({platform})\n\
         Mã hết hạn sau 10 phút. Đừng gửi mã này cho ai.\n\
         Nếu không phải bạn, hãy bỏ qua email này và đổi mật khẩu Sanser.\n\n\
         ---\n\n\
         Your code to {purpose_en} is {code}.\n\
         Computer: {device} ({platform})\n\
         The code expires in 10 minutes. Never share it.\n\
         If this was not you, ignore this email and change your Sanser password.\n",
        device = origin.device_name,
        platform = origin.platform,
    );
    OutgoingEmail {
        to: email.to_owned(),
        subject,
        body,
    }
}

pub(crate) async fn load_account(state: &AppState, user_id: &str) -> Result<Account, AppError> {
    let row = sqlx::query("SELECT id, email, display_name, created_at FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::Unauthorized)?;
    Ok(Account {
        id: row.try_get("id").map_err(AppError::from_db)?,
        email: row.try_get("email").map_err(AppError::from_db)?,
        display_name: row.try_get("display_name").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
    })
}

pub(crate) fn new_code() -> String {
    format!("{:06}", OsRng.gen_range(0..1_000_000_u32))
}

/// Accepts "123456", "123 456" and "123-456".
pub fn normalize_code(value: &str) -> Option<String> {
    let digits: String = value
        .chars()
        .filter(|character| !matches!(character, ' ' | '-'))
        .collect();
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}

/// The code is bound to its challenge, so a code mailed for one challenge
/// never completes another.
pub(crate) fn code_mac(state: &AppState, digest: &str, code: &str) -> String {
    hex::encode(mac(state, digest, code).finalize().into_bytes())
}

pub(crate) fn code_matches(state: &AppState, digest: &str, code: &str, stored: &str) -> bool {
    hex::decode(stored)
        .is_ok_and(|expected| mac(state, digest, code).verify_slice(&expected).is_ok())
}

fn mac(state: &AppState, digest: &str, code: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&state.config.session_credential_key)
        .unwrap_or_else(|_| unreachable!("HMAC accepts keys of any length"));
    mac.update(b"sanser-email-code\0");
    mac.update(digest.as_bytes());
    mac.update(b"\0");
    mac.update(code.as_bytes());
    mac
}

/// "an****@gmail.com": enough for the person to recognise their address.
pub fn email_hint(email: &str) -> String {
    let (local, domain) = email.split_once('@').unwrap_or((email, ""));
    let visible: String = local.chars().take(2).collect();
    format!("{visible}****@{domain}")
}

/// 401: the challenge is unknown, used, expired or out of attempts; the
/// client starts the sign-in again.
pub(crate) const fn expired() -> AppError {
    AppError::Unauthorized
}

pub(crate) fn invalid_code() -> AppError {
    AppError::Validation("The code is incorrect. Check the latest Sanser email.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits_and_normalized() {
        for _ in 0..200 {
            let code = new_code();
            assert_eq!(normalize_code(&code).as_deref(), Some(code.as_str()));
        }
        assert_eq!(normalize_code(" 123 456 ").as_deref(), Some("123456"));
        assert_eq!(normalize_code("123-456").as_deref(), Some("123456"));
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("1234567"), None);
        assert_eq!(normalize_code("12a456"), None);
        assert_eq!(normalize_code("١٢٣٤٥٦"), None);
    }

    #[test]
    fn email_hint_hides_most_of_the_address() {
        assert_eq!(email_hint("someone@example.com"), "so****@example.com");
        assert_eq!(email_hint("a@example.com"), "a****@example.com");
    }

    #[test]
    fn email_names_the_code_and_computer_in_both_languages() {
        let origin = Origin {
            device_name: "Office Mac",
            platform: "macos",
            installation_id: None,
        };
        let login = verification_email("someone@example.com", "042917", "login", &origin);
        assert_eq!(login.to, "someone@example.com");
        assert!(login.subject.contains("042917"));
        assert!(login.body.contains("042917"));
        assert!(login.body.contains("Office Mac (macos)"));
        assert!(login.body.contains("đăng nhập"));
        assert!(login.body.contains("sign in"));
        let signup = verification_email("someone@example.com", "042917", "register", &origin);
        assert!(signup.subject.contains("tạo tài khoản"));
        let reset = verification_email("someone@example.com", "042917", "reset", &origin);
        assert!(reset.subject.contains("đặt lại mật khẩu") && reset.subject.contains("042917"));
        assert!(reset.body.contains("reset your Sanser password"));
    }
}
