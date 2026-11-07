//! Explicit local-only SFU test. No microphones, output devices or real accounts.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use sanser_server::{build_router, config::Config, db, state::AppState, voice_lab};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;

async fn post(app: &Router, path: &str, body: Value, token: Option<&str>) -> Value {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "local test API rejected request: {}",
        response.status()
    );
    let data = response.into_body().collect().await.unwrap().to_bytes();
    if data.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&data).unwrap()
    }
}
async fn wait_json(path: &Path, deadline: Duration) -> Value {
    tokio::time::timeout(deadline, async {
        loop {
            assert!(
                !path.parent().unwrap().join("error.json").exists(),
                "synthetic peer driver failed; inspect its generic error report"
            );
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(value) = serde_json::from_slice(&bytes) {
                    return value;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("local synthetic peer deadline")
}

#[tokio::test]
#[ignore = "requires isolated loopback PostgreSQL, LiveKit 1.13.11 and the documented Go peer driver"]
async fn real_sfu_closure_stops_three_synthetic_audio_peers() {
    let sample_seconds: u64 = std::env::var("SANSER_LIVEKIT_TEST_SAMPLE_SECONDS")
        .unwrap_or_else(|_| "10".into())
        .parse()
        .expect("numeric sample duration");
    assert!((10..=600).contains(&sample_seconds));
    let db_url = std::env::var("TEST_DATABASE_URL").expect("dedicated loopback database");
    let mut parsed = url::Url::parse(&db_url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert_eq!(parsed.port(), Some(55439));
    assert!(!parsed.query_pairs().any(|(key, _)| key == "options"));
    let exchange = PathBuf::from(
        std::env::var("SANSER_LIVEKIT_TEST_EXCHANGE_DIR")
            .expect("fresh private temporary exchange directory"),
    );
    assert!(exchange.starts_with(std::env::temp_dir()) || exchange.starts_with("/private/tmp"));
    assert_eq!(
        std::fs::read_dir(&exchange).unwrap().count(),
        0,
        "exchange directory must start empty"
    );
    let tone = PathBuf::from(
        std::env::var("SANSER_LIVEKIT_TEST_TONE").expect("synthetic Opus file, not a recording"),
    );
    assert!(tone.is_file());
    Config::test(db_url.clone()).unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();
    let schema = format!("sanser_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA \"{schema}\" "))
        .execute(&admin)
        .await
        .unwrap();
    parsed
        .query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let mut config = Config::test(parsed.into()).unwrap();
    config.voice_lab = Some(
        voice_lab::VoiceLabConfig::new("ws://127.0.0.1:17880", "test_key", "a".repeat(32))
            .unwrap()
            .with_gateway("ws://127.0.0.1:17883")
            .unwrap(),
    );
    let pool = db::connect(&config).await.unwrap();
    let state = AppState::new(config, pool);
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:17883")
        .await
        .unwrap();
    let gateway_app = app.clone();
    let gateway_stop = state.shutdown.clone();
    let gateway = tokio::spawn(async move {
        axum::serve(listener, gateway_app)
            .with_graceful_shutdown(gateway_stop.cancelled_owned())
            .await
            .unwrap();
    });
    let auth=post(&app,"/api/v2/auth/register",json!({"email":format!("{}@example.test",Uuid::new_v4()),"password":"OnlySyntheticTest123"}),None).await;
    let access = auth["accessToken"].as_str().unwrap();
    let mut devices = Vec::new();
    for index in 0..3 {
        let id = Uuid::new_v4().to_string();
        post(&app,"/api/v2/devices/register",json!({"id":id,"computerId":Uuid::new_v4().to_string(),"deviceRole":if index==0{"host"}else{"client"},
            "name":"Synthetic peer","platform":"macos-aarch64","sanserVersion":"2.3.3","crossPlatform":true,"nativeTransport":true,"codecs":["h264"]}),Some(access)).await;
        post(
            &app,
            "/api/v2/devices/heartbeat",
            json!({"deviceId":id,"streaming":false}),
            Some(access),
        )
        .await;
        devices.push(id);
    }
    let group = post(
        &app,
        "/api/v2/sharing",
        json!({"hostDeviceId":devices[0]}),
        Some(access),
    )
    .await;
    let group = group["id"].as_str().unwrap();
    let sharing_path = format!("/api/v2/sharing/{group}");
    for viewer in devices.iter().skip(1) {
        let invited = post(
            &app,
            &format!("{sharing_path}/invite"),
            json!({"viewerDeviceId":viewer}),
            Some(access),
        )
        .await;
        let member = invited["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["viewerDeviceId"] == *viewer)
            .unwrap()["id"]
            .as_str()
            .unwrap();
        post(
            &app,
            &format!("{sharing_path}/members/{member}/respond"),
            json!({"viewerDeviceId":viewer,"accept":true}),
            Some(access),
        )
        .await;
    }
    let mut tokens = Vec::new();
    for device in &devices {
        let grant = post(
            &app,
            &format!("{sharing_path}/voice-lab/credentials"),
            json!({"deviceId":device}),
            Some(access),
        )
        .await;
        tokens.push(grant["token"].as_str().unwrap().to_owned());
    }
    let data =
        serde_json::to_vec(&json!({"url":"ws://127.0.0.1:17883","tokens":tokens,"tone":tone,"sampleSeconds":sample_seconds}))
            .unwrap();
    // Model a live app during the whole sample, including fresh presence and
    // the production room-closure worker. Do not hide expiry bugs by running
    // revocation only after the audio test finishes.
    let heartbeat_app = app.clone();
    let heartbeat_devices = devices.clone();
    let heartbeat_access = access.to_owned();
    let heartbeat_stop = state.shutdown.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = heartbeat_stop.cancelled() => break,
                () = tokio::time::sleep(Duration::from_secs(5)) => {
                    for device in &heartbeat_devices {
                        post(&heartbeat_app, "/api/v2/devices/heartbeat", json!({"deviceId":device,"streaming":false}),Some(&heartbeat_access)).await;
                    }
                }
            }
        }
    });
    let closer = voice_lab::spawn_room_closer(state.clone(), state.shutdown.clone()).unwrap();
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(exchange.join("credentials.json")).unwrap();
    file.write_all(&data).unwrap();
    drop(file);
    let ready = wait_json(
        &exchange.join("ready.json"),
        Duration::from_secs(sample_seconds + 30),
    )
    .await;
    assert_eq!(ready["connected"], 3);
    assert!(ready["sampleDurationMs"].as_u64().unwrap() >= sample_seconds * 1000);
    assert_eq!(ready["receivedTracks"], json!([2, 2, 2]));
    assert!(
        ready["rtpPackets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n.as_u64().unwrap() >= 400)
    );
    post(
        &app,
        &format!("{sharing_path}/stop"),
        json!({}),
        Some(access),
    )
    .await;
    let result = wait_json(&exchange.join("result.json"), Duration::from_secs(25)).await;
    assert_eq!(result["disconnected"], 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM voice_lab_rooms WHERE closed_at IS NOT NULL"
        )
        .fetch_one(&state.pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(result["cachedTokenRejoinSucceeded"], false);
    assert_eq!(result["refreshedTokenRejoinSucceeded"], false);
    println!(
        "{}",
        json!({"connected":3,"disconnected":3,"rtpPackets":result["rtpPackets"],
        "sampleDurationMs":result["sampleDurationMs"],"receivedTracks":result["receivedTracks"],
        "cachedTokenRejoinSucceeded":result["cachedTokenRejoinSucceeded"],
        "refreshedTokenRejoinSucceeded":result["refreshedTokenRejoinSucceeded"],
        "gatewayRevocationVerified":true,"hardwareAudioVerified":false})
    );
    state.shutdown.cancel();
    heartbeat.await.unwrap();
    closer.await.unwrap();
    gateway.await.unwrap();
    let denied = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{sharing_path}/voice-lab/credentials"))
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {access}"))
                .body(Body::from(json!({"deviceId":devices[0]}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    state.pool.close().await;
    sqlx::query(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
