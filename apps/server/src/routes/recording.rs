//! `/api/v2/sessions/{id}/recording`: consent relay between a recording
//! viewer and the host. No media or file names pass through the server.

use std::time::Instant;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::AuthContext,
    error::{ApiJson, AppError},
    recording_consent::{Action, ConsentError, ConsentView, Participants},
    routes::{devices, sessions},
    state::AppState,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingRequest {
    device_id: String,
    action: String,
}

fn uuid(value: &str, field: &str) -> Result<String, AppError> {
    Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| AppError::Validation(format!("{field} must be a UUID")))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<ConsentView>, AppError> {
    let id = uuid(&id, "id")?;
    let current = sessions::fetch_owned(&state, &auth.user_id, &id).await?;
    if current.state != "accepted" {
        return Ok(Json(ConsentView {
            state: crate::recording_consent::ConsentState::None,
            viewer_name: None,
            viewer_device_id: None,
        }));
    }
    Ok(Json(
        state.recording_consents.get(&id, Instant::now()).await,
    ))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthContext,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<RecordingRequest>,
) -> Result<Json<ConsentView>, AppError> {
    let id = uuid(&id, "id")?;
    let device = uuid(&request.device_id, "deviceId")?;
    let action = Action::parse(&request.action).ok_or_else(|| {
        AppError::Validation("action must be request, stop, allow or deny".into())
    })?;
    let current = sessions::fetch_owned(&state, &auth.user_id, &id).await?;
    if current.state != "accepted" && action != Action::Stop {
        return Err(AppError::Conflict(
            "recording requires an accepted session".into(),
        ));
    }
    let viewer = devices::fetch_owned(&state, &auth.user_id, &current.requester_device_id).await?;
    let participants = Participants {
        requester_device_id: &current.requester_device_id,
        host_device_id: &current.host_device_id,
        viewer_name: &viewer.name,
    };
    state
        .recording_consents
        .apply(&id, &participants, &device, action, Instant::now())
        .await
        .map(Json)
        .map_err(|error| match error {
            ConsentError::Forbidden => AppError::Forbidden,
            ConsentError::Conflict(message) => AppError::Conflict(message.into()),
        })
}
