use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{
    engine_output::EngineOutput,
    error::DesktopError,
    models::{EngineKind, EngineStatus, LaunchEngineRequest, VideoCodec},
};

#[cfg(target_os = "macos")]
const HOST_SIDECAR: &str = "sanser-host-macos";
#[cfg(not(target_os = "macos"))]
const HOST_SIDECAR: &str = "sanser-host-windows";
#[cfg(target_os = "windows")]
const CLIENT_SIDECAR: &str = "sanser-client-windows";
#[cfg(not(target_os = "windows"))]
const CLIENT_SIDECAR: &str = "sanser-client-macos";
const SIDECAR_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const ENGINE_STARTUP_STABILIZATION: Duration = Duration::from_millis(250);
const ENGINE_STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(25);
const INHERITED_ENVIRONMENT: [&str; 7] = [
    "SystemRoot",
    "WINDIR",
    "PATH",
    "HOME",
    "TMP",
    "TEMP",
    "USERPROFILE",
];

struct EngineChild {
    session_id: Option<Uuid>,
    process: Child,
    output: EngineOutput,
    acknowledgements: std::sync::mpsc::Receiver<crate::engine_output::SharingAck>,
    sharing: Option<crate::sharing_engine::SharingHost>,
    first_frame: Arc<std::sync::atomic::AtomicBool>,
    sharing_statuses: crate::engine_output::SharingStatuses,
    direct_bridges: HashMap<Uuid, crate::direct::Bridge>,
}

impl EngineChild {
    fn spawn(
        command: &mut Command,
        debug_log_path: Option<&Path>,
        metrics: Option<std::sync::mpsc::SyncSender<crate::connection_report::Sample>>,
    ) -> std::io::Result<Self> {
        let mut process = command.spawn()?;
        let Some(stderr) = process.stderr.take() else {
            let _ = process.kill();
            let _ = process.wait();
            return Err(std::io::Error::other("native stderr pipe is missing"));
        };
        let first_frame = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sharing_statuses = Arc::new(Mutex::new(HashMap::new()));
        let (acknowledgements, ack_receiver) = std::sync::mpsc::sync_channel(16);
        if let Some(stdout) = process.stdout.take()
            && let Err(error) = EngineOutput::start_observed(
                stdout,
                None,
                metrics.clone(),
                Some(acknowledgements),
                Some(Arc::clone(&first_frame)),
                Some(Arc::clone(&sharing_statuses)),
            )
        {
            let _ = process.kill();
            let _ = process.wait();
            return Err(error);
        }
        match EngineOutput::start_recording(
            stderr,
            debug_log_path.and_then(open_private_log),
            metrics,
        ) {
            Ok(output) => Ok(Self {
                session_id: None,
                process,
                output,
                acknowledgements: ack_receiver,
                sharing: None,
                first_frame,
                sharing_statuses,
                direct_bridges: HashMap::new(),
            }),
            Err(error) => {
                let _ = process.kill();
                let _ = process.wait();
                Err(error)
            }
        }
    }

    fn sharing_command(&mut self, request: Uuid, command: &[u8]) -> Result<(), DesktopError> {
        let input = self.process.stdin.as_mut().ok_or_else(|| {
            DesktopError::Process("native sharing control pipe is unavailable".into())
        })?;
        if input
            .write_all(command)
            .and_then(|()| input.flush())
            .is_err()
        {
            if let Some(sharing) = self.sharing.as_mut() {
                sharing.desynchronized = true;
            }
            return Err(DesktopError::Process(
                "unable to send native sharing command".into(),
            ));
        }
        self.wait_sharing_ack(request, Duration::from_secs(3))
    }
    fn wait_sharing_ack(&mut self, request: Uuid, timeout: Duration) -> Result<(), DesktopError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.acknowledgements.recv_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(ENGINE_STARTUP_POLL_INTERVAL),
            ) {
                Ok(ack) if ack.request == request => {
                    return if ack.accepted {
                        Ok(())
                    } else {
                        Err(DesktopError::Process(
                            "native host rejected this sharing peer".into(),
                        ))
                    };
                }
                Ok(_) => {}
                Err(error) => {
                    // Permission/capture failure can occur after the initial
                    // stabilization period. Preserve stderr instead of hiding
                    // the host's failure behind an acknowledgement timeout.
                    if let Some(status) = self.process.try_wait().map_err(|error| {
                        DesktopError::Process(format!("unable to inspect native host: {error}"))
                    })? {
                        return Err(DesktopError::Process(self.output.exit_message(format!(
                            "native host exited before sharing acknowledgement with {status}"
                        ))));
                    }
                    if Instant::now() < deadline {
                        if matches!(error, std::sync::mpsc::RecvTimeoutError::Disconnected) {
                            thread::sleep(ENGINE_STARTUP_POLL_INTERVAL);
                        }
                        continue;
                    }
                    if let Some(sharing) = self.sharing.as_mut() {
                        sharing.desynchronized = true;
                    }
                    return Err(DesktopError::Process("native sharing acknowledgement timed out; existing viewers remain active, stop and restart sharing before changing membership".into()));
                }
            }
        }
    }
}

