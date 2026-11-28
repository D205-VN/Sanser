//! Exam mode. See docs/exam-mode.md.
//!
//! A supervisor opens a room and reads its code to the class. Each student
//! joins from their own account and computer. While the room is open the
//! student's Sanser keeps only the allowed apps in front, closes AI apps and
//! sends a heartbeat (what is in front, any violations, now and then a
//! screenshot). The heartbeat answer carries the supervisor's decisions, so
//! pausing a student or ending the exam reaches them within seconds.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::{AuthContext, audit, clean_label},
    error::{ApiJson, AppError},
    routes::devices,
    state::AppState,
    time::now_unix,
};

const CODE_LENGTH: usize = 6;
/// Long enough that guessing it offline against the hash is hopeless.
const UNLOCK_CODE_LENGTH: usize = 8;
/// PBKDF2-HMAC-SHA256 rounds for the offline unlock check.
pub const UNLOCK_ITERATIONS: u32 = 100_000;
const CODE_ALPHABET: &[u8; 31] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const MAX_ALLOWED_APPS: usize = 12;
const MAX_HOURS: i64 = 8;
const MAX_SCREEN_BYTES: usize = 180_000;
/// A student is shown as disconnected after this many seconds of silence.
pub const ONLINE_WITHIN_SECONDS: i64 = 20;
/// A silence longer than this is written down as a violation when they return.
const GAP_VIOLATION_SECONDS: i64 = 30;
const NORMAL_HEARTBEAT_SECONDS: u32 = 5;
const NORMAL_SCREEN_SECONDS: u32 = 10;
const WATCHED_SECONDS: u32 = 2;
const ENDED_RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
const CLIENT_VIOLATIONS: [&str; 5] = [
    "left-app",
    "ai-app",
    "ai-site",
    "guard-failed",
    "quit-attempt",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AllowedApp {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRoom {
    title: String,
    #[serde(default)]
    allowed_apps: Vec<AllowedApp>,
    #[serde(default = "yes")]
    block_ai: bool,
    #[serde(default = "default_hours")]
    hours: i64,
}
fn yes() -> bool {
    true
}
fn default_hours() -> i64 {
    3
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRoom {
    allowed_apps: Option<Vec<AllowedApp>>,
    block_ai: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinRoom {
    code: String,
    device_id: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberAction {
    action: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientViolation {
    kind: String,
    detail: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Heartbeat {
    #[serde(default)]
    foreground_app: Option<String>,
    #[serde(default)]
    violations: Vec<ClientViolation>,
    #[serde(default)]
    screenshot: Option<String>,
}

#[derive(Deserialize)]
pub struct ScreensQuery {
    #[serde(default)]
    since: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSummary {
    id: String,
    title: String,
    code: String,
    allowed_apps: Vec<AllowedApp>,
    block_ai: bool,
    state: String,
    created_at: i64,
    expires_at: i64,
    ended_at: Option<i64>,
    /// Read to one student whose computer is stuck; absent on older rooms.
    #[serde(skip_serializing_if = "Option::is_none")]
    unlock_code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockCheck {
    /// Base64 salt and PBKDF2-HMAC-SHA256 hash of the upper-case code.
    salt: String,
    hash: String,
    iterations: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnlockRequest {
    code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    id: String,
    display_name: String,
    platform: String,
    state: String,
    online: bool,
    joined_at: i64,
    last_seen_at: i64,
    foreground_app: Option<String>,
    violation_count: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViolationView {
    member_id: String,
    display_name: String,
    kind: String,
    detail: String,
    created_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDetail {
    #[serde(flatten)]
    room: RoomSummary,
    members: Vec<MemberView>,
    violations: Vec<ViolationView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenView {
    member_id: String,
    jpeg_base64: String,
    captured_at: i64,
}

/// What the student computer must enforce right now.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    member_id: String,
    room_id: String,
    title: String,
    supervisor_name: String,
    /// `open` or `ended`.
    room_state: String,
    /// `active`, `paused` or `removed`.
    member_state: String,
    allowed_apps: Vec<AllowedApp>,
    block_ai: bool,
    heartbeat_seconds: u32,
    screen_seconds: u32,
    /// Lets the computer accept the emergency code without the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    unlock: Option<UnlockCheck>,
}

fn uuid(value: &str, field: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation(format!("{field} must be a UUID")))
}

/// Bundle identifiers and Windows file names share the blocked-app rules.
pub fn valid_app_id(id: &str) -> bool {
    let value = id.trim();
    !value.is_empty()
        && value.len() <= 255
        && !value.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

pub fn clean_allowed_apps(apps: Vec<AllowedApp>) -> Result<Vec<AllowedApp>, AppError> {
    if apps.len() > MAX_ALLOWED_APPS {
        return Err(AppError::Validation(format!(
            "at most {MAX_ALLOWED_APPS} apps can be allowed"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut clean = Vec::with_capacity(apps.len());
    for app in apps {
        if !valid_app_id(&app.id) {
            return Err(AppError::Validation(
                "an allowed app needs a bundle identifier or a program file name".into(),
            ));
        }
        let id = app.id.trim().to_owned();
        if !seen.insert(id.to_ascii_lowercase()) {
            continue;
        }
        let name = clean_label(&app.name, "app name", 80).unwrap_or_else(|_| id.clone());
        clean.push(AllowedApp { id, name });
    }
    Ok(clean)
}

pub fn normalize_code(value: &str) -> Option<String> {
    let code: String = value
        .chars()
        .filter(|c| !matches!(c, ' ' | '-'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == CODE_LENGTH && code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)))
        .then_some(code)
}

fn random_code(length: usize) -> String {
    (0..length)
        .map(|_| char::from(CODE_ALPHABET[OsRng.gen_range(0..CODE_ALPHABET.len())]))
        .collect()
}

fn generate_code() -> String {
    random_code(CODE_LENGTH)
}

/// Upper case without spaces or dashes, as people type it.
pub fn normalize_unlock_code(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// PBKDF2-HMAC-SHA256 with one 32-byte block (RFC 8018), as Web Crypto does.
pub fn unlock_hash(code: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let keyed = <Hmac<Sha256> as Mac>::new_from_slice(code.as_bytes())
        .expect("HMAC accepts keys of any length");
    let mut first = keyed.clone();
    first.update(salt);
    first.update(&1_u32.to_be_bytes());
    let mut block: [u8; 32] = first.finalize().into_bytes().into();
    let mut out = block;
    for _ in 1..iterations {
        let mut round = keyed.clone();
        round.update(&block);
        block = round.finalize().into_bytes().into();
        for (target, byte) in out.iter_mut().zip(block) {
            *target ^= byte;
        }
    }
    out
}

/// A new emergency code with its salt and hash, all ready to store.
fn new_unlock() -> (String, String, String) {
    let code = random_code(UNLOCK_CODE_LENGTH);
    let mut salt = [0_u8; 16];
    OsRng.fill_bytes(&mut salt);
    let hash = unlock_hash(&code, &salt, UNLOCK_ITERATIONS);
    (code, STANDARD.encode(salt), STANDARD.encode(hash))
}

fn codes_match(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Short free text from the student computer, made safe to store and show.
pub fn clean_detail(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// A JPEG of sensible size, or a validation error.
pub fn checked_screenshot(value: &str) -> Result<String, AppError> {
    let bytes = STANDARD
        .decode(value.trim())
        .map_err(|_| AppError::Validation("screenshot must be base64".into()))?;
    if bytes.len() > MAX_SCREEN_BYTES {
        return Err(AppError::PayloadTooLarge);
    }
    if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(AppError::Validation("screenshot must be a JPEG".into()));
    }
    Ok(value.trim().to_owned())
}

fn apps_from_json(value: &str) -> Vec<AllowedApp> {
    serde_json::from_str(value).unwrap_or_default()
}

fn row_to_room(row: &sqlx::postgres::PgRow) -> Result<RoomSummary, AppError> {
    Ok(RoomSummary {
        id: row.try_get("id").map_err(AppError::from_db)?,
        title: row.try_get("title").map_err(AppError::from_db)?,
        code: row.try_get("code").map_err(AppError::from_db)?,
        allowed_apps: apps_from_json(
            &row.try_get::<String, _>("allowed_apps_json")
                .map_err(AppError::from_db)?,
        ),
        block_ai: row.try_get("block_ai").map_err(AppError::from_db)?,
        state: row.try_get("state").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
        expires_at: row.try_get("expires_at").map_err(AppError::from_db)?,
        ended_at: row.try_get("ended_at").map_err(AppError::from_db)?,
        unlock_code: row.try_get("unlock_code").map_err(AppError::from_db)?,
    })
}

async fn owned_room(state: &AppState, user_id: &str, id: &str) -> Result<RoomSummary, AppError> {
    let row = sqlx::query("SELECT * FROM exam_rooms WHERE id=$1 AND owner_user_id=$2")
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?;
    row_to_room(&row)
}

async fn detail(state: &AppState, room: RoomSummary) -> Result<RoomDetail, AppError> {
    let now = now_unix();
    let members = sqlx::query(
        "SELECT * FROM exam_members WHERE room_id=$1 AND state <> 'removed' ORDER BY display_name, joined_at",
    )
    .bind(&room.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .into_iter()
    .map(|row| {
        let last_seen_at: i64 = row.try_get("last_seen_at").map_err(AppError::from_db)?;
        Ok(MemberView {
            id: row.try_get("id").map_err(AppError::from_db)?,
            display_name: row.try_get("display_name").map_err(AppError::from_db)?,
            platform: row.try_get("platform").map_err(AppError::from_db)?,
            state: row.try_get("state").map_err(AppError::from_db)?,
            online: now - last_seen_at <= ONLINE_WITHIN_SECONDS,
            joined_at: row.try_get("joined_at").map_err(AppError::from_db)?,
            last_seen_at,
            foreground_app: row.try_get("foreground_app").map_err(AppError::from_db)?,
            violation_count: row.try_get("violation_count").map_err(AppError::from_db)?,
        })
    })
    .collect::<Result<Vec<_>, AppError>>()?;
    let violations = sqlx::query(
        "SELECT v.member_id, m.display_name, v.kind, v.detail, v.created_at \
         FROM exam_violations v JOIN exam_members m ON m.id = v.member_id \
         WHERE v.room_id=$1 ORDER BY v.created_at DESC LIMIT 100",
    )
    .bind(&room.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .into_iter()
    .map(|row| {
        Ok(ViolationView {
            member_id: row.try_get("member_id").map_err(AppError::from_db)?,
            display_name: row.try_get("display_name").map_err(AppError::from_db)?,
            kind: row.try_get("kind").map_err(AppError::from_db)?,
            detail: row.try_get("detail").map_err(AppError::from_db)?,
            created_at: row.try_get("created_at").map_err(AppError::from_db)?,
        })
    })
    .collect::<Result<Vec<_>, AppError>>()?;
    Ok(RoomDetail {
        room,
        members,
        violations,
    })
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<CreateRoom>,
) -> Result<Json<RoomDetail>, AppError> {
    let title = clean_label(&request.title, "title", 80)?;
    let apps = clean_allowed_apps(request.allowed_apps)?;
    if !(1..=MAX_HOURS).contains(&request.hours) {
        return Err(AppError::Validation(format!(
            "hours must be between 1 and {MAX_HOURS}"
        )));
    }
    let open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM exam_rooms WHERE owner_user_id=$1 AND state='open'",
    )
    .bind(&auth.user_id)
    .fetch_one(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if open >= 5 {
        return Err(AppError::Conflict(
            "end an open exam before starting another".into(),
        ));
    }
    let now = now_unix();
    let id = Uuid::new_v4().to_string();
    let apps_json = serde_json::to_string(&apps).map_err(|_| AppError::Internal)?;
    // A hundred thousand hash rounds: keep them off the async workers.
    let (unlock_code, unlock_salt, unlock_hash) = tokio::task::spawn_blocking(new_unlock)
        .await
        .map_err(|_| AppError::Internal)?;
    // A code collision with another open room is rare; try a few codes.
    for _ in 0..8 {
        let inserted = sqlx::query(
            "INSERT INTO exam_rooms (id, owner_user_id, title, code, allowed_apps_json, block_ai, created_at, updated_at, expires_at, \
             unlock_code, unlock_salt, unlock_hash) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING",
        )
        .bind(&id)
        .bind(&auth.user_id)
        .bind(&title)
        .bind(generate_code())
        .bind(&apps_json)
        .bind(request.block_ai)
        .bind(now)
        .bind(now + request.hours * 3600)
        .bind(&unlock_code)
        .bind(&unlock_salt)
        .bind(&unlock_hash)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
        if inserted.rows_affected() == 1 {
            audit(
                &state.pool,
                Some(&auth.user_id),
                "exam.create",
                Some("exam_room"),
                Some(&id),
                serde_json::json!({}),
            )
            .await;
            let room = owned_room(&state, &auth.user_id, &id).await?;
            return Ok(Json(detail(&state, room).await?));
        }
    }
    Err(AppError::Unavailable)
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Vec<RoomSummary>>, AppError> {
    let rows = sqlx::query(
        "SELECT * FROM exam_rooms WHERE owner_user_id=$1 ORDER BY created_at DESC LIMIT 20",
    )
    .bind(&auth.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    Ok(Json(
        rows.iter().map(row_to_room).collect::<Result<_, _>>()?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<RoomDetail>, AppError> {
    let room = owned_room(&state, &auth.user_id, &uuid(&id, "id")?).await?;
    Ok(Json(detail(&state, room).await?))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateRoom>,
) -> Result<Json<RoomDetail>, AppError> {
    let room = owned_room(&state, &auth.user_id, &uuid(&id, "id")?).await?;
    if room.state != "open" {
        return Err(AppError::Conflict("this exam has ended".into()));
    }
    let apps = match request.allowed_apps {
        Some(apps) => clean_allowed_apps(apps)?,
        None => room.allowed_apps.clone(),
    };
    let apps_json = serde_json::to_string(&apps).map_err(|_| AppError::Internal)?;
    sqlx::query(
        "UPDATE exam_rooms SET allowed_apps_json=$1, block_ai=$2, updated_at=$3 WHERE id=$4",
    )
    .bind(apps_json)
    .bind(request.block_ai.unwrap_or(room.block_ai))
    .bind(now_unix())
    .bind(&room.id)
    .execute(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    let room = owned_room(&state, &auth.user_id, &room.id).await?;
    Ok(Json(detail(&state, room).await?))
}

pub async fn end(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<RoomDetail>, AppError> {
    let room = owned_room(&state, &auth.user_id, &uuid(&id, "id")?).await?;
    let now = now_unix();
    sqlx::query("UPDATE exam_rooms SET state='ended', ended_at=$1, updated_at=$1 WHERE id=$2 AND state='open'")
        .bind(now)
        .bind(&room.id)
        .execute(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    state.exam_live.forget_room(&room.id).await;
    audit(
        &state.pool,
        Some(&auth.user_id),
        "exam.end",
        Some("exam_room"),
        Some(&room.id),
        serde_json::json!({}),
    )
    .await;
    let room = owned_room(&state, &auth.user_id, &room.id).await?;
    Ok(Json(detail(&state, room).await?))
}

pub async fn member_action(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((id, member)): Path<(String, String)>,
    ApiJson(request): ApiJson<MemberAction>,
) -> Result<Json<RoomDetail>, AppError> {
    let room = owned_room(&state, &auth.user_id, &uuid(&id, "id")?).await?;
    let member = uuid(&member, "memberId")?;
    let next = match request.action.as_str() {
        "pause" => Some("paused"),
        "resume" => Some("active"),
        "remove" => Some("removed"),
        "watch" => None,
        _ => {
            return Err(AppError::Validation(
                "action must be pause, resume, remove or watch".into(),
            ));
        }
    };
    let exists: Option<String> =
        sqlx::query_scalar("SELECT state FROM exam_members WHERE id=$1 AND room_id=$2")
            .bind(&member)
            .bind(&room.id)
            .fetch_optional(&state.pool)
            .await
            .map_err(AppError::from_db)?;
    match exists.as_deref() {
        None | Some("removed") => return Err(AppError::NotFound),
        Some(_) => {}
    }
    match next {
        Some(next) => {
            sqlx::query("UPDATE exam_members SET state=$1 WHERE id=$2")
                .bind(next)
                .bind(&member)
                .execute(&state.pool)
                .await
                .map_err(AppError::from_db)?;
            if next == "removed" {
                state.exam_live.forget_member(&member).await;
            }
        }
        None => state.exam_live.watch(&member).await,
    }
    Ok(Json(detail(&state, room).await?))
}

pub async fn screens(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    Query(query): Query<ScreensQuery>,
) -> Result<Json<Vec<ScreenView>>, AppError> {
    let room = owned_room(&state, &auth.user_id, &uuid(&id, "id")?).await?;
    Ok(Json(
        state
            .exam_live
            .room_screens(&room.id, query.since)
            .await
            .into_iter()
            .map(|(member_id, screen)| ScreenView {
                member_id,
                jpeg_base64: screen.jpeg_base64,
                captured_at: screen.captured_at,
            })
            .collect(),
    ))
}

pub async fn join(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<JoinRoom>,
) -> Result<Json<Policy>, AppError> {
    if !state.exam_join_limiter.check(auth.user_id.clone()).await {
        return Err(AppError::RateLimited);
    }
    let device_id = uuid(&request.device_id, "deviceId")?;
    let device = devices::fetch_owned(&state, &auth.user_id, &device_id).await?;
    let display_name = clean_label(&request.display_name, "name", 60)?;
    let code = normalize_code(&request.code)
        .ok_or_else(|| AppError::Validation("enter the 6-character exam code".into()))?;
    let now = now_unix();
    let room_id: String = sqlx::query_scalar(
        "SELECT id FROM exam_rooms WHERE code=$1 AND state='open' AND expires_at > $2",
    )
    .bind(&code)
    .bind(now)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or_else(|| AppError::Validation("no open exam uses this code".into()))?;
    let elsewhere: Option<String> = sqlx::query_scalar(
        "SELECT m.id FROM exam_members m JOIN exam_rooms r ON r.id = m.room_id \
         WHERE m.device_id=$1 AND m.room_id <> $2 AND m.state <> 'removed' AND r.state='open' LIMIT 1",
    )
    .bind(&device_id)
    .bind(&room_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?;
    if elsewhere.is_some() {
        return Err(AppError::Conflict(
            "this computer is already taking another exam".into(),
        ));
    }
    let existing: Option<(String, String)> =
        sqlx::query_as("SELECT id, state FROM exam_members WHERE room_id=$1 AND device_id=$2")
            .bind(&room_id)
            .bind(&device_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(AppError::from_db)?;
    let member_id = match existing {
        Some((_, member_state)) if member_state == "removed" => {
            return Err(AppError::Forbidden);
        }
        Some((id, _)) => {
            sqlx::query("UPDATE exam_members SET display_name=$1, last_seen_at=$2 WHERE id=$3")
                .bind(&display_name)
                .bind(now)
                .bind(&id)
                .execute(&state.pool)
                .await
                .map_err(AppError::from_db)?;
            id
        }
        None => {
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO exam_members (id, room_id, user_id, device_id, display_name, platform, joined_at, last_seen_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$7)",
            )
            .bind(&id)
            .bind(&room_id)
            .bind(&auth.user_id)
            .bind(&device_id)
            .bind(&display_name)
            .bind(clean_detail(&device.platform, 40))
            .bind(now)
            .execute(&state.pool)
            .await
            .map_err(AppError::from_db)?;
            id
        }
    };
    audit(
        &state.pool,
        Some(&auth.user_id),
        "exam.join",
        Some("exam_member"),
        Some(&member_id),
        serde_json::json!({ "roomId": room_id }),
    )
    .await;
    Ok(Json(policy(&state, &auth.user_id, &member_id).await?))
}

async fn policy(state: &AppState, user_id: &str, member_id: &str) -> Result<Policy, AppError> {
    let row = sqlx::query(
        "SELECT m.id AS member_id, m.state AS member_state, r.id AS room_id, r.title, r.state AS room_state, \
         r.expires_at, r.allowed_apps_json, r.block_ai, r.unlock_salt, r.unlock_hash, u.display_name AS supervisor_name \
         FROM exam_members m JOIN exam_rooms r ON r.id = m.room_id JOIN users u ON u.id = r.owner_user_id \
         WHERE m.id=$1 AND m.user_id=$2",
    )
    .bind(member_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    let expires_at: i64 = row.try_get("expires_at").map_err(AppError::from_db)?;
    let mut room_state: String = row.try_get("room_state").map_err(AppError::from_db)?;
    if room_state == "open" && expires_at <= now_unix() {
        room_state = "ended".into();
    }
    let watched = state.exam_live.is_watched(member_id).await;
    let salt: Option<String> = row.try_get("unlock_salt").map_err(AppError::from_db)?;
    let hash: Option<String> = row.try_get("unlock_hash").map_err(AppError::from_db)?;
    let unlock = salt.zip(hash).map(|(salt, hash)| UnlockCheck {
        salt,
        hash,
        iterations: UNLOCK_ITERATIONS,
    });
    Ok(Policy {
        member_id: row.try_get("member_id").map_err(AppError::from_db)?,
        room_id: row.try_get("room_id").map_err(AppError::from_db)?,
        title: row.try_get("title").map_err(AppError::from_db)?,
        supervisor_name: row.try_get("supervisor_name").map_err(AppError::from_db)?,
        room_state,
        member_state: row.try_get("member_state").map_err(AppError::from_db)?,
        allowed_apps: apps_from_json(
            &row.try_get::<String, _>("allowed_apps_json")
                .map_err(AppError::from_db)?,
        ),
        block_ai: row.try_get("block_ai").map_err(AppError::from_db)?,
        heartbeat_seconds: if watched {
            WATCHED_SECONDS
        } else {
            NORMAL_HEARTBEAT_SECONDS
        },
        screen_seconds: if watched {
            WATCHED_SECONDS
        } else {
            NORMAL_SCREEN_SECONDS
        },
        unlock,
    })
}

/// Student side: the supervisor read out the emergency code. The student
/// leaves the room and the supervisor sees an "emergency-unlock" entry.
pub async fn unlock(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(member): Path<String>,
    ApiJson(request): ApiJson<UnlockRequest>,
) -> Result<Json<Policy>, AppError> {
    let member_id = uuid(&member, "memberId")?;
    if !state
        .exam_join_limiter
        .check(format!("unlock:{member_id}"))
        .await
    {
        return Err(AppError::RateLimited);
    }
    let current = policy(&state, &auth.user_id, &member_id).await?;
    if current.room_state != "open" || current.member_state == "removed" {
        return Ok(Json(current));
    }
    let expected: Option<String> =
        sqlx::query_scalar("SELECT unlock_code FROM exam_rooms WHERE id=$1")
            .bind(&current.room_id)
            .fetch_one(&state.pool)
            .await
            .map_err(AppError::from_db)?;
    let typed = normalize_unlock_code(&request.code);
    if !expected.is_some_and(|code| codes_match(&code, &typed)) {
        return Err(AppError::Validation("the unlock code is not right".into()));
    }
    let now = now_unix();
    let mut transaction = state.pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE exam_members SET state='removed', last_seen_at=$1, violation_count = violation_count + 1 WHERE id=$2",
    )
    .bind(now)
    .bind(&member_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query(
        "INSERT INTO exam_violations (id, room_id, member_id, kind, detail, created_at) VALUES ($1,$2,$3,'emergency-unlock','',$4)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&current.room_id)
    .bind(&member_id)
    .bind(now)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    transaction.commit().await.map_err(AppError::from_db)?;
    state.exam_live.forget_member(&member_id).await;
    Ok(Json(policy(&state, &auth.user_id, &member_id).await?))
}

pub async fn heartbeat(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(member): Path<String>,
    ApiJson(request): ApiJson<Heartbeat>,
) -> Result<Json<Policy>, AppError> {
    let member_id = uuid(&member, "memberId")?;
    if !state.exam_heartbeat_limiter.check(member_id.clone()).await {
        return Err(AppError::RateLimited);
    }
    if request.violations.len() > 20 {
        return Err(AppError::Validation(
            "too many violations in one heartbeat".into(),
        ));
    }
    let current = policy(&state, &auth.user_id, &member_id).await?;
    if current.room_state != "open" || current.member_state == "removed" {
        return Ok(Json(current));
    }
    let now = now_unix();
    let last_seen_at: i64 = sqlx::query_scalar("SELECT last_seen_at FROM exam_members WHERE id=$1")
        .bind(&member_id)
        .fetch_one(&state.pool)
        .await
        .map_err(AppError::from_db)?;
    let mut violations: Vec<(String, String)> = Vec::new();
    let silence = now - last_seen_at;
    if silence > GAP_VIOLATION_SECONDS {
        violations.push(("disconnected".into(), format!("{silence}")));
    }
    for violation in request.violations {
        if !CLIENT_VIOLATIONS.contains(&violation.kind.as_str()) {
            return Err(AppError::Validation("unknown violation kind".into()));
        }
        violations.push((violation.kind, clean_detail(&violation.detail, 200)));
    }
    let foreground = request
        .foreground_app
        .as_deref()
        .map(|value| clean_detail(value, 120))
        .filter(|value| !value.is_empty());
    let mut transaction = state.pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query(
        "UPDATE exam_members SET last_seen_at=$1, foreground_app=COALESCE($2, foreground_app), \
         violation_count = violation_count + $3 WHERE id=$4",
    )
    .bind(now)
    .bind(&foreground)
    .bind(i32::try_from(violations.len()).unwrap_or(i32::MAX))
    .bind(&member_id)
    .execute(&mut *transaction)
    .await
    .map_err(AppError::from_db)?;
    for (kind, detail) in &violations {
        sqlx::query(
            "INSERT INTO exam_violations (id, room_id, member_id, kind, detail, created_at) VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&current.room_id)
        .bind(&member_id)
        .bind(kind)
        .bind(detail)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(AppError::from_db)?;
    }
    transaction.commit().await.map_err(AppError::from_db)?;
    if let Some(screenshot) = request.screenshot.as_deref() {
        let screenshot = checked_screenshot(screenshot)?;
        state
            .exam_live
            .put_screen(&member_id, &current.room_id, screenshot, now)
            .await;
    }
    Ok(Json(current))
}

/// Student side: read the current rules without reporting anything, used
/// after Sanser restarts in the middle of an exam.
pub async fn member_policy(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(member): Path<String>,
) -> Result<Json<Policy>, AppError> {
    Ok(Json(
        policy(&state, &auth.user_id, &uuid(&member, "memberId")?).await?,
    ))
}

pub async fn cleanup(pool: &sqlx::PgPool, now: i64) {
    if let Err(error) = sqlx::query(
        "UPDATE exam_rooms SET state='ended', ended_at=$1, updated_at=$1 WHERE state='open' AND expires_at <= $1",
    )
    .bind(now)
    .execute(pool)
    .await
    {
        tracing::warn!(error = %error, "failed to end expired exams");
    }
    if let Err(error) = sqlx::query("DELETE FROM exam_rooms WHERE state='ended' AND ended_at < $1")
        .bind(now - ENDED_RETENTION_SECONDS)
        .execute(pool)
        .await
    {
        tracing::warn!(error = %error, "failed to delete old exams");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn unlock_hash_is_standard_pbkdf2_sha256() {
        assert_eq!(
            hex(&unlock_hash("password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            hex(&unlock_hash("password", b"salt", 2)),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
    }

    #[test]
    fn unlock_codes_are_typed_loosely_and_compared_exactly() {
        assert_eq!(normalize_unlock_code(" ab3k-9mz q "), "AB3K9MZQ");
        assert!(codes_match("AB3K9MZQ", "AB3K9MZQ"));
        assert!(!codes_match("AB3K9MZQ", "AB3K9MZ"));
        assert!(!codes_match("AB3K9MZQ", "AB3K9MZR"));
        let (code, salt, hash) = new_unlock();
        assert_eq!(code.len(), UNLOCK_CODE_LENGTH);
        let salt = STANDARD.decode(salt).expect("salt");
        assert_eq!(
            STANDARD.encode(unlock_hash(&code, &salt, UNLOCK_ITERATIONS)),
            hash
        );
    }

    #[test]
    fn codes_are_six_unambiguous_characters_and_normalize() {
        for _ in 0..100 {
            let code = generate_code();
            assert_eq!(normalize_code(&code).as_deref(), Some(code.as_str()));
        }
        assert_eq!(normalize_code("ab2-c3d").as_deref(), Some("AB2C3D"));
        assert_eq!(normalize_code("AB2C3"), None);
        // 0, 1, I, L and O are left out so a code read aloud is not misheard.
        assert_eq!(normalize_code("AB2C3O"), None);
        assert_eq!(normalize_code("AB2C31"), None);
    }

    #[test]
    fn allowed_apps_are_validated_and_deduplicated() {
        let apps = clean_allowed_apps(vec![
            AllowedApp {
                id: "com.microsoft.Word".into(),
                name: "Word".into(),
            },
            AllowedApp {
                id: "COM.MICROSOFT.WORD".into(),
                name: "Word again".into(),
            },
            AllowedApp {
                id: "WINWORD.EXE".into(),
                name: " ".into(),
            },
        ])
        .expect("valid apps");
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[1].name, "WINWORD.EXE");
        assert!(
            clean_allowed_apps(vec![AllowedApp {
                id: "../evil".into(),
                name: "x".into()
            }])
            .is_err()
        );
        let many = (0..13)
            .map(|i| AllowedApp {
                id: format!("app{i}.exe"),
                name: "x".into(),
            })
            .collect();
        assert!(clean_allowed_apps(many).is_err());
    }

    #[test]
    fn screenshots_must_be_small_jpegs() {
        let jpeg = STANDARD.encode([0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3]);
        assert!(checked_screenshot(&jpeg).is_ok());
        assert!(checked_screenshot(&STANDARD.encode(b"\x89PNG....")).is_err());
        assert!(checked_screenshot("not base64!").is_err());
        let mut big = vec![0xFF, 0xD8, 0xFF];
        big.resize(MAX_SCREEN_BYTES + 1, 0);
        assert!(matches!(
            checked_screenshot(&STANDARD.encode(big)),
            Err(AppError::PayloadTooLarge)
        ));
    }

    #[test]
    fn details_drop_control_characters_and_are_bounded() {
        assert_eq!(
            clean_detail("  Chrome\n\u{7}ChatGPT ", 200),
            "ChromeChatGPT"
        );
        assert_eq!(clean_detail(&"x".repeat(500), 200).len(), 200);
    }
}
