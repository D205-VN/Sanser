//! LiveKit signaling admission boundary. The SFU HTTP listener MUST be private.
//! Audio travels over WebRTC directly; this proxy never queues/decodes audio.
use super::*;
use axum::{
    extract::{
        OriginalUri,
        ws::{Message, WebSocketUpgrade},
    },
    http::{HeaderMap, Uri},
};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{self, protocol::WebSocketConfig},
};

const MAX_SIGNAL: usize = 256 * 1024;

// No Debug: tokens/opaque routing identifiers must not enter diagnostics.
struct Admission {
    room: String,
    identity: String,
}

impl VoiceLabConfig {
    fn verify_join(&self, token: &str, now: i64) -> Result<Admission, AppError> {
        let denied = || AppError::Unauthorized;
        if token.len() > 8192 {
            return Err(denied());
        }
        let parts: Vec<_> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(denied());
        }
        let decode = |s| URL_SAFE_NO_PAD.decode(s).map_err(|_| denied());
        let header: serde_json::Value =
            serde_json::from_slice(&decode(parts[0])?).map_err(|_| denied())?;
        if header["alg"] != "HS256" || header.get("crit").is_some() {
            return Err(denied());
        }
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.secret.as_bytes()).map_err(|_| denied())?;
        mac.update(format!("{}.{}", parts[0], parts[1]).as_bytes());
        mac.verify_slice(&decode(parts[2])?).map_err(|_| denied())?;
        let claims: serde_json::Value =
            serde_json::from_slice(&decode(parts[1])?).map_err(|_| denied())?;
        let video = &claims["video"];
        if claims["iss"] != self.key
            || claims["exp"].as_i64().is_none_or(|v| v <= now)
            || claims["nbf"].as_i64().is_none_or(|v| v > now)
            || video["roomJoin"] != true
            || video["canSubscribe"] != true
            || video["canPublish"] != true
            || video["canPublishSources"] != serde_json::json!(["microphone"])
            || video["canPublishData"] != false
            || [
                "roomAdmin",
                "roomCreate",
                "roomList",
                "roomRecord",
                "ingressAdmin",
                "canUpdateOwnMetadata",
            ]
            .iter()
            .any(|name| video.get(name).is_some_and(|v| v != false))
        {
            return Err(denied());
        }
        let room = video["room"].as_str().ok_or_else(denied)?;
        let identity = claims["sub"].as_str().ok_or_else(denied)?;
        let valid = |value: &str, prefix: &str| {
            value.strip_prefix(prefix).is_some_and(|v| {
                v.len() == 43
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        };
        if !valid(room, "snvl_") || !valid(identity, "snp_") {
            return Err(denied());
        }
        Ok(Admission {
            room: room.into(),
            identity: identity.into(),
        })
    }
}

pub(super) async fn admitted(
    state: &AppState,
    room: &str,
    identity: &str,
) -> Result<bool, AppError> {
    let config = state
        .config
        .voice_lab
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let now = now_unix();
    tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_scalar::<_, bool>("SELECT sanser_voice_admission_valid($1,$2,$3,$4,$5,$6)")
            .bind(room)
            .bind(identity)
            .bind(config.sfu_scope())
            .bind(now)
            .bind(now.saturating_sub(state.config.device_offline_after.as_secs() as i64))
            .bind(now.saturating_sub(crate::auth::SESSION_IDLE_SECONDS - 1))
            .fetch_one(&state.pool),
    )
    .await
    .map_err(|_| AppError::Unavailable)?
    .map_err(AppError::from_db)
}

async fn authorize(
    state: &AppState,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<(Admission, url::Url), AppError> {
    let config = state
        .config
        .voice_lab
        .as_ref()
        .filter(|c| c.gateway_url.is_some())
        .ok_or(AppError::Unavailable)?;
    let query = uri.query().unwrap_or_default();
    if query.len() > 64 * 1024 {
        return Err(AppError::Unauthorized);
    }
    let tokens: Vec<_> = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(k, _)| k == "access_token")
        .map(|(_, v)| v.into_owned())
        .collect();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let token = match (tokens.as_slice(), bearer) {
        ([token], None) => token.as_str(),
        ([], Some(token)) => token,
        ([token], Some(bearer)) if token == bearer => bearer,
        _ => return Err(AppError::Unauthorized),
    };
    let grant = config.verify_join(token, now_unix())?;
    if !admitted(state, &grant.room, &grant.identity).await? {
        return Err(AppError::Forbidden);
    }
    let mut upstream = url::Url::parse(&config.url).map_err(|_| AppError::Internal)?;
    upstream.set_path(uri.path());
    upstream.set_query(Some(query));
    if tokens.is_empty() {
        upstream
            .query_pairs_mut()
            .append_pair("access_token", token);
    }
    Ok((grant, upstream))
}

pub async fn validate(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let (_, mut upstream) = authorize(&state, &uri, &headers).await?;
    upstream
        .set_scheme("http")
        .map_err(|_| AppError::Internal)?;
    let config = state
        .config
        .voice_lab
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let response = config
        .admin_client
        .get(upstream)
        .send()
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !response.status().is_success() {
        return Err(AppError::Forbidden);
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], "success"))
}

