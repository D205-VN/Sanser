use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Serialize;

use crate::{
    config::{PROTOCOL_VERSION, SANSER_VERSION},
    db,
    state::AppState,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    protocol_version: u8,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: SANSER_VERSION,
        protocol_version: PROTOCOL_VERSION,
    })
}

pub async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let ready = tokio::time::timeout(state.config.request_timeout, db::ready(&state.pool))
        .await
        .unwrap_or(false);
    let status = if ready { "ready" } else { "unavailable" };
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status_code,
        Json(HealthResponse {
            status,
            version: SANSER_VERSION,
            protocol_version: PROTOCOL_VERSION,
        }),
    )
}
