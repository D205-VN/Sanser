//! Stream settings advisor. See docs/stream-advisor.md.
//!
//! The app sends what it knows about both computers and the network between
//! them. When an OpenAI-compatible model (LM Studio) is configured the server
//! asks it for settings; its answer is then checked and clamped by the same
//! limits as the rules. Without a model, or when it is slow or answers
//! nonsense, the rules answer instead. The model is never reachable from
//! outside: only this signed-in endpoint talks to it.

use std::time::Duration;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    auth::AuthContext,
    config::AdvisorConfig,
    error::{ApiJson, AppError},
    state::AppState,
};

/// Below the request timeout, so a slow model still leaves time for the rules.
const MODEL_TIMEOUT: Duration = Duration::from_secs(11);
const RESOLUTIONS: [&str; 5] = ["auto", "720p", "1080p", "1440p", "2160p"];
const FRAME_RATES: [u32; 4] = [30, 60, 90, 120];
const CODECS: [&str; 3] = ["auto", "h264", "hevc"];
const NETWORK_MODES: [&str; 3] = ["auto", "direct", "relay"];
/// What the shared relay can carry for one session without hurting others.
const RELAY_MAX_MBPS: u32 = 15;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineProfile {
    #[serde(default)]
    platform: String,
    #[serde(default)]
    os_version: String,
    #[serde(default)]
    gpu: String,
    #[serde(default)]
    cpu_model: String,
    #[serde(default)]
    cpu_cores: Option<u32>,
    #[serde(default)]
    memory_gb: Option<u32>,
    #[serde(default)]
    display_width: Option<u32>,
    #[serde(default)]
    display_height: Option<u32>,
    #[serde(default)]
    refresh_hz: Option<u32>,
    #[serde(default)]
    codecs: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkProfile {
    /// `direct`, `relay` or `unknown`: what the last session used.
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    rtt_ms: Option<u32>,
    #[serde(default)]
    udp_works: Option<bool>,
    #[serde(default)]
    server_ms: Option<u32>,
    /// Measured just now: the viewer's download speed from the server.
    #[serde(default)]
    download_mbps: Option<f64>,
    /// Measured just now: the host's upload speed to the server.
    #[serde(default)]
    upload_mbps: Option<f64>,
}

impl NetworkProfile {
    /// What the path between the two computers carried in the live test:
    /// the slower of the host's upload and the viewer's download.
    fn measured_mbps(&self) -> Option<f64> {
        let sane = |value: Option<f64>| value.filter(|mbps| mbps.is_finite() && *mbps > 0.0);
        match (sane(self.download_mbps), sane(self.upload_mbps)) {
            (Some(down), Some(up)) => Some(down.min(up)),
            (one, other) => one.or(other),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdviceRequest {
    viewer: MachineProfile,
    host: MachineProfile,
    #[serde(default)]
    network: NetworkProfile,
    #[serde(default)]
    locale: Option<String>,
    /// What the person cares about most: `balanced` (default), `smooth`,
    /// `sharp` or `responsive`.
    #[serde(default)]
    goal: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    Balanced,
    Smooth,
    Sharp,
    Responsive,
}

impl Goal {
    fn of(request: &AdviceRequest) -> Self {
        match request.goal.as_deref() {
            Some("smooth") => Self::Smooth,
            Some("sharp") => Self::Sharp,
            Some("responsive") => Self::Responsive,
            _ => Self::Balanced,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Balanced => "a balance of smoothness and sharpness",
            Self::Smooth => {
                "smooth and stable motion first (high fps, fewer stutters), sharpness second"
            }
            Self::Sharp => "a sharp and stable picture first (high resolution), fps second",
            Self::Responsive => {
                "the fastest response to input first (small frames, high fps, low bitrate)"
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    resolution: String,
    fps: u32,
    bitrate_mbps: u32,
    codec: String,
    network_mode: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Advice {
    /// `model` when the language model answered, `rules` otherwise.
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    settings: Settings,
    reasons: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisorStatus {
    model_available: bool,
}

pub async fn status(State(state): State<AppState>, _auth: AuthContext) -> Json<AdvisorStatus> {
    Json(AdvisorStatus {
        model_available: state.config.advisor.is_some(),
    })
}

pub async fn advise(
    State(state): State<AppState>,
    auth: AuthContext,
    ApiJson(request): ApiJson<AdviceRequest>,
) -> Result<Json<Advice>, AppError> {
    if !state.advisor_limiter.check(auth.user_id.clone()).await {
        return Err(AppError::RateLimited);
    }
    let vietnamese = request.locale.as_deref() != Some("en");
    let (rules, rule_reasons) = rule_settings(&request, vietnamese);
    if let Some(config) = state.config.advisor.as_ref() {
        match ask_model(config, &request, &rules, vietnamese).await {
            Ok((settings, reasons)) => {
                let reasons = readable_reasons(reasons);
                return Ok(Json(Advice {
                    source: "model",
                    model: Some(config.model.clone()),
                    settings: near_rules(clamp(settings, &request), &rules),
                    reasons: if reasons.is_empty() {
                        rule_reasons
                    } else {
                        reasons
                    },
                }));
            }
            Err(reason) => tracing::warn!(reason, "stream advisor model unavailable; using rules"),
        }
    }
    Ok(Json(Advice {
        source: "rules",
        model: None,
        settings: rules,
        reasons: rule_reasons,
    }))
}

fn both_support_hevc(request: &AdviceRequest) -> bool {
    let has = |profile: &MachineProfile| {
        profile
            .codecs
            .iter()
            .any(|codec| codec.eq_ignore_ascii_case("hevc") || codec.eq_ignore_ascii_case("h265"))
    };
    has(&request.viewer) && has(&request.host)
}

fn on_relay(request: &AdviceRequest) -> bool {
    request.network.path.as_deref() == Some("relay") || request.network.udp_works == Some(false)
}

/// The tallest resolution worth sending: the viewer's screen bounds it.
fn resolution_cap(request: &AdviceRequest) -> &'static str {
    match request
        .viewer
        .display_height
        .or(request.host.display_height)
    {
        Some(height) if height <= 800 => "720p",
        Some(height) if height <= 1200 => "1080p",
        Some(height) if height <= 1600 => "1440p",
        Some(_) => "2160p",
        None => "1080p",
    }
}

fn resolution_rank(value: &str) -> usize {
    RESOLUTIONS
        .iter()
        .position(|item| *item == value)
        .unwrap_or(0)
}

/// Moves a resolution one step up or down, between 720p and the cap.
/// `auto` counts as the cap.
fn step_resolution(value: &str, up: bool, cap: &'static str) -> &'static str {
    let top = resolution_rank(cap).max(1);
    let rank = if value == "auto" {
        top
    } else {
        resolution_rank(value).max(1)
    };
    let next = if up { rank + 1 } else { rank.saturating_sub(1) };
    RESOLUTIONS[next.clamp(1, top)]
}

/// Keeps a model answer inside what both computers and the route can do.
pub fn clamp(mut settings: Settings, request: &AdviceRequest) -> Settings {
    if !RESOLUTIONS.contains(&settings.resolution.as_str()) {
        settings.resolution = "auto".into();
    }
    let cap = resolution_cap(request);
    if settings.resolution != "auto" && resolution_rank(&settings.resolution) > resolution_rank(cap)
    {
        settings.resolution = cap.into();
    }
    if !FRAME_RATES.contains(&settings.fps) {
        settings.fps = 60;
    }
    let refresh = request
        .viewer
        .refresh_hz
        .unwrap_or(60)
        .min(request.host.refresh_hz.unwrap_or(60));
    if settings.fps > 60 && settings.fps > refresh {
        settings.fps = 60;
    }
    if !CODECS.contains(&settings.codec.as_str())
        || (settings.codec == "hevc" && !both_support_hevc(request))
    {
        settings.codec = "auto".into();
    }
    if Goal::of(request) == Goal::Responsive && settings.codec == "hevc" {
        settings.codec = "h264".into();
    }
    if !NETWORK_MODES.contains(&settings.network_mode.as_str()) {
        settings.network_mode = "auto".into();
    }
    if request.network.udp_works == Some(false) && settings.network_mode == "direct" {
        settings.network_mode = "auto".into();
    }
    let ceiling = if on_relay(request) || settings.network_mode == "relay" {
        RELAY_MAX_MBPS
    } else {
        80
    };
    settings.bitrate_mbps = settings.bitrate_mbps.clamp(2, ceiling);
    settings
}

pub fn rule_settings(request: &AdviceRequest, vietnamese: bool) -> (Settings, Vec<String>) {
    let say = |vi: &str, en: &str| {
        if vietnamese {
            vi.to_owned()
        } else {
            en.to_owned()
        }
    };
    let mut reasons = Vec::new();
    let rtt = request
        .network
        .rtt_ms
        .or(request.network.server_ms.map(|ms| ms * 2));
    let relay = on_relay(request);
    let measured = request.network.measured_mbps();
    let (mut resolution, mut fps, bitrate): (&'static str, u32, u32) = if let Some(mbps) = measured
    {
        // Leave about 40% unused so short dips do not stall the picture.
        let budget = (mbps * 0.6).floor().clamp(2.0, 80.0) as u32;
        reasons.push(if vietnamese {
            format!("Đo được khoảng {mbps:.0} Mb/s giữa hai máy; dùng {budget} Mb/s để còn dư cho ổn định.")
        } else {
            format!("Measured about {mbps:.0} Mb/s between the computers; {budget} Mb/s leaves headroom for stability.")
        });
        let (resolution, fps) = match budget {
            0..5 => ("720p", 30),
            5..9 => ("720p", 60),
            9..16 => ("1080p", 60),
            16..28 => ("1440p", 60),
            _ => ("auto", 60),
        };
        if rtt.is_some_and(|ms| ms > 150) {
            reasons.push(say(
                "Độ trễ cao: thao tác sẽ hơi chậm dù hình vẫn mượt.",
                "Latency is high: input will feel a little slow even though the picture stays smooth.",
            ));
        }
        (resolution, fps, budget)
    } else if relay {
        reasons.push(say(
            "Mạng đang đi qua relay nên giữ bitrate thấp để hình không bị giật.",
            "The connection uses the relay, so a low bitrate keeps the picture smooth.",
        ));
        match rtt {
            Some(ms) if ms > 150 => ("720p", 30, 6),
            Some(ms) if ms > 80 => ("1080p", 30, 8),
            _ => ("1080p", 60, 10),
        }
    } else {
        match rtt {
            Some(ms) if ms <= 30 => {
                reasons.push(say(
                    "Độ trễ thấp, mạng đủ cho chất lượng cao.",
                    "Latency is low, so the network can carry high quality.",
                ));
                ("auto", 60, 30)
            }
            Some(ms) if ms <= 80 => ("1080p", 60, 20),
            Some(ms) if ms <= 150 => {
                reasons.push(say(
                    "Độ trễ khá cao, giảm bitrate để tránh nghẽn.",
                    "Latency is fairly high, so a lower bitrate avoids congestion.",
                ));
                ("1080p", 60, 12)
            }
            Some(_) => {
                reasons.push(say(
                    "Độ trễ rất cao, ưu tiên mượt hơn nét.",
                    "Latency is very high, so smoothness comes before sharpness.",
                ));
                ("720p", 30, 8)
            }
            None => ("auto", 60, 20),
        }
    };
    let cap = resolution_cap(request);
    if resolution != "auto" && resolution_rank(resolution) > resolution_rank(cap) {
        resolution = cap;
    }
    let refresh = request
        .viewer
        .refresh_hz
        .unwrap_or(60)
        .min(request.host.refresh_hz.unwrap_or(60));
    let goal = Goal::of(request);
    if goal == Goal::Balanced && !relay && refresh >= 120 && rtt.is_some_and(|ms| ms <= 30) {
        fps = 120;
        reasons.push(say(
            "Cả hai màn hình đều 120 Hz nên dùng 120 FPS.",
            "Both screens run at 120 Hz, so 120 FPS is used.",
        ));
    }
    let fast_screens = !relay && refresh >= 120 && rtt.is_none_or(|ms| ms <= 80);
    let mut bitrate = bitrate;
    match goal {
        Goal::Balanced => {}
        Goal::Smooth => {
            fps = if fast_screens { 120 } else { fps.max(60) };
            if relay || rtt.is_some_and(|ms| ms > 30) {
                resolution = step_resolution(resolution, false, cap);
            }
            bitrate = bitrate * 4 / 5;
            reasons.insert(
                0,
                say(
                    "Ưu tiên mượt và ổn định: giữ khung hình cao, giảm độ phân giải và bitrate một chút để ít giật.",
                    "Smooth and stable first: a high frame rate, with resolution and bitrate a little lower to avoid stutters.",
                ),
            );
        }
        Goal::Sharp => {
            resolution = cap;
            if relay || rtt.is_some_and(|ms| ms > 80) {
                fps = 30;
            } else {
                fps = fps.min(60);
            }
            bitrate = bitrate * 13 / 10;
            reasons.insert(
                0,
                say(
                    "Ưu tiên nét và ổn định: độ phân giải cao nhất màn hình cho phép, khung hình vừa phải để mạng không quá tải.",
                    "Sharp and stable first: the highest resolution the screen allows, with a moderate frame rate so the network keeps up.",
                ),
            );
        }
        Goal::Responsive => {
            resolution = step_resolution(resolution, false, "1080p");
            fps = if fast_screens { 120 } else { fps.max(60) };
            bitrate = bitrate * 7 / 10;
            reasons.insert(
                0,
                say(
                    "Ưu tiên phản hồi nhanh: hình nhỏ hơn, bitrate thấp và H.264 để mỗi khung hình đến sớm nhất.",
                    "Fastest response first: smaller frames, a low bitrate and H.264 so every frame arrives as early as possible.",
                ),
            );
        }
    }
    let codec = if goal != Goal::Responsive && both_support_hevc(request) {
        reasons.push(say(
            "Cả hai máy giải mã được HEVC: hình nét hơn với cùng băng thông.",
            "Both computers handle HEVC: a sharper picture for the same bandwidth.",
        ));
        "hevc"
    } else {
        "h264"
    };
    let network_mode = if request.network.udp_works == Some(false) {
        reasons.push(say(
            "Mạng này chặn UDP nên chọn relay luôn cho nhanh vào.",
            "This network blocks UDP, so the relay is chosen up front.",
        ));
        "relay"
    } else {
        "auto"
    };
    if let Some(mbps) = measured {
        // Goals may lean either way, but never past 80% of what was measured.
        bitrate = bitrate.min(((mbps * 0.8).floor() as u32).max(2));
    }
    reasons.truncate(3);
    let settings = Settings {
        resolution: resolution.into(),
        fps,
        bitrate_mbps: bitrate,
        codec: codec.into(),
        network_mode: network_mode.into(),
    };
    (clamp(settings, request), reasons)
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["resolution", "fps", "bitrateMbps", "codec", "networkMode", "reasons"],
        "properties": {
            "resolution": {"type": "string", "enum": RESOLUTIONS},
            "fps": {"type": "integer", "enum": FRAME_RATES},
            "bitrateMbps": {"type": "integer", "minimum": 2, "maximum": 80},
            "codec": {"type": "string", "enum": CODECS},
            "networkMode": {"type": "string", "enum": NETWORK_MODES},
            "reasons": {"type": "array", "maxItems": 4, "items": {"type": "string", "maxLength": 200}}
        }
    })
}

/// Small models drift; the rules' bitrate is a measured starting point, so a
/// model answer may move it by half at most in either direction.
pub fn near_rules(mut settings: Settings, rules: &Settings) -> Settings {
    let low = (rules.bitrate_mbps / 2).max(2);
    let high = rules.bitrate_mbps.saturating_mul(3) / 2;
    settings.bitrate_mbps = settings.bitrate_mbps.clamp(low, high.max(low));
    settings
}

/// Keeps reasons a person can read: a few words, not a field name.
pub fn readable_reasons(reasons: Vec<String>) -> Vec<String> {
    reasons
        .into_iter()
        .map(|reason| reason.trim().to_owned())
        .filter(|reason| reason.split_whitespace().count() >= 3)
        .take(3)
        .collect()
}

fn prompt(request: &AdviceRequest, rules: &Settings, vietnamese: bool) -> String {
    let facts = json!({
        "viewer": request.viewer,
        "host": request.host,
        "network": request.network,
        "startingPoint": rules,
        "goal": Goal::of(request).describe(),
    });
    format!(
        "You tune Sanser, a low-latency remote desktop app. The viewer computer shows the host computer's screen.\n\
         Pick stream settings for this goal: {goal}.\n\
         Rules: on the relay or when udpWorks is false keep bitrate at most {RELAY_MAX_MBPS}; \
         use hevc only when both computers list hevc; fps above 60 only when both refreshHz are at least that; \
         resolution no larger than the viewer display; prefer networkMode auto unless UDP is blocked (then relay).\n\
         downloadMbps and uploadMbps, when present, were measured just now between the computers and this server; \
         keep bitrateMbps below 80% of the smaller one.\n\
         startingPoint is what measured rules suggest; keep it unless a fact clearly calls for a change.\n\
         Give 1 to 3 reasons in {language}, each a full sentence for a non-technical person.\n\
         Facts (JSON): {facts}",
        language = if vietnamese { "Vietnamese" } else { "English" },
        goal = Goal::of(request).describe(),
    )
}

async fn ask_model(
    config: &AdvisorConfig,
    request: &AdviceRequest,
    rules: &Settings,
    vietnamese: bool,
) -> Result<(Settings, Vec<String>), &'static str> {
    let client = reqwest::Client::builder()
        .timeout(MODEL_TIMEOUT)
        .user_agent(concat!("sanser-server/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "client")?;
    let endpoint = config
        .url
        .join("v1/chat/completions")
        .map_err(|_| "address")?;
    let body = json!({
        "model": config.model,
        "temperature": 0.2,
        "max_tokens": 400,
        "messages": [
            {"role": "system", "content": "Answer only with JSON that matches the schema."},
            {"role": "user", "content": prompt(request, rules, vietnamese)}
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "stream_settings", "strict": true, "schema": schema()}
        }
    });
    let mut call = client.post(endpoint).json(&body);
    if let Some(key) = config.api_key() {
        call = call.bearer_auth(key);
    }
    let response = call.send().await.map_err(|_| "unreachable")?;
    if !response.status().is_success() {
        return Err("status");
    }
    let payload: Value = response.json().await.map_err(|_| "body")?;
    let content = payload["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("content")?;
    parse_answer(content)
}

/// Reads the model's JSON. Anything malformed falls back to the rules.
pub fn parse_answer(content: &str) -> Result<(Settings, Vec<String>), &'static str> {
    // Small models sometimes wrap JSON in a code fence despite the schema.
    let trimmed = content.trim();
    let start = trimmed.find('{').ok_or("json")?;
    let end = trimmed.rfind('}').ok_or("json")?;
    let value: Value =
        serde_json::from_str(trimmed.get(start..=end).ok_or("json")?).map_err(|_| "json")?;
    let text = |key: &str| value[key].as_str().map(str::to_owned).ok_or("field");
    let number = |key: &str| {
        value[key]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("field")
    };
    let reasons = value["reasons"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|reason| {
                    reason
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(200)
                        .collect::<String>()
                })
                .filter(|reason| !reason.trim().is_empty())
                .take(4)
                .collect()
        })
        .unwrap_or_default();
    Ok((
        Settings {
            resolution: text("resolution")?,
            fps: number("fps")?,
            bitrate_mbps: number("bitrateMbps")?,
            codec: text("codec")?,
            network_mode: text("networkMode")?,
        },
        reasons,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(height: u32, refresh: u32, codecs: &[&str]) -> MachineProfile {
        MachineProfile {
            platform: "macos".into(),
            display_height: Some(height),
            refresh_hz: Some(refresh),
            codecs: codecs.iter().map(|codec| (*codec).to_owned()).collect(),
            ..MachineProfile::default()
        }
    }

    fn request(path: &str, rtt: u32, udp: bool) -> AdviceRequest {
        AdviceRequest {
            viewer: machine(1080, 60, &["h264", "hevc"]),
            host: machine(1440, 120, &["h264", "hevc"]),
            network: NetworkProfile {
                path: Some(path.into()),
                rtt_ms: Some(rtt),
                udp_works: Some(udp),
                server_ms: None,
                download_mbps: None,
                upload_mbps: None,
            },
            locale: None,
            goal: None,
        }
    }

    #[test]
    fn relay_keeps_the_bitrate_low_and_blocked_udp_picks_relay() {
        let (settings, reasons) = rule_settings(&request("relay", 120, false), true);
        assert!(settings.bitrate_mbps <= RELAY_MAX_MBPS);
        assert_eq!(settings.network_mode, "relay");
        assert_eq!(settings.fps, 30);
        assert!(!reasons.is_empty());
    }

    #[test]
    fn a_fast_direct_link_gets_high_quality_and_hevc() {
        let (settings, _) = rule_settings(&request("direct", 12, true), false);
        assert_eq!(settings.codec, "hevc");
        assert_eq!(settings.bitrate_mbps, 30);
        // The viewer screen is 60 Hz, so 120 FPS would be wasted.
        assert_eq!(settings.fps, 60);
        assert_eq!(settings.network_mode, "auto");
    }

    #[test]
    fn each_goal_leans_its_own_way_from_the_same_facts() {
        let pick = |goal: &str, rtt: u32| {
            let mut request = request("direct", rtt, true);
            request.goal = Some(goal.into());
            let (settings, reasons) = rule_settings(&request, true);
            assert!(reasons.len() <= 3);
            (
                settings.resolution,
                settings.fps,
                settings.bitrate_mbps,
                settings.codec,
            )
        };
        let tuple = |r: &str, f: u32, b: u32, c: &str| (r.to_owned(), f, b, c.to_owned());
        assert_eq!(pick("balanced", 200), tuple("720p", 30, 8, "hevc"));
        assert_eq!(pick("smooth", 200), tuple("720p", 60, 6, "hevc"));
        assert_eq!(pick("sharp", 200), tuple("1080p", 30, 10, "hevc"));
        assert_eq!(pick("responsive", 200), tuple("720p", 60, 5, "h264"));
        assert_eq!(pick("smooth", 20), tuple("auto", 60, 24, "hevc"));
        assert_eq!(pick("sharp", 20), tuple("1080p", 60, 39, "hevc"));
        assert_eq!(pick("responsive", 20), tuple("720p", 60, 21, "h264"));
    }

    #[test]
    fn measured_bandwidth_sets_the_bitrate_instead_of_latency() {
        let plan = |down: f64, up: f64, goal: &str| {
            let mut request = request("direct", 200, true);
            request.network.download_mbps = Some(down);
            request.network.upload_mbps = Some(up);
            request.goal = Some(goal.into());
            let (settings, reasons) = rule_settings(&request, true);
            let measured = down.is_finite() && down > 0.0;
            assert_eq!(
                reasons.iter().any(|reason| reason.contains("Mb/s")),
                measured,
                "{reasons:?}"
            );
            (settings.resolution, settings.fps, settings.bitrate_mbps)
        };
        let tuple = |r: &str, f: u32, b: u32| (r.to_owned(), f, b);
        // The slower side decides: 12 Mb/s upload → 7 Mb/s budget.
        assert_eq!(plan(300.0, 12.0, "balanced"), tuple("720p", 60, 7));
        assert_eq!(plan(25.0, 90.0, "balanced"), tuple("1080p", 60, 15));
        // The viewer screen is 1080p, so 1440p is capped.
        assert_eq!(plan(40.0, 40.0, "balanced"), tuple("1080p", 60, 24));
        assert_eq!(plan(200.0, 200.0, "balanced"), tuple("auto", 60, 80));
        // Sharp leans up but stays under 80% of what was measured.
        assert_eq!(plan(25.0, 25.0, "sharp"), tuple("1080p", 30, 19));
        // Nothing usable measured: back to the latency rules.
        assert_eq!(plan(f64::NAN, 0.0, "balanced"), tuple("720p", 30, 8));
    }

    #[test]
    fn model_answers_are_clamped_to_what_the_computers_can_do() {
        let mut request = request("relay", 40, true);
        request.host.codecs = vec!["h264".into()];
        let wild = Settings {
            resolution: "2160p".into(),
            fps: 120,
            bitrate_mbps: 500,
            codec: "hevc".into(),
            network_mode: "direct".into(),
        };
        let settings = clamp(wild, &request);
        assert_eq!(settings.resolution, "1080p");
        assert_eq!(settings.fps, 60);
        assert_eq!(settings.bitrate_mbps, RELAY_MAX_MBPS);
        assert_eq!(settings.codec, "auto");
        let unknown = Settings {
            resolution: "8k".into(),
            fps: 45,
            bitrate_mbps: 0,
            codec: "av1".into(),
            network_mode: "tailscale".into(),
        };
        let settings = clamp(unknown, &request);
        assert_eq!(
            (
                settings.resolution.as_str(),
                settings.fps,
                settings.codec.as_str(),
                settings.network_mode.as_str()
            ),
            ("auto", 60, "auto", "auto")
        );
        assert_eq!(settings.bitrate_mbps, 2);
    }

    #[test]
    fn model_bitrate_stays_near_the_rules_and_junk_reasons_are_dropped() {
        let rules = Settings {
            resolution: "auto".into(),
            fps: 60,
            bitrate_mbps: 30,
            codec: "hevc".into(),
            network_mode: "auto".into(),
        };
        let low = Settings {
            bitrate_mbps: 3,
            ..rules.clone()
        };
        assert_eq!(near_rules(low, &rules).bitrate_mbps, 15);
        let high = Settings {
            bitrate_mbps: 80,
            ..rules.clone()
        };
        assert_eq!(near_rules(high, &rules).bitrate_mbps, 45);
        assert_eq!(
            readable_reasons(vec![
                "memoryGb".into(),
                "path".into(),
                "Mạng nhanh nên dùng chất lượng cao.".into()
            ]),
            ["Mạng nhanh nên dùng chất lượng cao."]
        );
    }

    #[test]
    fn model_json_is_read_even_inside_a_code_fence() {
        let content = "```json\n{\"resolution\":\"1080p\",\"fps\":60,\"bitrateMbps\":18,\"codec\":\"h264\",\"networkMode\":\"auto\",\"reasons\":[\"Mạng ổn định\",\"\"]}\n```";
        let (settings, reasons) = parse_answer(content).expect("parsed");
        assert_eq!(settings.bitrate_mbps, 18);
        assert_eq!(reasons, ["Mạng ổn định"]);
        assert!(parse_answer("I think 1080p is good").is_err());
        assert!(parse_answer("{\"resolution\":\"1080p\"}").is_err());
    }
}
