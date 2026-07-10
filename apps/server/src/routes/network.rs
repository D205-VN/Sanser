use axum::{Json, extract::State};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha1::Sha1;

use crate::{
    auth::AuthContext, config::NetworkMode, error::AppError, state::AppState, time::expires_at,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IceConfiguration {
    ice_servers: Vec<IceServer>,
    network_mode: NetworkMode,
    ice_transport_policy: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IceServer {
    urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential: Option<String>,
}

pub async fn ice(
    State(state): State<AppState>,
    auth: AuthContext,
) -> Result<Json<IceConfiguration>, AppError> {
    let mut servers = Vec::with_capacity(2);
    if !state.config.stun_urls.is_empty() && state.config.network_mode != NetworkMode::Relay {
        servers.push(IceServer {
            urls: state.config.stun_urls.clone(),
            username: None,
            credential: None,
        });
    }

    let mut credential_expiry = None;
    if !state.config.turn_urls.is_empty() {
        let (username, credential, expiry) = if let Some(shared_secret) =
            state.config.turn_shared_secret.as_deref()
        {
            let expiry = expires_at(state.config.turn_credential_ttl);
            let username = format!("{expiry}:{}", auth.user_id);
            let mut mac = Hmac::<Sha1>::new_from_slice(shared_secret.as_bytes()).map_err(|_| {
                tracing::error!("TURN shared secret could not initialize HMAC");
                AppError::Internal
            })?;
            mac.update(username.as_bytes());
            let credential = STANDARD.encode(mac.finalize().into_bytes());
            (username, credential, Some(expiry))
        } else {
            // Static credentials are retained for small self-hosted deployments.
            // A shared secret is preferred because it produces expiring credentials.
            let username = state
                .config
                .turn_username
                .clone()
                .ok_or(AppError::Unavailable)?;
            let credential = state
                .config
                .turn_credential
                .clone()
                .ok_or(AppError::Unavailable)?;
            (username, credential, None)
        };
        credential_expiry = expiry;
        servers.push(IceServer {
            urls: state.config.turn_urls.clone(),
            username: Some(username),
            credential: Some(credential),
        });
    }

    Ok(Json(IceConfiguration {
        ice_servers: servers,
        network_mode: state.config.network_mode,
        ice_transport_policy: if state.config.network_mode == NetworkMode::Relay {
            "relay"
        } else {
            "all"
        },
        expires_at: credential_expiry,
    }))
}
