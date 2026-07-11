use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
};

use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    error::DesktopError,
    models::{EngineKind, EngineStatus, LaunchEngineRequest, NetworkMode, VideoCodec},
};

const HOST_SIDECAR: &str = "sanser-host-windows";
const CLIENT_SIDECAR: &str = "sanser-client-macos";
const INHERITED_ENVIRONMENT: [&str; 7] = [
    "SystemRoot",
    "WINDIR",
    "PATH",
    "HOME",
    "TMP",
    "TEMP",
    "USERPROFILE",
];

#[derive(Default)]
struct EngineState {
    children: HashMap<EngineKind, Child>,
    last_errors: HashMap<EngineKind, String>,
}

#[derive(Default)]
pub struct EngineManager {
    state: Mutex<EngineState>,
}

impl Drop for EngineManager {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut() {
            for child in state.children.values_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn sidecar_name(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Host => HOST_SIDECAR,
        EngineKind::Client => CLIENT_SIDECAR,
        // Kept for backward-compatible command serialization only. Local
        // server/database mode is deliberately not bundled in Sanser 2.
        EngineKind::LocalServer => "sanser-server-disabled",
    }
}

fn supported_on_platform(kind: EngineKind) -> bool {
    match kind {
        EngineKind::Host => cfg!(target_os = "windows"),
        EngineKind::Client => cfg!(target_os = "macos"),
        EngineKind::LocalServer => false,
    }
}

fn executable_filename(base: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

#[cfg(debug_assertions)]
#[allow(unused_variables)]
fn staged_sidecar_filename(base: &str) -> Option<String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some(format!("{base}-aarch64-apple-darwin"));
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Some(format!("{base}-x86_64-apple-darwin"));
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Some(format!("{base}-x86_64-pc-windows-msvc.exe"));
    #[allow(unreachable_code)]
    None
}

fn candidate_paths(app: &AppHandle, kind: EngineKind) -> Vec<PathBuf> {
    let filename = executable_filename(sidecar_name(kind));
    let mut candidates = Vec::with_capacity(5);
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join(&filename));
        candidates.push(resource_dir.join("binaries").join(&filename));
    }
    if let Ok(current) = std::env::current_exe()
        && let Some(directory) = current.parent()
    {
        candidates.push(directory.join(&filename));
        candidates.push(directory.join("binaries").join(&filename));
    }
    #[cfg(debug_assertions)]
    if let Some(staged) = staged_sidecar_filename(sidecar_name(kind)) {
        // Tauri strips the target triple when bundling. During `tauri dev`,
        // however, the CLI leaves the staged binary beside this manifest.
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries")
                .join(staged),
        );
    }
    candidates
}

pub fn find_sidecar(app: &AppHandle, kind: EngineKind) -> Option<PathBuf> {
    if !supported_on_platform(kind) {
        return None;
    }
    candidate_paths(app, kind)
        .into_iter()
        .find(|path| path.is_file())
}

fn endpoint(address: IpAddr, port: u16) -> String {
    match address {
        IpAddr::V4(value) => format!("{value}:{port}"),
        IpAddr::V6(value) => format!("[{value}]:{port}"),
    }
}

fn validated_session(request: &LaunchEngineRequest) -> Result<&str, DesktopError> {
    let session = request
        .session_id
        .as_deref()
        .ok_or_else(|| DesktopError::InvalidRequest("session ID is required".into()))?;
    Uuid::parse_str(session)
        .map_err(|_| DesktopError::InvalidRequest("session ID must be a UUID".into()))?;
    Ok(session)
}

fn validated_session_token(request: &LaunchEngineRequest) -> Result<&str, DesktopError> {
    let token = request.session_token.as_deref().ok_or_else(|| {
        DesktopError::InvalidRequest("authorized native session credential is required".into())
    })?;
    if !(32..=512).contains(&token.len())
        || !token.is_ascii()
        || token.chars().any(char::is_whitespace)
    {
        return Err(DesktopError::InvalidRequest(
            "native session credential has an invalid format".into(),
        ));
    }
    Ok(token)
}

