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
    let (mut resolution, mut fps, bitrate) = if relay {
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
    if !relay && refresh >= 120 && rtt.is_some_and(|ms| ms <= 30) {
        fps = 120;
        reasons.push(say(
            "Cả hai màn hình đều 120 Hz nên dùng 120 FPS.",
            "Both screens run at 120 Hz, so 120 FPS is used.",
        ));
    }
    let codec = if both_support_hevc(request) {
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
    });
    format!(
        "You tune Sanser, a low-latency remote desktop app. The viewer computer shows the host computer's screen.\n\
         Pick stream settings that feel smooth first and sharp second.\n\
         Rules: on the relay or when udpWorks is false keep bitrate at most {RELAY_MAX_MBPS}; \
         use hevc only when both computers list hevc; fps above 60 only when both refreshHz are at least that; \
         resolution no larger than the viewer display; prefer networkMode auto unless UDP is blocked (then relay).\n\
         startingPoint is what measured rules suggest; keep it unless a fact clearly calls for a change.\n\
         Give 1 to 3 reasons in {language}, each a full sentence for a non-technical person.\n\
         Facts (JSON): {facts}",
        language = if vietnamese { "Vietnamese" } else { "English" },
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
            },
            locale: None,
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