#[derive(Default)]
struct EngineState {
    children: HashMap<EngineKind, EngineChild>,
    last_errors: HashMap<EngineKind, String>,
    closed_viewers: HashMap<EngineKind, Uuid>,
    last_sessions: HashMap<EngineKind, Uuid>,
}

#[derive(Default)]
pub struct EngineManager {
    state: Mutex<EngineState>,
}

impl Drop for EngineManager {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut() {
            for child in state.children.values_mut() {
                let _ = stop_child(child);
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

fn sidecar_engine_id(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Host => HOST_SIDECAR.strip_prefix("sanser-").unwrap_or(HOST_SIDECAR),
        EngineKind::Client => CLIENT_SIDECAR
            .strip_prefix("sanser-")
            .unwrap_or(CLIENT_SIDECAR),
        EngineKind::LocalServer => "server-disabled",
    }
}

fn supported_on_platform(kind: EngineKind) -> bool {
    match kind {
        EngineKind::Host | EngineKind::Client => {
            cfg!(any(target_os = "windows", target_os = "macos"))
        }
        EngineKind::LocalServer => false,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
struct SidecarCapabilities {
    product: String,
    version: String,
    protocol_version: u8,
    engine: String,
    native_snv2: bool,
    native_direct: bool,
    #[serde(default)]
    h264_encoder_implementation: bool,
    #[serde(default)]
    h264_decoder_implementation: bool,
    #[serde(default)]
    hevc_encoder_implementation: bool,
    #[serde(default)]
    hevc_decoder_implementation: bool,
    #[serde(default)]
    cross_platform: bool,
    #[serde(default)]
    multi_viewer_host: bool,
    #[serde(default)]
    snv2_audio_implementation: bool,
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct SidecarProbe {
    pub compatible: bool,
    pub native_snv2: bool,
    pub native_direct: bool,
    pub hevc: bool,
    pub cross_platform: bool,
    pub multi_viewer_host: bool,
    pub snv2_audio: bool,
    pub reason: Option<String>,
}

impl SidecarProbe {
    fn incompatible(reason: impl Into<String>) -> Self {
        Self {
            compatible: false,
            native_snv2: false,
            native_direct: false,
            hevc: false,
            cross_platform: false,
            multi_viewer_host: false,
            snv2_audio: false,
            reason: Some(reason.into()),
        }
    }
}

fn parse_sidecar_capabilities(
    bytes: &[u8],
    kind: EngineKind,
) -> Result<SidecarCapabilities, String> {
    let capabilities = serde_json::from_slice::<SidecarCapabilities>(bytes)
        .map_err(|_| "native sidecar returned an invalid capability document".to_owned())?;
    let expected_engine = sidecar_engine_id(kind);
    if capabilities.product != "Sanser"
        || capabilities.version != crate::models::SANSER_VERSION
        || capabilities.protocol_version != crate::models::PROTOCOL_VERSION
        || capabilities.engine != expected_engine
    {
        return Err("native sidecar identity or protocol version does not match this app".into());
    }
    let media_implementation = match kind {
        EngineKind::Host => capabilities.h264_encoder_implementation,
        EngineKind::Client => capabilities.h264_decoder_implementation,
        EngineKind::LocalServer => false,
    };
    if !media_implementation {
        return Err("native sidecar does not provide the required H.264 media engine".into());
    }
    Ok(capabilities)
}

fn native_command(path: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Output is piped to Sanser; do not allocate a blank terminal.
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn probe_path(path: &Path, kind: EngineKind) -> SidecarProbe {
    let mut command = native_command(path);
    command
        .arg("--capabilities-json")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in INHERITED_ENVIRONMENT {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }

    let Ok(mut child) = command.spawn() else {
        return SidecarProbe::incompatible("native sidecar could not be started");
    };
    let deadline = Instant::now() + SIDECAR_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return SidecarProbe::incompatible("native sidecar capability probe failed");
                }
                break;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return SidecarProbe::incompatible("native sidecar capability probe timed out");
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return SidecarProbe::incompatible("native sidecar capability probe failed");
            }
        }
    }

    let Ok(output) = child.wait_with_output() else {
        return SidecarProbe::incompatible("native sidecar capability output is unavailable");
    };
    match parse_sidecar_capabilities(&output.stdout, kind) {
        Ok(capabilities) => {
            let hevc = match kind {
                EngineKind::Host => capabilities.hevc_encoder_implementation,
                EngineKind::Client => capabilities.hevc_decoder_implementation,
                EngineKind::LocalServer => false,
            };
            SidecarProbe {
                compatible: true,
                native_snv2: capabilities.native_snv2,
                native_direct: capabilities.native_direct,
                hevc,
                cross_platform: capabilities.cross_platform,
                multi_viewer_host: capabilities.multi_viewer_host,
                snv2_audio: capabilities.snv2_audio_implementation,
                reason: (!capabilities.native_direct).then(|| {
                    "Native sidecar does not advertise the authenticated direct transport"
                        .to_owned()
                }),
            }
        }
        Err(reason) => SidecarProbe::incompatible(reason),
    }
}

pub fn probe_sidecar(app: &AppHandle, kind: EngineKind) -> SidecarProbe {
    find_sidecar(app, kind).map_or_else(
        || SidecarProbe::incompatible(format!("{} is not bundled", sidecar_name(kind))),
        |path| probe_path(&path, kind),
    )
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
        || !request.width.is_multiple_of(2)
        || !request.height.is_multiple_of(2)
    {
        return Err(DesktopError::InvalidRequest(
            "resolution is outside supported even dimensions".into(),
        ));
    }
    // Relay mode validation removed as TURN is deprecated.
    Ok(())
}

fn native_codec(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::Auto | VideoCodec::H264 => "h264",
        VideoCodec::Hevc => "hevc",
    }
}

fn validate_wire_protocol(request: &LaunchEngineRequest) -> Result<bool, DesktopError> {
    if let Some(wire) = request.wire_protocol.as_deref()
        && !matches!(wire, "legacy" | "snv2")
    {
        return Err(DesktopError::InvalidRequest(
            "Unknown native wire protocol".into(),
        ));
    }
    let cross_platform = request.wire_protocol.as_deref() == Some("snv2");
    if cross_platform && (request.codec == VideoCodec::Hevc || request.relative_mouse) {
        return Err(DesktopError::Unavailable(
            "Cross-platform sessions require H.264/Auto and absolute mouse mode".into(),
        ));
    }
    Ok(cross_platform)
}

fn host_endpoint(request: &LaunchEngineRequest, port: u16) -> Result<String, DesktopError> {
    let address = request
        .address
        .as_deref()
        .ok_or_else(|| DesktopError::InvalidRequest("host target address is required".into()))?
        .parse::<IpAddr>()
        .map_err(|_| DesktopError::InvalidRequest("host target address must be an IP".into()))?;
    if !address.is_ipv4() {
        return Err(DesktopError::InvalidRequest(
            "authenticated native direct transport currently requires an IPv4 route".into(),
        ));
    }
    Ok(endpoint(address, port))
}

fn build_args(request: &LaunchEngineRequest) -> Result<Vec<String>, DesktopError> {
    let cross_platform = validate_wire_protocol(request)?;
    validate_common(request)?;
    let sharing = crate::sharing_engine::SharingPeer::from_request(request)?;
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
                host_endpoint(request, port)?,
                "--low-latency-encoder".into(),
                "--udp-pacing".into(),
            ];
            if !request.input_enabled {
                args.push("--disable-input".into());
            }
            if let Some(bind_port) = request.udp_bind_port {
                args.push("--udp-bind-port".into());
                args.push(bind_port.to_string());
            }
            if cross_platform {
                args.push("--snv2".into());
                if !request.audio_enabled {
                    args.push("--disable-audio".into());
                }
            } else {
                // Legacy Windows capture otherwise defaults to a 250 ms
                // screenshot interval, overriding the requested stream FPS.
                args.extend(["--interval-ms", "0", "--no-stream-cursor"].map(str::to_owned));
            }
            if let Some(sharing) = sharing {
                args.extend([
                    "--share-host".into(),
                    "--share-peer-id".into(),
                    sharing.viewer.to_string(),
                ]);
            }
            if request.ultra_low_latency {
                args.push("--ultra-low-latency".into());
            }
            Ok(args)
        }
        EngineKind::Client => {
            let mut args = vec![
                "--listen-render-snv".into(),
                port.to_string(),
                "--control-port".into(),
                "0".into(),
                "--audio-port".into(),
                "0".into(),
                "--udp-video".into(),
            ];
            if !request.input_enabled {
                args.push("--disable-input".into());
            }
            if request.relative_mouse {
                args.push("--relative-mouse".into());
            }
            if let Some(ref conn) = request.udp_connect {
                args.push("--udp-connect".into());
                args.push(conn.clone());
            }
            if cross_platform {
                args.push("--snv2".into());
                if !request.audio_enabled {
                    args.push("--disable-audio".into());
                }
            }
            if request.ultra_low_latency {
                args.push("--ultra-low-latency".into());
            }
            Ok(args)
        }
        EngineKind::LocalServer => unreachable!("local server requests are rejected above"),
    }
}