fn validate_common(request: &LaunchEngineRequest) -> Result<(), DesktopError> {
    if !matches!(request.fps, 30 | 60 | 90 | 120) {
        return Err(DesktopError::InvalidRequest(
            "FPS must be 30, 60, 90 or 120".into(),
        ));
    }
    if !(500..=200_000).contains(&request.bitrate_kbps) {
        return Err(DesktopError::InvalidRequest(
            "bitrate must be between 500 and 200000 Kbps".into(),
        ));
    }
    if !(640..=7680).contains(&request.width)
        || !(360..=4320).contains(&request.height)
        || request.width % 2 != 0
        || request.height % 2 != 0
    {
        return Err(DesktopError::InvalidRequest(
            "resolution is outside supported even dimensions".into(),
        ));
    }
    if request.network_mode == NetworkMode::Relay && request.kind != EngineKind::LocalServer {
        return Err(DesktopError::InvalidRequest(
            "SNV2 cannot be launched in Relay mode; use WebRTC TURN".into(),
        ));
    }
    Ok(())
}

fn native_codec(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::Auto | VideoCodec::H264 => "h264",
        VideoCodec::Hevc => "hevc",
    }
}

fn build_args(request: &LaunchEngineRequest) -> Result<Vec<String>, DesktopError> {
    validate_common(request)?;
    if request.kind == EngineKind::LocalServer {
        return Err(DesktopError::Unavailable(
            "local server mode is disabled; use the deployed PostgreSQL/Neon API".into(),
        ));
    }

    let _session = validated_session(request)?;
    let _token = validated_session_token(request)?;
    let port = request
        .port
        .filter(|value| (1..=65_533).contains(value))
        .ok_or_else(|| {
            DesktopError::InvalidRequest("base port must be between 1 and 65533".into())
        })?;

    match request.kind {
        EngineKind::Host => {
            let address = request
                .address
                .as_deref()
                .ok_or_else(|| {
                    DesktopError::InvalidRequest("host target address is required".into())
                })?
                .parse::<IpAddr>()
                .map_err(|_| {
                    DesktopError::InvalidRequest("host target address must be an IP".into())
                })?;
            let mut args = vec![
                "--encode-pipe".into(),
                native_codec(request.codec).into(),
                "--fps".into(),
                request.fps.to_string(),
                "--bitrate".into(),
                request
                    .bitrate_kbps
                    .checked_mul(1_000)
                    .ok_or_else(|| DesktopError::InvalidRequest("bitrate overflow".into()))?
                    .to_string(),
                "--stream-width".into(),
                request.width.to_string(),
                "--stream-height".into(),
                request.height.to_string(),
                "--udp-connect".into(),
                endpoint(address, port),
                "--low-latency-encoder".into(),
                "--udp-pacing".into(),
            ];
            if request.input_enabled {
                args.extend(["--control-connect".into(), endpoint(address, port + 1)]);
            }
            if request.audio_enabled {
                args.extend(["--audio-udp-connect".into(), endpoint(address, port + 2)]);
            }
            Ok(args)
        }
        EngineKind::Client => {
            let mut args = vec![
                "--listen-render-snv".into(),
                port.to_string(),
                "--control-port".into(),
                (port + 1).to_string(),
                "--audio-port".into(),
                if request.audio_enabled {
                    (port + 2).to_string()
                } else {
                    "0".into()
                },
                "--udp-video".into(),
            ];
            if request.relative_mouse {
                args.push("--relative-mouse".into());
            }
            Ok(args)
        }
        EngineKind::LocalServer => unreachable!("local server requests are rejected above"),
    }
}

fn sanitized_command(path: &Path, args: &[String], request: &LaunchEngineRequest) -> Command {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if request.kind != EngineKind::LocalServer {
        let inherited: Vec<(String, String)> = INHERITED_ENVIRONMENT
            .iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| ((*key).to_owned(), value))
            })
            .collect();
        command.env_clear();
        command.envs(inherited);
        if let Some(token) = request.session_token.as_deref() {
            command.env("SANSER_NATIVE_SESSION_TOKEN", token);
        }
    }
    command
}

