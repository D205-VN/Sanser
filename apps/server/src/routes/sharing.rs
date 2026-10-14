//! Sharing control plane. `accepted` means invitation consent, not streaming.
//! Native fan-out integration must activate media in a subsequent stage.
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
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSharing {
    host_device_id: String,
    #[serde(default = "default_limit")]
    max_viewers: i16,
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
    if device.device_role == role && device.native_transport && device.cross_platform {
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
    let ids = sqlx::query_scalar::<_, String>("SELECT id FROM sharing_groups WHERE user_id=$1 AND state='preparing' ORDER BY updated_at DESC LIMIT 20")
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
    let busy = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sharing_groups WHERE host_device_id=$1 AND state='preparing') OR EXISTS(SELECT 1 FROM connection_sessions WHERE state IN ('pending','accepted') AND (host_device_id=$1 OR requester_device_id=$1)) OR EXISTS(SELECT 1 FROM sharing_members WHERE viewer_device_id=$1 AND state IN ('invited','accepted'))")
        .bind(&host_id).fetch_one(&mut *tx).await.map_err(AppError::from_db)?;
    if busy {
        return Err(AppError::Conflict(
            "host already has an active session or sharing group".into(),
        ));
    }
    let sharing_id = Uuid::new_v4().to_string();
    let now = now_unix();
    sqlx::query("INSERT INTO sharing_groups(id,user_id,host_device_id,max_viewers,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$5)")
        .bind(&sharing_id).bind(&auth.user_id).bind(&host_id).bind(max_viewers).bind(now)
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
        != "preparing"
    {
        return Err(AppError::Conflict("sharing group has stopped".into()));
    }
    sqlx::query("SELECT id FROM devices WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(&viewer_id)
        .bind(&auth.user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::from_db)?;
    let busy = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM sharing_members WHERE viewer_device_id=$1 AND state IN ('invited','accepted')) OR EXISTS(SELECT 1 FROM connection_sessions WHERE state IN ('pending','accepted') AND (host_device_id=$1 OR requester_device_id=$1)) OR EXISTS(SELECT 1 FROM sharing_groups WHERE host_device_id=$1 AND state='preparing')")
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
    devices::fetch_owned(&state, &auth.user_id, &viewer_id).await?;
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
    if group_state != "preparing" {
        return Err(AppError::Conflict("sharing group has stopped".into()));
    }
    let next = if request.accept {
        "accepted"
    } else {
        "declined"
    };
    let changed=sqlx::query("UPDATE sharing_members SET state=$1,updated_at=$2 WHERE id=$3 AND sharing_id=$4 AND user_id=$5 AND viewer_device_id=$6 AND state='invited'")
        .bind(next).bind(now_unix()).bind(&member_id).bind(&sharing_id).bind(&auth.user_id).bind(&viewer_id)
        .execute(&mut *tx).await.map_err(AppError::from_db)?.rows_affected();
    if changed != 1 {
        return Err(AppError::Conflict(
            "invitation is no longer pending or belongs to another viewer".into(),
        ));
    }
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
        "sharing.updated",
        serde_json::json!({"sharingId":sharing_id}),
    )
    .await?;
    Ok(Json(fetch(&state, &auth.user_id, &sharing_id).await?))
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