fn sidecar_debug_log_path(app: &AppHandle, kind: EngineKind) -> Option<PathBuf> {
    let enabled = std::env::var("SANSER_SIDECAR_DEBUG_LOG").is_ok_and(|val| val == "true");
    if !enabled {
        return None;
    }
    let directory = app.path().app_log_dir().ok()?;
    fs::create_dir_all(&directory).ok()?;
    Some(directory.join(format!("{}-debug.log", sidecar_engine_id(kind))))
}

fn open_private_log(path: &Path) -> Option<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .ok()?;
    }
    Some(file)
}

fn sanitized_command(path: &Path, args: &[String], request: &LaunchEngineRequest) -> Command {
    let mut command = native_command(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

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
        command.env("SANSER_TELEMETRY", "1");
        if let Some(token) = request.session_token.as_deref() {
            command.env("SANSER_NATIVE_SESSION_TOKEN", token);
        }
    }
    if request.wire_protocol.as_deref() == Some("snv2") {
        command
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .env("SANSER_CONTROL_STDIN", "1");
    }
    command
}

fn prepare_direct(
    reserved: Option<crate::direct::DirectTransport>,
    request: &mut LaunchEngineRequest,
) -> Result<Option<crate::direct::Prepared>, DesktopError> {
    match reserved {
        Some(transport) if transport.monitor.is_some() => {
            crate::direct::Prepared::new(transport, request).map(Some)
        }
        _ => Ok(None),
    }
}