impl EngineManager {
    pub fn launch(
        &self,
        app: &AppHandle,
        request: &LaunchEngineRequest,
    ) -> Result<(), DesktopError> {
        if request.kind == EngineKind::LocalServer {
            return Err(DesktopError::Unavailable(
                "local server mode is disabled; use the deployed PostgreSQL/Neon API".into(),
            ));
        }
        let path = find_sidecar(app, request.kind).ok_or_else(|| {
            DesktopError::Unavailable(format!("{} is not bundled", sidecar_name(request.kind)))
        })?;
        let args = build_args(request)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;

        if let Some(child) = state.children.get_mut(&request.kind) {
            match child.try_wait() {
                Ok(None) => {
                    return Err(DesktopError::Process(
                        "the requested native engine is already running".into(),
                    ));
                }
                Ok(Some(_)) | Err(_) => {
                    state.children.remove(&request.kind);
                }
            }
        }

        let child = sanitized_command(&path, &args, request)
            .spawn()
            .map_err(|error| DesktopError::Process(error.to_string()))?;
        state.last_errors.remove(&request.kind);
        state.children.insert(request.kind, child);
        Ok(())
    }

    pub fn stop(&self, kind: EngineKind) -> Result<(), DesktopError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;
        let Some(mut child) = state.children.remove(&kind) else {
            return Ok(());
        };
        child
            .kill()
            .map_err(|error| DesktopError::Process(error.to_string()))?;
        child
            .wait()
            .map_err(|error| DesktopError::Process(error.to_string()))?;
        Ok(())
    }

    pub fn status(&self, app: &AppHandle, kind: EngineKind) -> Result<EngineStatus, DesktopError> {
        let installed = find_sidecar(app, kind).is_some();
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;
        let mut running = false;
        let mut process_id = None;
        let mut exited = false;
        if let Some(child) = state.children.get_mut(&kind) {
            match child.try_wait() {
                Ok(None) => {
                    running = true;
                    process_id = Some(child.id());
                }
                Ok(Some(status)) => {
                    state
                        .last_errors
                        .insert(kind, format!("process exited with {status}"));
                    exited = true;
                }
                Err(error) => {
                    state.last_errors.insert(kind, error.to_string());
                    exited = true;
                }
            }
        }
        if exited {
            state.children.remove(&kind);
        }
        Ok(EngineStatus {
            kind,
            installed,
            running,
            process_id,
            last_error: state.last_errors.get(&kind).cloned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use super::*;

    fn request(kind: EngineKind) -> LaunchEngineRequest {
        LaunchEngineRequest {
            kind,
            session_id: Some("018f4d89-5e8b-7a80-bd1e-cb7cb9f43189".into()),
            address: Some("2001:db8::1".into()),
            port: Some(50_000),
            codec: VideoCodec::Auto,
            fps: 60,
            bitrate_kbps: 25_000,
            width: 1920,
            height: 1080,
            network_mode: NetworkMode::Direct,
            audio_enabled: true,
            input_enabled: true,
            relative_mouse: false,
            session_token: Some("a".repeat(32)),
        }
    }

    #[test]
    fn formats_ipv6_endpoints_without_ambiguity() {
        let address = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert_eq!(endpoint(address, 5000), "[2001:db8::1]:5000");
    }

    #[test]
    fn host_auto_codec_uses_compatible_h264_and_bounded_ports() -> Result<(), DesktopError> {
        let args = build_args(&request(EngineKind::Host))?;
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--encode-pipe", "h264"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--control-connect", "[2001:db8::1]:50001"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--audio-udp-connect", "[2001:db8::1]:50002"])
        );
        Ok(())
    }

    #[test]
    fn rejects_zero_base_port() {
        let mut invalid = request(EngineKind::Client);
        invalid.port = Some(0);
        assert!(matches!(
            build_args(&invalid),
            Err(DesktopError::InvalidRequest(_))
        ));
    }

    #[test]
    fn rejects_removed_local_server_mode() {
        assert!(matches!(
            build_args(&request(EngineKind::LocalServer)),
            Err(DesktopError::Unavailable(_))
        ));
    }
}
