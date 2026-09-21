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
    features: [&'static str; 3],
}

const FEATURES: [&str; 3] = ["device-roles", "cross-platform-native", "session-idle-7d"];

pub async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        version: SANSER_VERSION,
        protocol_version: PROTOCOL_VERSION,
        features: FEATURES,
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
            features: FEATURES,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn health_advertises_bidirectional_registration_without_database_access() {
        let response = health().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["features"], serde_json::json!(FEATURES));
    }
}