fn stop_child(engine: &mut EngineChild) -> std::io::Result<()> {
    let child = &mut engine.process;
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(b"stop\n");
        drop(input);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if child.try_wait()?.is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    child.kill()?;
    child.wait()?;
    Ok(())
}

fn stabilize_child(child: &mut EngineChild) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match child.process.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => {
                return Err(child
                    .output
                    .exit_message(format!("process exited during startup with {status}")));
            }
            Err(error) => {
                return Err(format!("could not inspect process during startup: {error}"));
            }
        }

        let elapsed = started.elapsed();
        if elapsed >= ENGINE_STARTUP_STABILIZATION {
            return Ok(());
        }
        let remaining = ENGINE_STARTUP_STABILIZATION
            .checked_sub(elapsed)
            .unwrap_or_default();
        thread::sleep(ENGINE_STARTUP_POLL_INTERVAL.min(remaining));
    }
}

impl EngineManager {
    pub fn sharing_peer_status(
        &self,
        group: Uuid,
    ) -> Result<Vec<crate::models::SharingPeerStatus>, DesktopError> {
        let state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;
        let Some(child) = state.children.get(&EngineKind::Host) else {
            return Ok(Vec::new());
        };
        let Some(host) = child.sharing.as_ref() else {
            return Ok(Vec::new());
        };
        let statuses = child
            .sharing_statuses
            .lock()
            .map_err(|_| DesktopError::Process("sharing status is unavailable".into()))?;
        Ok(statuses
            .values()
            .filter(|(status, observed)| {
                observed.elapsed() < Duration::from_secs(5)
                    && host.has_viewer(group, status.viewer_id)
            })
            .map(|(status, _)| status.clone())
            .collect())
    }
    pub fn is_sharing(&self, kind: EngineKind) -> bool {
        self.state.lock().is_ok_and(|state| {
            state
                .children
                .get(&kind)
                .is_some_and(|child| child.sharing.is_some())
        })
    }
    // Validate/probe, reserve ports and stabilize the managed process in one lifecycle operation.
    #[allow(clippy::too_many_lines)]
    pub fn launch(
        &self,
        app: &AppHandle,
        request: &LaunchEngineRequest,
        reserved_socket: Option<crate::direct::DirectTransport>,
    ) -> Result<(), DesktopError> {
        if request.kind == EngineKind::LocalServer {
            return Err(DesktopError::Unavailable(
                "local server mode is disabled; use the deployed PostgreSQL/Neon API".into(),
            ));
        }
        let path = find_sidecar(app, request.kind).ok_or_else(|| {
            DesktopError::Unavailable(format!("{} is not bundled", sidecar_name(request.kind)))
        })?;
        let probe = probe_path(&path, request.kind);
        let sharing = crate::sharing_engine::SharingPeer::from_request(request)?;
        if !probe.compatible {
            return Err(DesktopError::Unavailable(probe.reason.unwrap_or_else(
                || "native sidecar capability probe failed".into(),
            )));
        }
        if !probe.native_direct {
            return Err(DesktopError::Unavailable(
                "native sidecar does not advertise the authenticated direct transport".into(),
            ));
        }
        if sharing.is_some() && !probe.multi_viewer_host {
            return Err(DesktopError::Unavailable(
                "installed native host does not support multi-viewer sharing".into(),
            ));
        }
        if request.codec == VideoCodec::Hevc && !probe.hevc {
            return Err(DesktopError::Unavailable(
                "native sidecar does not provide the requested HEVC engine".into(),
            ));
        }
        let requires_snv2 = (cfg!(target_os = "macos") && request.kind == EngineKind::Host)
            || (cfg!(target_os = "windows") && request.kind == EngineKind::Client);
        if requires_snv2 && request.wire_protocol.as_deref() != Some("snv2") {
            return Err(DesktopError::Unavailable(
                "This direction requires the updated server and SNV2 session negotiation".into(),
            ));
        }
        if request.wire_protocol.as_deref() == Some("snv2") && !probe.cross_platform {
            return Err(DesktopError::Unavailable(
                "The installed native engine does not support cross-platform sessions".into(),
            ));
        }
        let mut native_request = request.clone();
        let prepared = prepare_direct(reserved_socket, &mut native_request)?;
        let args = build_args(&native_request)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;

        if let Some(child) = state.children.get_mut(&request.kind) {
            match child.process.try_wait() {
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

        let debug_log_path = sidecar_debug_log_path(app, request.kind);
        let report =
            app.path().app_log_dir().ok().and_then(
                |directory| match crate::connection_report::start(
                    &directory.join("connections"),
                    request,
                    Some(Box::new({
                        let app = app.clone();
                        move |health| {
                            let _ = app.emit("direct-route-health", health);
                        }
                    })),
                    Some(Box::new({
                        let app = app.clone();
                        move |performance| {
                            let _ = app.emit("native-performance", performance);
                        }
                    })),
                ) {
                    Ok(report) => Some(report),
                    Err(error) => {
                        eprintln!("Unable to save automatic connection report: {error}");
                        None
                    }
                },
            );
        if request.relay
            && let Some(session) = request
                .session_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok())
        {
            app.state::<crate::relay::RelayManager>().attach_report(
                request.kind,
                session,
                report.clone(),
            );
        }
        let mut command = sanitized_command(&path, &args, &native_request);
        let direct_bridge = prepared.map(|prepared| {
            prepared.start(
                app.clone(),
                request.kind,
                request.direct_attempt_id.clone().unwrap_or_default(),
                report.clone(),
            )
        });
        let mut child = EngineChild::spawn(&mut command, debug_log_path.as_deref(), report)
            .map_err(|error| {
                let message = error.to_string();
                state.last_errors.insert(request.kind, message.clone());
                DesktopError::Process(message)
            })?;
        if let Some(bridge) = direct_bridge
            && let Some(session) = request
                .sharing_viewer_id
                .as_deref()
                .or(request.session_id.as_deref())
                .and_then(|id| Uuid::parse_str(id).ok())
        {
            child.direct_bridges.insert(session, bridge);
        }
        child.session_id = request
            .session_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok());
        if let Err(message) = stabilize_child(&mut child) {
            // `try_wait` reaps a child that already exited. If inspecting the
            // child failed instead, make a best-effort cleanup so launch never
            // leaves an unmanaged sidecar behind.
            let _ = child.process.kill();
            let _ = child.process.wait();
            state.last_errors.insert(request.kind, message.clone());
            return Err(DesktopError::Process(message));
        }
        if let Some(peer) = sharing {
            let viewer = peer.viewer;
            child.sharing = Some(crate::sharing_engine::SharingHost::new(
                crate::sharing_engine::SharingPeer::from_request(&native_request)?.unwrap_or(peer),
            ));
            if let Err(error) = child.wait_sharing_ack(viewer, Duration::from_secs(25)) {
                let _ = stop_child(&mut child);
                return Err(error);
            }
        }
        state.last_errors.remove(&request.kind);
        state.closed_viewers.remove(&request.kind);
        state.last_sessions.remove(&request.kind);
        state.children.insert(request.kind, child);
        Ok(())
    }

