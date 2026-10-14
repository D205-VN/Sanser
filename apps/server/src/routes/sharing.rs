//! Consent creates independent authenticated view-only sessions. `accepted`
//! means permission to connect; first-frame status is reported by the engines.
use crate::{
    auth::AuthContext,
    error::{ApiJson, AppError},
    events,
    models::{Page, SessionAccess, SharingGroup, SharingMember},
    routes::devices,
    state::AppState,
    time::now_unix,
};
use axum::{
    Json,
    extract::{Path, State},
};
use sanser_core::NetworkMode;
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSharing {
    host_device_id: String,
    #[serde(default = "default_limit")]
    max_viewers: i16,
    #[serde(default)]
    network_mode: NetworkMode,
    #[serde(default = "default_quality")]
    quality_profile: String,
}
fn default_quality() -> String {
    "balanced".into()
}
fn default_limit() -> i16 {
    3
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InviteViewer {
    viewer_device_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RespondInvitation {
    viewer_device_id: String,
    accept: bool,
}

fn id(value: &str) -> Result<String, AppError> {
    Uuid::parse_str(value)
        .map(|value| value.to_string())
        .map_err(|_| AppError::Validation("a valid device or sharing UUID is required".into()))
}
fn limit(value: i16) -> Result<i16, AppError> {
    if (1..=3).contains(&value) {
        Ok(value)
    } else {
        Err(AppError::Validation(
            "maxViewers must be between 1 and 3".into(),
        ))
    }
}
fn ensure_role(device: &crate::models::Device, role: &str) -> Result<(), AppError> {
    if device.device_role == role
        && device.native_transport
        && device.cross_platform
        && super::sessions::supports_view_only(&device.sanser_version)
    {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "sharing requires an updated native host and native viewers".into(),
        ))
    }
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<Page<SharingGroup>>, AppError> {
    let ids = sqlx::query_scalar::<_, String>("SELECT id FROM sharing_groups WHERE user_id=$1 AND state IN ('preparing','active') ORDER BY updated_at DESC LIMIT 20")
        .bind(&auth.user_id).fetch_all(&state.pool).await.map_err(AppError::from_db)?;
    let mut items = Vec::new();
    for value in ids {
        items.push(fetch(&state, &auth.user_id, &value).await?);
    }
    Ok(Json(Page {
        items,
        next_cursor: None,
    }))
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(value): Path<String>,
) -> Result<Json<SharingGroup>, AppError> {
    Ok(Json(fetch(&state, &auth.user_id, &id(&value)?).await?))
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<CreateSharing>,
) -> Result<Json<SharingGroup>, AppError> {
    let host_id = id(&request.host_device_id)?;
    let max_viewers = limit(request.max_viewers)?;
    if request.network_mode == NetworkMode::Manual {
        return Err(AppError::Validation(
            "sharing supports Auto, Direct or Relay".into(),
        ));
    }
    if !matches!(
        request.quality_profile.as_str(),
        "auto" | "competitive" | "balanced" | "quality"
    ) {
        return Err(AppError::Validation(
            "invalid sharing quality profile".into(),
        ));
    }
    let host = devices::fetch_owned(&state, &auth.user_id, &host_id).await?;
    ensure_role(&host, "host")?;
    super::sessions::ensure_device_online(&state, &host)?;
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query("SELECT id FROM devices WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(&host_id)
        .bind(&auth.user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::from_db)?;
    let busy = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sharing_groups WHERE host_device_id=$1 AND state IN ('preparing','active')) OR EXISTS(SELECT 1 FROM connection_sessions WHERE state IN ('pending','accepted') AND (host_device_id=$1 OR requester_device_id=$1)) OR EXISTS(SELECT 1 FROM sharing_members WHERE viewer_device_id=$1 AND state IN ('invited','accepted'))")
        .bind(&host_id).fetch_one(&mut *tx).await.map_err(AppError::from_db)?;
    if busy {
        return Err(AppError::Conflict(
            "host already has an active session or sharing group".into(),
        ));
    }
    let sharing_id = Uuid::new_v4().to_string();
    let now = now_unix();
    sqlx::query("INSERT INTO sharing_groups(id,user_id,host_device_id,max_viewers,created_at,updated_at,network_mode,quality_profile) VALUES($1,$2,$3,$4,$5,$5,$6,$7)")
        .bind(&sharing_id).bind(&auth.user_id).bind(&host_id).bind(max_viewers).bind(now)
        .bind(super::sessions::network_mode_str(request.network_mode)).bind(&request.quality_profile)
        .execute(&mut *tx).await.map_err(AppError::from_db)?;
    tx.commit().await.map_err(AppError::from_db)?;
    Ok(Json(fetch(&state, &auth.user_id, &sharing_id).await?))
}
pub async fn invite(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(value): Path<String>,
    ApiJson(request): ApiJson<InviteViewer>,
) -> Result<Json<SharingGroup>, AppError> {
    let sharing_id = id(&value)?;
    let viewer_id = id(&request.viewer_device_id)?;
    let group = fetch(&state, &auth.user_id, &sharing_id).await?;
    let viewer = devices::fetch_owned(&state, &auth.user_id, &viewer_id).await?;
    let host = devices::fetch_owned(&state, &auth.user_id, &group.host_device_id).await?;
    ensure_role(&viewer, "client")?;
    if viewer.id == host.id
        || (host.computer_id.is_some() && host.computer_id == viewer.computer_id)
    {
        return Err(AppError::Validation(
            "a computer cannot view its own sharing group".into(),
        ));
    }
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    let locked = sqlx::query(
        "SELECT state,max_viewers FROM sharing_groups WHERE id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(&sharing_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    if locked
        .try_get::<String, _>("state")
        .map_err(AppError::from_db)?
        == "stopped"
    {
        return Err(AppError::Conflict("sharing group has stopped".into()));
    }
    sqlx::query("SELECT id FROM devices WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(&viewer_id)
        .bind(&auth.user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::from_db)?;
    let busy = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sharing_members WHERE viewer_device_id=$1 AND state IN ('invited','accepted')) OR EXISTS(SELECT 1 FROM connection_sessions WHERE state IN ('pending','accepted') AND (host_device_id=$1 OR requester_device_id=$1)) OR EXISTS(SELECT 1 FROM sharing_groups WHERE host_device_id=$1 AND state IN ('preparing','active'))")
        .bind(&viewer_id).fetch_one(&mut *tx).await.map_err(AppError::from_db)?;
    if busy {
        return Err(AppError::Conflict(
            "viewer already has an active session or invitation".into(),
        ));
    }
    let slots = sqlx::query_scalar::<_, i16>(
        "SELECT slot FROM sharing_members WHERE sharing_id=$1 AND state IN ('invited','accepted')",
    )
    .bind(&sharing_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    let max: i16 = locked.try_get("max_viewers").map_err(AppError::from_db)?;
    let slot = (1..=max)
        .find(|value| !slots.contains(value))
        .ok_or_else(|| AppError::Conflict("sharing group is full (maximum 3 viewers)".into()))?;
    let now = now_unix();
    let member_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sharing_members(id,sharing_id,user_id,viewer_device_id,slot,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$6)")
        .bind(&member_id).bind(&sharing_id).bind(&auth.user_id).bind(&viewer_id).bind(slot).bind(now)
        .execute(&mut *tx).await.map_err(AppError::from_db)?;
    sqlx::query("UPDATE sharing_groups SET updated_at=$1 WHERE id=$2 AND user_id=$3")
        .bind(now_unix())
        .bind(&sharing_id)
        .bind(&auth.user_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::from_db)?;
    tx.commit().await.map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        None,
        "sharing.invited",
        serde_json::json!({"sharingId":sharing_id,"viewerDeviceId":viewer_id}),
    )
    .await?;
    Ok(Json(fetch(&state, &auth.user_id, &sharing_id).await?))
}
pub async fn respond(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((value, member)): Path<(String, String)>,
    ApiJson(request): ApiJson<RespondInvitation>,
) -> Result<Json<SharingGroup>, AppError> {
    let sharing_id = id(&value)?;
    let member_id = id(&member)?;
    let viewer_id = id(&request.viewer_device_id)?;
    let viewer = devices::fetch_owned(&state, &auth.user_id, &viewer_id).await?;
    let group = fetch(&state, &auth.user_id, &sharing_id).await?;
    if request.accept {
        let host = devices::fetch_owned(&state, &auth.user_id, &group.host_device_id).await?;
        ensure_role(&host, "host")?;
        ensure_role(&viewer, "client")?;
        super::sessions::ensure_device_online(&state, &host)?;
        super::sessions::ensure_device_online(&state, &viewer)?;
    }
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    let group_state = sqlx::query_scalar::<_, String>(
        "SELECT state FROM sharing_groups WHERE id=$1 AND user_id=$2 FOR UPDATE",
    )
    .bind(&sharing_id)
    .bind(&auth.user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::from_db)?
    .ok_or(AppError::NotFound)?;
    if group_state == "stopped" {
        return Err(AppError::Conflict("sharing group has stopped".into()));
    }
    let member_state=sqlx::query_scalar::<_,String>("SELECT state FROM sharing_members WHERE id=$1 AND sharing_id=$2 AND user_id=$3 AND viewer_device_id=$4 FOR UPDATE")
        .bind(&member_id).bind(&sharing_id).bind(&auth.user_id).bind(&viewer_id).fetch_optional(&mut *tx).await.map_err(AppError::from_db)?
        .ok_or_else(|| AppError::Conflict("invitation belongs to another viewer".into()))?;
    if member_state == "accepted" && request.accept {
        tx.commit().await.map_err(AppError::from_db)?;
        return Ok(Json(fetch(&state, &auth.user_id, &sharing_id).await?));
    }
    if member_state != "invited" {
        return Err(AppError::Conflict("invitation is no longer pending".into()));
    }
    let now = now_unix();
    let session_id = if request.accept {
        sqlx::query("SELECT id FROM devices WHERE id=$1 AND user_id=$2 FOR UPDATE")
            .bind(&viewer_id)
            .bind(&auth.user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(AppError::from_db)?;
        let busy=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM connection_sessions WHERE user_id=$1 AND state IN ('pending','accepted') AND (requester_device_id=$2 OR host_device_id=$2))")
            .bind(&auth.user_id).bind(&viewer_id).fetch_one(&mut *tx).await.map_err(AppError::from_db)?;
        if busy {
            return Err(AppError::Conflict(
                "viewer already has an active media session".into(),
            ));
        }
        let session_id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO connection_sessions(id,user_id,requester_device_id,host_device_id,state,access_mode,network_mode,quality_profile,requested_codec,selected_transport,sharing_id,created_at,updated_at,accepted_at) VALUES($1,$2,$3,$4,'accepted','view-only',$5,$6,'h264','native',$7,$8,$8,$8)")
            .bind(&session_id).bind(&auth.user_id).bind(&viewer_id).bind(&group.host_device_id)
            .bind(super::sessions::network_mode_str(group.network_mode)).bind(&group.quality_profile).bind(&sharing_id).bind(now)
            .execute(&mut *tx).await.map_err(AppError::from_db)?;
        Some(session_id)
    } else {
        None
    };
    let next = if request.accept {
        "accepted"
    } else {
        "declined"
    };
    sqlx::query("UPDATE sharing_members SET state=$1,updated_at=$2,session_id=$3 WHERE id=$4 AND user_id=$5")
        .bind(next).bind(now).bind(&session_id).bind(&member_id).bind(&auth.user_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    sqlx::query("UPDATE sharing_groups SET state=CASE WHEN $1 THEN 'active' ELSE state END,updated_at=$2 WHERE id=$3 AND user_id=$4")
        .bind(request.accept).bind(now).bind(&sharing_id).bind(&auth.user_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    tx.commit().await.map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        session_id.as_deref(),
        "sharing.updated",
        serde_json::json!({"sharingId":sharing_id}),
    )
    .await?;
    Ok(Json(fetch(&state, &auth.user_id, &sharing_id).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharingHeartbeat {
    host_device_id: String,
}
pub async fn heartbeat(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(value): Path<String>,
    ApiJson(request): ApiJson<SharingHeartbeat>,
) -> Result<axum::http::StatusCode, AppError> {
    let sharing_id = id(&value)?;
    let host = id(&request.host_device_id)?;
    let updated=sqlx::query("UPDATE sharing_groups SET updated_at=$1 WHERE id=$2 AND user_id=$3 AND host_device_id=$4 AND state IN ('preparing','active')")
        .bind(now_unix()).bind(sharing_id).bind(&auth.user_id).bind(host).execute(&state.pool).await.map_err(AppError::from_db)?.rows_affected();
    if updated != 1 {
        return Err(AppError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub(super) async fn refresh_streaming(
    connection: &mut sqlx::PgConnection,
    user: &str,
    sharing: &str,
    now: i64,
) -> Result<(), AppError> {
    sqlx::query("UPDATE devices SET streaming=EXISTS(SELECT 1 FROM sharing_members AS member JOIN connection_sessions AS session ON session.id=member.session_id AND session.user_id=member.user_id WHERE member.sharing_id=$3 AND member.user_id=$2 AND member.state='accepted' AND member.started_at IS NOT NULL AND session.state='accepted'),updated_at=$1 WHERE user_id=$2 AND id=(SELECT host_device_id FROM sharing_groups WHERE id=$3 AND user_id=$2)")
        .bind(now).bind(user).bind(sharing).execute(connection).await.map_err(AppError::from_db)?;
    Ok(())
}

pub async fn remove(
    State(state): State<AppState>,
    auth: AuthContext,
    Path((value, member)): Path<(String, String)>,
) -> Result<Json<SharingGroup>, AppError> {
    let sharing = id(&value)?;
    let member = id(&member)?;
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query("SELECT id FROM sharing_groups WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(&sharing)
        .bind(&auth.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?;
    let session=sqlx::query_scalar::<_,Option<String>>("SELECT session_id FROM sharing_members WHERE id=$1 AND sharing_id=$2 AND user_id=$3 FOR UPDATE")
        .bind(&member).bind(&sharing).bind(&auth.user_id).fetch_optional(&mut *tx).await.map_err(AppError::from_db)?.ok_or(AppError::NotFound)?;
    let now = now_unix();
    if let Some(session) = session {
        sqlx::query("UPDATE connection_sessions SET state='disconnected',ended_at=$1,updated_at=$1,disconnect_reason='sharing_member_removed' WHERE id=$2 AND user_id=$3 AND state IN ('pending','accepted')")
            .bind(now).bind(session).bind(&auth.user_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    }
    sqlx::query("UPDATE sharing_members SET state='removed',updated_at=$1 WHERE id=$2 AND user_id=$3 AND state IN ('invited','accepted')")
        .bind(now).bind(&member).bind(&auth.user_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    refresh_streaming(&mut tx, &auth.user_id, &sharing, now).await?;
    tx.commit().await.map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        None,
        "sharing.updated",
        serde_json::json!({"sharingId":sharing}),
    )
    .await?;
    Ok(Json(fetch(&state, &auth.user_id, &sharing).await?))
}
pub async fn stop(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(value): Path<String>,
) -> Result<Json<SharingGroup>, AppError> {
    let sharing_id = id(&value)?;
    let mut tx = state.pool.begin().await.map_err(AppError::from_db)?;
    sqlx::query("SELECT id FROM sharing_groups WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(&sharing_id)
        .bind(&auth.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?;
    let now = now_unix();
    sqlx::query(
        "UPDATE sharing_groups SET state='stopped',updated_at=$1 WHERE id=$2 AND user_id=$3",
    )
    .bind(now)
    .bind(&sharing_id)
    .bind(&auth.user_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::from_db)?;
    sqlx::query("UPDATE sharing_members SET state='removed',updated_at=$1 WHERE sharing_id=$2 AND user_id=$3 AND state IN ('invited','accepted')")
        .bind(now).bind(&sharing_id).bind(&auth.user_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    sqlx::query("UPDATE connection_sessions SET state='disconnected',ended_at=$1,updated_at=$1,disconnect_reason='sharing_stopped' WHERE sharing_id=$2 AND user_id=$3 AND state IN ('pending','accepted')")
        .bind(now).bind(&sharing_id).bind(&auth.user_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    sqlx::query("UPDATE devices SET streaming=FALSE,updated_at=$1 WHERE user_id=$2 AND id=(SELECT host_device_id FROM sharing_groups WHERE id=$3 AND user_id=$2)")
        .bind(now).bind(&auth.user_id).bind(&sharing_id).execute(&mut *tx).await.map_err(AppError::from_db)?;
    tx.commit().await.map_err(AppError::from_db)?;
    events::publish(
        &state,
        &auth.user_id,
        None,
        "sharing.stopped",
        serde_json::json!({"sharingId":sharing_id}),
    )
    .await?;
    Ok(Json(fetch(&state, &auth.user_id, &sharing_id).await?))
}
async fn fetch(state: &AppState, user: &str, value: &str) -> Result<SharingGroup, AppError> {
    let row = sqlx::query("SELECT * FROM sharing_groups WHERE id=$1 AND user_id=$2")
        .bind(value)
        .bind(user)
        .fetch_optional(&state.pool)
        .await
        .map_err(AppError::from_db)?
        .ok_or(AppError::NotFound)?;
    let rows=sqlx::query("SELECT * FROM sharing_members WHERE sharing_id=$1 AND user_id=$2 ORDER BY (state IN ('invited','accepted')) DESC,created_at DESC,id LIMIT 20")
        .bind(value).bind(user).fetch_all(&state.pool).await.map_err(AppError::from_db)?;
    let members = rows
        .into_iter()
        .map(|r| {
            Ok(SharingMember {
                id: r.try_get("id")?,
                viewer_device_id: r.try_get("viewer_device_id")?,
                state: r.try_get("state")?,
                access_mode: SessionAccess::ViewOnly,
                session_id: r.try_get("session_id")?,
                started_at: r.try_get("started_at")?,
                created_at: r.try_get("created_at")?,
                updated_at: r.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(AppError::from_db)?;
    Ok(SharingGroup {
        id: row.try_get("id").map_err(AppError::from_db)?,
        host_device_id: row.try_get("host_device_id").map_err(AppError::from_db)?,
        state: row.try_get("state").map_err(AppError::from_db)?,
        max_viewers: row.try_get("max_viewers").map_err(AppError::from_db)?,
        network_mode: super::sessions::parse_network_mode(
            &row.try_get::<String, _>("network_mode")
                .map_err(AppError::from_db)?,
        )?,
        quality_profile: row.try_get("quality_profile").map_err(AppError::from_db)?,
        created_at: row.try_get("created_at").map_err(AppError::from_db)?,
        updated_at: row.try_get("updated_at").map_err(AppError::from_db)?,
        members,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_capacity_and_explicit_consent() {
        assert!(limit(0).is_err());
        assert!(limit(4).is_err());
        assert!(limit(3).is_ok());
        assert!(serde_json::from_str::<RespondInvitation>(r#"{"viewerDeviceId":"v"}"#).is_err());
        assert!(
            serde_json::from_str::<InviteViewer>(
                r#"{"viewerDeviceId":"v","accessMode":"control"}"#
            )
            .is_err()
        );
    }
}