pub async fn socket(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let (grant, upstream) = authorize(&state, &uri, &headers).await?;
    let options = WebSocketConfig::default()
        .max_message_size(Some(MAX_SIGNAL))
        .max_frame_size(Some(MAX_SIGNAL));
    let (upstream, _) = tokio::time::timeout(
        Duration::from_secs(3),
        connect_async_with_config(upstream.as_str(), Some(options), true),
    )
    .await
    .map_err(|_| AppError::Unavailable)?
    .map_err(|_| AppError::Unavailable)?;
    // Recheck after upstream handshake too: removal may race its network wait.
    if !admitted(&state, &grant.room, &grant.identity).await? {
        return Err(AppError::Forbidden);
    }
    Ok(ws.max_message_size(MAX_SIGNAL).max_frame_size(MAX_SIGNAL).on_upgrade(move |client| async move {
        let (mut client_tx,mut client_rx)=client.split();
        let (mut remote_tx,mut remote_rx)=upstream.split();
        let mut check=tokio::time::interval(Duration::from_secs(2));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=state.shutdown.cancelled()=>break,
                _=check.tick()=>if !admitted(&state,&grant.room,&grant.identity).await.unwrap_or(false) {
                    // A raw socket close can start an SDK reconnect loop and
                    // hide revocation until its retry deadline. Send the same
                    // terminal LiveKit signal as room closure before closing.
                    let leave=Message::Text(r#"{"leave":{"canReconnect":false,"reason":"ROOM_CLOSED"}}"#.into());
                    let _=tokio::time::timeout(Duration::from_secs(2),client_tx.send(leave)).await;
                    break;
                },
                message=client_rx.next()=>{
                    let message=match message { Some(Ok(Message::Text(v)))=>tungstenite::Message::Text(v.as_str().into()),
                        Some(Ok(Message::Binary(v)))=>tungstenite::Message::Binary(v),
                        Some(Ok(Message::Ping(_)|Message::Pong(_)))=>continue, _=>break };
                    if !matches!(tokio::time::timeout(Duration::from_secs(2),remote_tx.send(message)).await,Ok(Ok(()))) { break; }
                },
                message=remote_rx.next()=>{
                    let message=match message { Some(Ok(tungstenite::Message::Text(v)))=>Message::Text(v.as_str().into()),
                        Some(Ok(tungstenite::Message::Binary(v)))=>Message::Binary(v),
                        Some(Ok(tungstenite::Message::Ping(_)|tungstenite::Message::Pong(_)))=>continue, _=>break };
                    if !matches!(tokio::time::timeout(Duration::from_secs(2),client_tx.send(message)).await,Ok(Ok(()))) { break; }
                }
            }
        }
        let _=tokio::time::timeout(Duration::from_millis(200),remote_tx.close()).await;
        let _=tokio::time::timeout(Duration::from_millis(200),client_tx.close()).await;
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_expiry_scope_and_audio_privileges_are_checked() {
        let c = VoiceLabConfig::new("ws://127.0.0.1:7880", "test_key", "a".repeat(32)).unwrap();
        let token = c.issue("u", "g", "d", "login", 0, 100).unwrap().token;
        assert!(c.verify_join(&token, 100).is_ok());
        assert!(c.verify_join(&token, 160).is_err());
        assert!(c.verify_join(&token, 90).is_err());
        let parts: Vec<_> = token.split('.').collect();
        let original: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        for (key, value) in [
            ("roomCreate", serde_json::json!(true)),
            ("canPublishSources", serde_json::json!(["camera"])),
            ("canPublishData", serde_json::json!(true)),
        ] {
            let mut claims = original.clone();
            claims["video"][key] = value;
            assert!(
                c.verify_join(&c.sign_claims(&claims).unwrap(), 100)
                    .is_err()
            );
        }
        let mut changed = token.into_bytes();
        let n = changed.len();
        changed[n - 5] = if changed[n - 5] == b'A' { b'B' } else { b'A' };
        assert!(
            c.verify_join(std::str::from_utf8(&changed).unwrap(), 100)
                .is_err()
        );
        let mut refreshed = original;
        refreshed.as_object_mut().unwrap().remove("jti");
        refreshed["exp"] = serde_json::json!(700);
        assert!(
            c.verify_join(&c.sign_claims(&refreshed).unwrap(), 200)
                .is_ok()
        );
    }
    #[test]
    fn public_signaling_or_same_origin_gateway_is_rejected() {
        let c = || VoiceLabConfig::new("ws://127.0.0.1:7880", "test_key", "a".repeat(32)).unwrap();
        assert!(c().with_gateway("ws://127.0.0.1:7881").is_ok());
        for url in [
            "ws://127.0.0.1:7880",
            "ws://voice.example",
            "wss://user:secret@voice.example",
            "wss://voice.example/path",
        ] {
            assert!(c().with_gateway(url).is_err());
        }
        assert!(
            VoiceLabConfig::new("wss://voice.example", "test_key", "a".repeat(32))
                .unwrap()
                .with_gateway("wss://gateway.example")
                .is_err()
        );
    }
}