    pub fn add_sharing_peer(
        &self,
        app: &AppHandle,
        request: &LaunchEngineRequest,
        reserved_socket: Option<crate::direct::DirectTransport>,
    ) -> Result<(), DesktopError> {
        let mut native_request = request.clone();
        let prepared = prepare_direct(reserved_socket, &mut native_request)?;
        let request = &native_request;
        let _ = build_args(request)?;
        let peer = crate::sharing_engine::SharingPeer::from_request(request)?.ok_or_else(|| {
            DesktopError::InvalidRequest("sharing configuration is required".into())
        })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;
        let child = state
            .children
            .get_mut(&EngineKind::Host)
            .ok_or_else(|| DesktopError::Process("sharing host is not running".into()))?;
        if child
            .process
            .try_wait()
            .map_err(|_| DesktopError::Process("unable to inspect sharing host".into()))?
            .is_some()
        {
            return Err(DesktopError::Process("sharing host has exited".into()));
        }
        child
            .sharing
            .as_ref()
            .ok_or_else(|| {
                DesktopError::Process("running host is a single-peer remote-control session".into())
            })?
            .validate_add(&peer)?;
        let nonce = Uuid::new_v4();
        let command = peer.add_command(nonce, validated_session_token(request)?);
        let direct_bridge = prepared.map(|prepared| {
            prepared.start(
                app.clone(),
                request.kind,
                request.direct_attempt_id.clone().unwrap_or_default(),
                None,
            )
        });
        child.sharing_command(nonce, command.as_bytes())?;
        if let Some(sharing) = child.sharing.as_mut() {
            sharing.add(peer);
        }
        if let Some(bridge) = direct_bridge
            && let Some(session) = request
                .sharing_viewer_id
                .as_deref()
                .or(request.session_id.as_deref())
                .and_then(|id| Uuid::parse_str(id).ok())
        {
            child.direct_bridges.insert(session, bridge);
        }
        Ok(())
    }

    pub fn remove_sharing_peer(&self, group: Uuid, viewer: Uuid) -> Result<(), DesktopError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;
        let child = state
            .children
            .get_mut(&EngineKind::Host)
            .ok_or_else(|| DesktopError::Process("sharing host is not running".into()))?;
        let sharing = child
            .sharing
            .as_ref()
            .ok_or_else(|| DesktopError::Process("running host is not a sharing host".into()))?;
        if !sharing.validate_remove(group, viewer)? {
            return Ok(());
        }
        let nonce = Uuid::new_v4();
        child.sharing_command(
            nonce,
            format!("share-remove\t{nonce}\t{viewer}\n").as_bytes(),
        )?;
        if let Some(sharing) = child.sharing.as_mut() {
            sharing.remove(viewer);
            child.direct_bridges.remove(&viewer);
        }
        if let Ok(mut statuses) = child.sharing_statuses.lock() {
            statuses.remove(&viewer);
        }
        Ok(())
    }

    pub fn stop(&self, kind: EngineKind) -> Result<(), DesktopError> {
        self.stop_matching(kind, None)
    }

    pub fn stop_for_session(&self, kind: EngineKind, session: Uuid) -> Result<(), DesktopError> {
        self.stop_matching(kind, Some(session))
    }

    fn stop_matching(&self, kind: EngineKind, session: Option<Uuid>) -> Result<(), DesktopError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("engine state is unavailable".into()))?;
        if session.is_some_and(|session| {
            state
                .children
                .get(&kind)
                .is_none_or(|child| child.session_id != Some(session))
        }) {
            return Ok(());
        }
        let Some(mut child) = state.children.remove(&kind) else {
            return Ok(());
        };
        stop_child(&mut child).map_err(|error| DesktopError::Process(error.to_string()))?;
        Ok(())
    }

    pub fn record_relay_failure(&self, kind: EngineKind, session: Uuid, failure: &str) {
        if let Ok(mut state) = self.state.lock() {
            let owner = state
                .children
                .get(&kind)
                .and_then(|child| child.session_id)
                .or_else(|| state.last_sessions.get(&kind).copied());
            if owner == Some(session) && !state.closed_viewers.contains_key(&kind) {
                state.last_sessions.insert(kind, session);
                state.last_errors.insert(kind, failure.to_owned());
            }
        }
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
        let mut video_received = false;
        let mut session_id = state.last_sessions.get(&kind).copied();
        if let Some(child) = state.children.get_mut(&kind) {
            session_id = child.session_id;
            match child.process.try_wait() {
                Ok(None) => {
                    running = true;
                    process_id = Some(child.process.id());
                    video_received = child.first_frame.load(std::sync::atomic::Ordering::Relaxed);
                }
                Ok(Some(status)) => {
                    let message = child
                        .output
                        .exit_message(format!("process exited with {status}"));
                    if kind == EngineKind::Client && crate::engine_output::viewer_closed(&message) {
                        if let Some(id) = session_id {
                            state.closed_viewers.insert(kind, id);
                        }
                        state.last_errors.remove(&kind);
                    } else {
                        state.last_errors.insert(kind, message);
                    }
                    exited = true;
                }
                Err(error) => {
                    state.last_errors.insert(kind, error.to_string());
                    exited = true;
                }
            }
        }
        if exited {
            if let Some(id) = session_id {
                state.last_sessions.insert(kind, id);
            }
            state.children.remove(&kind);
        }
        Ok(EngineStatus {
            kind,
            session_id,
            installed,
            running,
            process_id,
            video_received,
            user_closed: state.closed_viewers.contains_key(&kind),
            last_error: state
                .last_errors
                .get(&kind)
                .map(|message| crate::engine_output::display_error(message)),
            startup_failure: state
                .last_errors
                .get(&kind)
                .and_then(|message| crate::engine_output::startup_failure(message)),
            route_failure: state.last_errors.get(&kind).and_then(|message| {
                crate::engine_output::route_failure(message)
                    .or_else(|| crate::engine_output::relay_route_failure(message))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv6Addr;

    use super::*;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn relay_reset_after_native_exit_remains_bound_to_the_exited_session() {
        let manager = EngineManager::default();
        let old = Uuid::new_v4();
        let new = Uuid::new_v4();
        manager
            .state
            .lock()
            .unwrap()
            .last_sessions
            .insert(EngineKind::Client, old);
        let reset = "relay connection failed: WebSocket protocol error: Connection reset without closing handshake";
        manager.record_relay_failure(EngineKind::Client, old, reset);
        assert_eq!(
            manager
                .state
                .lock()
                .unwrap()
                .last_errors
                .get(&EngineKind::Client)
                .map(String::as_str),
            Some(reset)
        );
        manager
            .state
            .lock()
            .unwrap()
            .last_sessions
            .insert(EngineKind::Client, new);
        manager.record_relay_failure(EngineKind::Client, old, "late old failure");
        manager
            .state
            .lock()
            .unwrap()
            .closed_viewers
            .insert(EngineKind::Client, new);
        manager.record_relay_failure(EngineKind::Client, new, "user close is not route failure");
        assert_eq!(
            manager
                .state
                .lock()
                .unwrap()
                .last_errors
                .get(&EngineKind::Client)
                .map(String::as_str),
            Some(reset)
        );
    }

    #[cfg(unix)]
    #[test]
    fn stopping_an_old_session_does_not_close_a_new_native_process()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "read ignored"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = EngineChild::spawn(&mut command, None, None)?;
        let current = Uuid::new_v4();
        child.session_id = Some(current);
        let manager = EngineManager::default();
        manager
            .state
            .lock()
            .map_err(|_| "state lock")?
            .children
            .insert(EngineKind::Client, child);
        manager.stop_for_session(EngineKind::Client, Uuid::new_v4())?;
        let retained = manager
            .state
            .lock()
            .map_err(|_| "state lock")?
            .children
            .get_mut(&EngineKind::Client)
            .is_some_and(|child| {
                child
                    .process
                    .try_wait()
                    .is_ok_and(|status| status.is_none())
            });
        manager.stop_for_session(EngineKind::Client, current)?;
        assert!(
            retained,
            "late cleanup must leave a differently owned process alive"
        );
        assert!(
            !manager
                .state
                .lock()
                .map_err(|_| "state lock")?
                .children
                .contains_key(&EngineKind::Client)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn sharing_startup_preserves_delayed_capture_error() -> Result<(), Box<dyn std::error::Error>> {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "sleep 0.1; printf 'Screen Recording denied; quit and reopen Sanser\\n' >&2; exit 1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = EngineChild::spawn(&mut command, None, None)?;
        let result = child.wait_sharing_ack(Uuid::new_v4(), Duration::from_secs(2));
        assert!(matches!(result, Err(DesktopError::Process(ref message))
            if message.contains("Screen Recording denied") && !message.contains("timed out")));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn sharing_startup_accepts_delayed_ack() -> Result<(), Box<dyn std::error::Error>> {
        let request = Uuid::new_v4();
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "sleep 0.1; printf 'SNSHARE_ACK request=%s accepted=1\\n' \"$SANSER_TEST_ACK\"",
            ])
            .env("SANSER_TEST_ACK", request.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = EngineChild::spawn(&mut command, None, None)?;
        let result = child.wait_sharing_ack(request, Duration::from_secs(2));
        child.process.wait()?;
        result?;
        Ok(())
    }

    fn request(kind: EngineKind) -> LaunchEngineRequest {
        LaunchEngineRequest {
            ultra_low_latency: false,
            direct_check: None,
            kind,
            session_id: Some("018f4d89-5e8b-7a80-bd1e-cb7cb9f43189".into()),
            sharing_id: None,
            sharing_viewer_id: None,
            address: Some("192.0.2.10".into()),
            port: Some(50_000),
            codec: VideoCodec::Auto,
            fps: 60,
            bitrate_kbps: 25_000,
            width: 1920,
            height: 1080,
            network_mode: crate::models::NetworkMode::DirectOnly,
            audio_enabled: true,
            input_enabled: true,
            relative_mouse: false,
            session_token: Some("a".repeat(32)),
            udp_connect: None,
            udp_bind_port: None,
            relay: false,
            relay_transport: None,
            direct_probe: None,
            direct_attempt_id: None,
            wire_protocol: None,
        }
    }

    #[test]
    fn sharing_args_select_one_snv2_host_and_never_enable_input() -> Result<(), DesktopError> {
        let mut value = request(EngineKind::Host);
        let viewer = Uuid::new_v4().to_string();
        value.sharing_id = Some(Uuid::new_v4().to_string());
        value.sharing_viewer_id = Some(viewer.clone());
        value.wire_protocol = Some("snv2".into());
        value.input_enabled = false;
        value.audio_enabled = false;
        value.udp_bind_port = Some(50002);
        let args = build_args(&value)?;
        assert!(args.contains(&"--snv2".into()));
        assert!(args.contains(&"--share-host".into()));
        assert!(args.contains(&viewer));
        assert!(args.contains(&"--disable-input".into()));
        assert!(
            !args.contains(
                value
                    .session_token
                    .as_ref()
                    .ok_or_else(|| DesktopError::InvalidRequest("test token missing".into()))?
            )
        );
        value.wire_protocol = Some("legacy".into());
        assert!(build_args(&value).is_err());
        Ok(())
    }

    #[test]
    fn ultra_latency_reaches_both_engines_without_reenabling_tcp_input() -> Result<(), DesktopError>
    {
        for kind in [EngineKind::Host, EngineKind::Client] {
            let mut value = request(kind);
            assert!(!build_args(&value)?.contains(&"--ultra-low-latency".into()));
            value.ultra_low_latency = true;
            for protocol in ["legacy", "snv2"] {
                value.wire_protocol = Some(protocol.into());
                let args = build_args(&value)?;
                assert!(args.contains(&"--ultra-low-latency".into()));
                if kind == EngineKind::Client {
                    assert!(args.windows(2).any(|pair| pair == ["--control-port", "0"]));
                    assert!(args.contains(&"--udp-video".into()));
                }
            }
        }
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn native_child_runs_without_allocating_a_console() -> std::io::Result<()> {
        let output = native_command(Path::new("powershell.exe"))
            .args([
                "-NoProfile", "-NonInteractive", "-Command",
                "Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class ConsoleProbe { [DllImport(\"kernel32.dll\")] public static extern IntPtr GetConsoleWindow(); }'; [ConsoleProbe]::GetConsoleWindow().ToInt64()",
            ])
            .stdin(Stdio::null())
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "0");
        Ok(())
    }

    #[test]
    fn desktop_uses_local_cursor_without_changing_other_native_arguments()
    -> Result<(), DesktopError> {
        let mut value = request(EngineKind::Host);
        assert!(build_args(&value)?.contains(&"--no-stream-cursor".into()));
        value.wire_protocol = Some("snv2".into());
        assert!(!build_args(&value)?.contains(&"--no-stream-cursor".into()));
        Ok(())
    }

    #[test]
    fn formats_ipv6_endpoints_without_ambiguity() {
        let address = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert_eq!(endpoint(address, 5000), "[2001:db8::1]:5000");
        let mut ipv6 = request(EngineKind::Host);
        ipv6.address = Some("2001:db8::1".into());
        assert!(matches!(
            build_args(&ipv6),
            Err(DesktopError::InvalidRequest(_))
        ));
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
                .any(|pair| pair == ["--udp-connect", "192.0.2.10:50000"])
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--control-connect" || arg == "--audio-udp-connect")
        );
        Ok(())
    }

    #[test]
    fn host_stream_uses_fps_pacing_without_screenshot_delay() -> Result<(), DesktopError> {
        let mut host = request(EngineKind::Host);
        let args = build_args(&host)?;
        assert!(args.windows(2).any(|pair| pair == ["--interval-ms", "0"]));
        assert!(args.windows(2).any(|pair| pair == ["--fps", "60"]));

        // SNV2 has its own FPS loop and rejects legacy-only arguments.
        host.wire_protocol = Some("snv2".into());
        let args = build_args(&host)?;
        assert!(args.iter().any(|arg| arg == "--snv2"));
        assert!(!args.iter().any(|arg| arg == "--interval-ms"));
        Ok(())
    }

    #[test]
    fn snv2_audio_respects_both_host_and_viewer_preferences() -> Result<(), DesktopError> {
        for kind in [EngineKind::Host, EngineKind::Client] {
            let mut value = request(kind);
            value.wire_protocol = Some("snv2".into());
            value.audio_enabled = true;
            assert!(
                !build_args(&value)?
                    .iter()
                    .any(|arg| arg == "--disable-audio")
            );
            value.audio_enabled = false;
            assert!(
                build_args(&value)?
                    .iter()
                    .any(|arg| arg == "--disable-audio")
            );
        }
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
    fn host_keeps_authenticated_control_when_remote_input_is_disabled() -> Result<(), DesktopError>
    {
        let mut disabled = request(EngineKind::Host);
        disabled.input_enabled = false;
        let args = build_args(&disabled)?;
        assert!(args.iter().any(|argument| argument == "--disable-input"));
        assert!(!args.iter().any(|arg| arg == "--control-connect"));
        Ok(())
    }

    #[test]
    fn legacy_mac_viewer_disables_input_without_disabling_control() -> Result<(), DesktopError> {
        let mut viewer = request(EngineKind::Client);
        viewer.input_enabled = false;
        let args = build_args(&viewer)?;
        assert!(args.iter().any(|arg| arg == "--disable-input"));
        assert!(args.iter().any(|arg| arg == "--udp-video"));
        Ok(())
    }

    #[test]
    fn rejects_removed_local_server_mode() {
        assert!(matches!(
            build_args(&request(EngineKind::LocalServer)),
            Err(DesktopError::Unavailable(_))
        ));
    }

    #[test]
    fn cross_platform_args_require_compatible_media_and_preserve_input_permissions()
    -> Result<(), DesktopError> {
        let mut modern = request(EngineKind::Client);
        modern.wire_protocol = Some("snv2".into());
        modern.input_enabled = false;
        let args = build_args(&modern)?;
        assert!(args.iter().any(|arg| arg == "--snv2"));
        assert!(args.iter().any(|arg| arg == "--disable-input"));
        modern.codec = VideoCodec::Hevc;
        assert!(build_args(&modern).is_err());
        modern.codec = VideoCodec::Auto;
        modern.relative_mouse = true;
        assert!(build_args(&modern).is_err());
        modern.relative_mouse = false;
        modern.wire_protocol = Some("unknown".into());
        assert!(build_args(&modern).is_err());
        Ok(())
    }

    #[test]
    fn capability_parser_requires_matching_engine_and_media_implementation() -> Result<(), String> {
        let version = crate::models::SANSER_VERSION;
        let engine = sidecar_engine_id(EngineKind::Client);
        let valid_json = format!(
            r#"{{"product":"Sanser","version":"{version}","protocolVersion":2,"engine":"{engine}","nativeSnv2":true,"nativeDirect":true,"h264DecoderImplementation":true,"hevcDecoderImplementation":true}}"#
        );
        let parsed = parse_sidecar_capabilities(valid_json.as_bytes(), EngineKind::Client)?;
        assert!(parsed.native_snv2);
        assert!(parsed.native_direct);
        assert!(parsed.hevc_decoder_implementation);
        assert!(!parsed.snv2_audio_implementation);
        // Existing legacy audio must not falsely advertise SNV2 support.
        let legacy_audio = valid_json.replace(
            "\"hevcDecoderImplementation\":true",
            "\"hevcDecoderImplementation\":true,\"audioImplementation\":true",
        );
        assert!(
            !parse_sidecar_capabilities(legacy_audio.as_bytes(), EngineKind::Client)?
                .snv2_audio_implementation
        );
        let snv2_audio = valid_json.replace(
            "\"hevcDecoderImplementation\":true",
            "\"hevcDecoderImplementation\":true,\"snv2AudioImplementation\":true",
        );
        assert!(
            parse_sidecar_capabilities(snv2_audio.as_bytes(), EngineKind::Client)?
                .snv2_audio_implementation
        );

        let wrong_engine_json = format!(
            r#"{{"product":"Sanser","version":"{version}","protocolVersion":2,"engine":"host-windows","nativeSnv2":true,"nativeDirect":true,"h264DecoderImplementation":true}}"#
        );
        assert!(
            parse_sidecar_capabilities(wrong_engine_json.as_bytes(), EngineKind::Client).is_err()
        );
        Ok(())
    }
}
