//! Continuously drain child stderr while retaining a bounded error tail.
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{Read, Write},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

const TAIL_BYTES: usize = 8 * 1024;
const DEBUG_FILE_BYTES: usize = 2 * 1024 * 1024;

const STARTUP_ERROR_PREFIX: &str = "SNU1 UDP render listener error: [startup:";

pub(crate) type SharingStatuses =
    Arc<Mutex<HashMap<uuid::Uuid, (crate::models::SharingPeerStatus, Instant)>>>;

fn sharing_peer_status(line: &[u8]) -> Option<crate::models::SharingPeerStatus> {
    use crate::models::{SharingPeerStatus, SharingVideoState};
    let line = std::str::from_utf8(line)
        .ok()?
        .strip_prefix("SNSHARE_PEER ")?;
    let mut viewer = None;
    let mut state = None;
    let mut pauses = None;
    let mut backoff = None;
    let mut ready = None;
    let mut running = None;
    for field in line.split_ascii_whitespace() {
        let (key, value) = field.split_once('=')?;
        match key {
            "viewer" if viewer.is_none() => viewer = Some(uuid::Uuid::parse_str(value).ok()?),
            "videoState" if state.is_none() => {
                state = Some(match value {
                    "active" => SharingVideoState::Active,
                    "paused" => SharingVideoState::Paused,
                    "probing" => SharingVideoState::Probing,
                    _ => return None,
                });
            }
            "pauses" if pauses.is_none() => pauses = Some(value.parse::<u64>().ok()?),
            "backoffMs" if backoff.is_none() => backoff = Some(value.parse::<u32>().ok()?),
            "ready" if ready.is_none() => {
                ready = Some(match value {
                    "0" => false,
                    "1" => true,
                    _ => return None,
                });
            }
            "running" if running.is_none() => {
                running = Some(match value {
                    "0" => false,
                    "1" => true,
                    _ => return None,
                });
            }
            "viewer" | "videoState" | "pauses" | "backoffMs" | "ready" | "running" => return None,
            _ => {}
        }
    }
    Some(SharingPeerStatus {
        viewer_id: viewer?,
        ready: ready?,
        running: running?,
        video_state: state?,
        pauses: pauses?,
        backoff_ms: backoff?,
    })
}

#[derive(Debug, PartialEq)]
pub(crate) struct SharingAck {
    pub request: uuid::Uuid,
    pub accepted: bool,
}

fn sharing_ack(line: &[u8]) -> Option<SharingAck> {
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let fields = line.strip_prefix("SNSHARE_ACK request=")?;
    let (request, accepted) = fields.split_once(" accepted=")?;
    Some(SharingAck {
        request: uuid::Uuid::parse_str(request).ok()?,
        accepted: match accepted {
            "1" => true,
            "0" => false,
            _ => return None,
        },
    })
}

/// Classify only an explicit terminal native startup failure. A user closing
/// the viewer, a crash or a generic timeout must never trigger auto-reopening.
pub(crate) fn startup_failure(message: &str) -> Option<crate::models::NativeStartupFailure> {
    use crate::models::NativeStartupFailure;
    let line = message.lines().last()?;
    let (code, _) = line.strip_prefix(STARTUP_ERROR_PREFIX)?.split_once("] ")?;
    match code {
        "no-udp" => Some(NativeStartupFailure::NoUdp),
        "unexpected-peer" => Some(NativeStartupFailure::UnexpectedPeer),
        "no-video" => Some(NativeStartupFailure::NoVideo),
        "auth-rejected" => Some(NativeStartupFailure::AuthRejected),
        "incomplete-video" => Some(NativeStartupFailure::IncompleteVideo),
        "decode-failed" => Some(NativeStartupFailure::DecodeFailed),
        _ => None,
    }
}

pub(crate) fn display_error(message: &str) -> String {
    if startup_failure(message).is_some()
        && let Some(line) = message.lines().last()
        && let Some((_, detail)) = line.split_once("] ")
    {
        return detail.to_owned();
    }
    message.to_owned()
}

/// Only an explicit terminal native transport timeout can cause reopening.
/// User close, host Disconnect, auth/decoder errors and input ACK timeouts do not.
pub(crate) fn route_failure(message: &str) -> Option<crate::models::NativeRouteFailure> {
    let last = message.lines().last()?.trim_end_matches('\r');
    (last=="SNV2_STOP reason=2 (2=peer-timeout 3=input-ack-timeout 4=priority-queue-overflow 5=transport-error)")
        .then_some(crate::models::NativeRouteFailure::PeerTimeout)
}

pub(crate) fn relay_route_failure(message: &str) -> Option<crate::models::NativeRouteFailure> {
    (message == "relay peer went offline; retry the accepted session"
        || message.starts_with("relay connection failed: ")
        || message.starts_with("local relay UDP bridge stopped receiving packets: "))
    .then_some(crate::models::NativeRouteFailure::RelayLost)
}

pub(crate) struct EngineOutput {
    tail: Arc<Mutex<VecDeque<u8>>>,
    completed: mpsc::Receiver<()>,
}

impl EngineOutput {
    #[cfg(test)]
    pub(crate) fn start(
        reader: impl Read + Send + 'static,
        log: Option<File>,
    ) -> std::io::Result<Self> {
        Self::start_recording(reader, log, None)
    }

    pub(crate) fn start_recording(
        reader: impl Read + Send + 'static,
        log: Option<File>,
        metrics: Option<mpsc::SyncSender<crate::connection_report::Sample>>,
    ) -> std::io::Result<Self> {
        Self::start_observed(reader, log, metrics, None, None, None)
    }

    pub(crate) fn start_observed(
        reader: impl Read + Send + 'static,
        log: Option<File>,
        metrics: Option<mpsc::SyncSender<crate::connection_report::Sample>>,
        acknowledgements: Option<mpsc::SyncSender<SharingAck>>,
        first_frame: Option<Arc<std::sync::atomic::AtomicBool>>,
        sharing: Option<SharingStatuses>,
    ) -> std::io::Result<Self> {
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_BYTES)));
        let worker_tail = Arc::clone(&tail);
        let (complete, completed) = mpsc::channel();
        thread::Builder::new()
            .name("sanser-engine-output".into())
            .spawn(move || {
                drain(
                    reader,
                    log,
                    &worker_tail,
                    metrics.as_ref(),
                    acknowledgements.as_ref(),
                    first_frame.as_deref(),
                    sharing.as_ref(),
                );
                let _ = complete.send(());
            })?;
        Ok(Self { tail, completed })
    }

    // Call only after the child has exited. A descendant retaining stderr cannot
    // hold the UI indefinitely; reading continues even after this bounded wait.
    pub(crate) fn exit_message(&self, fallback: String) -> String {
        let _ = self.completed.recv_timeout(Duration::from_millis(200));
        let Ok(tail) = self.tail.lock() else {
            return fallback;
        };
        let bytes: Vec<u8> = tail.iter().copied().collect();
        let message = String::from_utf8_lossy(&bytes);
        if message.trim().is_empty() {
            fallback
        } else {
            // Keep the final error rather than startup chatter.
            let start = message.char_indices().rev().nth(2047).map_or(0, |(i, _)| i);
            message[start..].trim().to_owned()
        }
    }
}

fn drain(
    mut reader: impl Read,
    mut log: Option<File>,
    tail: &Mutex<VecDeque<u8>>,
    metrics: Option<&mpsc::SyncSender<crate::connection_report::Sample>>,
    acknowledgements: Option<&mpsc::SyncSender<SharingAck>>,
    first_frame: Option<&std::sync::atomic::AtomicBool>,
    sharing: Option<&SharingStatuses>,
) {
    let mut buffer = [0_u8; 4096];
    let mut written = 0;
    let mut line = Vec::with_capacity(4096);
    let mut oversized = false;
    loop {
        let length = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => length,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if let Ok(mut tail) = tail.lock() {
            let excess = (tail.len() + length).saturating_sub(TAIL_BYTES);
            tail.drain(..excess);
            tail.extend(&buffer[..length]);
        }
        if metrics.is_some()
            || acknowledgements.is_some()
            || first_frame.is_some()
            || sharing.is_some()
        {
            for byte in &buffer[..length] {
                if *byte == b'\n' {
                    if !oversized
                        && let Some(sharing) = sharing
                        && let Some(status) = sharing_peer_status(&line)
                        && let Ok(mut sharing) = sharing.lock()
                        && (sharing.len() < 3 || sharing.contains_key(&status.viewer_id))
                    {
                        sharing.insert(status.viewer_id, (status, Instant::now()));
                    }
                    if !oversized
                        && line == b"SNV2_FIRST_FRAME decoded=1"
                        && let Some(first_frame) = first_frame
                    {
                        first_frame.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    if !oversized
                        && let Some(metrics) = metrics
                        && let Some(sample) = crate::connection_report::parse_sample(&line)
                    {
                        // Disk writing occurs in another worker; never block the native engine.
                        let _ = metrics.try_send(sample);
                    }
                    if !oversized
                        && let Some(acknowledgements) = acknowledgements
                        && let Some(ack) = sharing_ack(&line)
                    {
                        let _ = acknowledgements.try_send(ack);
                    }
                    line.clear();
                    oversized = false;
                } else if line.len() < 4096 {
                    line.push(*byte);
                } else {
                    oversized = true;
                }
            }
        }
        if let Some(file) = log.as_mut() {
            let count = length.min(DEBUG_FILE_BYTES.saturating_sub(written));
            if count > 0 && file.write_all(&buffer[..count]).is_ok() {
                written += count;
            } else {
                // Keep draining stderr after the file reaches its cap or disk I/O fails.
                log = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn route_recovery_requires_explicit_transport_failure_not_user_close_or_auth_errors() {
        use crate::models::NativeRouteFailure;
        let timeout = "SNV2_STOP reason=2 (2=peer-timeout 3=input-ack-timeout 4=priority-queue-overflow 5=transport-error)";
        assert_eq!(
            route_failure(timeout),
            Some(NativeRouteFailure::PeerTimeout)
        );
        for message in [
            "process exited with exit code: 0",
            "connection timed out",
            "decoder failed",
            "SNV2_STOP reason=3 (2=peer-timeout 3=input-ack-timeout 4=priority-queue-overflow 5=transport-error)",
            "error contains SNV2_STOP reason=2",
            "SNV2_STOP reason=2\nuser closed the window",
        ] {
            assert_eq!(route_failure(message), None);
        }
        assert_eq!(route_failure(&format!("{timeout}\ndecode failed")), None);
        assert_eq!(
            relay_route_failure("relay peer went offline; retry the accepted session"),
            Some(NativeRouteFailure::RelayLost)
        );
        assert_eq!(
            relay_route_failure("relay connection failed: connection reset"),
            Some(NativeRouteFailure::RelayLost)
        );
        assert_eq!(
            relay_route_failure(
                "relay frame authentication repeatedly failed; refresh the accepted session on both devices"
            ),
            None
        );
        assert_eq!(relay_route_failure("relay cipher lock failed"), None);
    }

    #[test]
    fn sharing_health_status_is_bounded_and_observed_without_metrics() -> std::io::Result<()> {
        use crate::models::SharingVideoState;
        use std::fmt::Write as _;
        let viewers = (0..4).map(|_| uuid::Uuid::new_v4()).collect::<Vec<_>>();
        let mut lines = String::new();
        for id in &viewers {
            writeln!(
                lines,
                "SNSHARE_PEER viewer={id} ready=1 running=1 videoState=paused pauses=2 backoffMs=2000"
            )
            .map_err(std::io::Error::other)?;
        }
        writeln!(
            lines,
            "SNSHARE_PEER viewer={} ready=1 running=1 videoState=probing pauses=2 backoffMs=500",
            viewers[0]
        )
        .map_err(std::io::Error::other)?;
        let sharing = Arc::new(Mutex::new(HashMap::new()));
        let output = EngineOutput::start_observed(
            Cursor::new(lines.into_bytes()),
            None,
            None,
            None,
            None,
            Some(Arc::clone(&sharing)),
        )?;
        let _ = output.exit_message(String::new());
        let statuses = sharing
            .lock()
            .map_err(|_| std::io::Error::other("status lock"))?;
        assert_eq!(statuses.len(), 3);
        assert_eq!(
            statuses[&viewers[0]].0.video_state,
            SharingVideoState::Probing
        );
        assert!(!statuses.contains_key(&viewers[3]));
        for invalid in [
            "SNSHARE_PEER viewer=secret ready=1 running=1 videoState=paused pauses=2 backoffMs=2000".to_owned(),
            format!(
                "SNSHARE_PEER viewer={} ready=1 running=1 videoState=unknown pauses=2 backoffMs=2000",
                viewers[0]
            ),
            format!(
                "SNSHARE_PEER viewer={} ready=1 running=1 videoState=active videoState=paused pauses=2 backoffMs=2000",
                viewers[0]
            ),
        ] {
            assert!(sharing_peer_status(invalid.as_bytes()).is_none());
        }
        Ok(())
    }

    #[test]
    fn only_decoded_frame_event_marks_video_received() -> std::io::Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let flag = Arc::new(AtomicBool::new(false));
        let output = EngineOutput::start_observed(
            Cursor::new(b"SNSHARE_STATUS viewers=3 readyViewers=3\n"),
            None,
            None,
            None,
            Some(Arc::clone(&flag)),
            None,
        )?;
        let _ = output.exit_message(String::new());
        assert!(!flag.load(Ordering::Relaxed));
        let output = EngineOutput::start_observed(
            Cursor::new(b"SNV2_FIRST_FRAME decoded=1\n"),
            None,
            None,
            None,
            Some(Arc::clone(&flag)),
            None,
        )?;
        let _ = output.exit_message(String::new());
        assert!(flag.load(Ordering::Relaxed));
        Ok(())
    }

    #[test]
    fn sharing_ack_is_bounded_and_works_without_metrics() -> std::io::Result<()> {
        let request = uuid::Uuid::new_v4();
        let bytes=format!("SNSHARE_ACK request={request} accepted=1\nSNSHARE_ACK request={request} accepted=0 extra=token\n").into_bytes();
        let (sender, receiver) = mpsc::sync_channel(1);
        let output =
            EngineOutput::start_observed(Cursor::new(bytes), None, None, Some(sender), None, None)?;
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(std::io::Error::other)?,
            SharingAck {
                request,
                accepted: true
            }
        );
        let _ = output.exit_message(String::new());
        assert!(receiver.try_recv().is_err());
        assert!(sharing_ack(b"SNSHARE_ACK request=token accepted=1").is_none());
        Ok(())
    }

    #[test]
    fn a_full_report_queue_does_not_block_output_draining() -> std::io::Result<()> {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let bytes = b"SNINPUT_RTT rttMs=150\n".repeat(200_000);
        let output = EngineOutput::start_recording(Cursor::new(bytes), None, Some(sender))?;
        assert!(
            output
                .completed
                .recv_timeout(Duration::from_secs(5))
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn only_explicit_terminal_startup_errors_are_classified() {
        use crate::models::NativeStartupFailure;
        for (code, expected) in [
            ("no-udp", NativeStartupFailure::NoUdp),
            ("unexpected-peer", NativeStartupFailure::UnexpectedPeer),
            ("no-video", NativeStartupFailure::NoVideo),
            ("auth-rejected", NativeStartupFailure::AuthRejected),
            ("incomplete-video", NativeStartupFailure::IncompleteVideo),
            ("decode-failed", NativeStartupFailure::DecodeFailed),
        ] {
            let message =
                format!("old diagnostic\n{STARTUP_ERROR_PREFIX}{code}] Human-readable error");
            assert_eq!(startup_failure(&message), Some(expected));
            assert_eq!(display_error(&message), "Human-readable error");
        }
        for message in [
            "process exited with exit status: 0",
            "UDP receive timed out",
            "[startup:no-udp] unrelated output",
            "SNU1 UDP render listener error: [startup:unknown] error",
            "SNU1 UDP render listener error: [startup:no-udp] error\nuser closed viewer",
        ] {
            assert_eq!(startup_failure(message), None);
            assert_eq!(display_error(message), message);
        }
    }

    #[test]
    fn noisy_output_retains_only_bounded_tail_and_final_error() -> std::io::Result<()> {
        let mut bytes = vec![b'x'; 4 * 1024 * 1024];
        bytes.extend_from_slice(b"\nfinal encoder error\n");
        let output = EngineOutput::start(Cursor::new(bytes), None)?;
        // Await EOF separately to avoid a slow machine affecting this unit check.
        assert!(
            output
                .completed
                .recv_timeout(Duration::from_secs(5))
                .is_ok()
        );
        let tail = output
            .tail
            .lock()
            .map_err(|_| std::io::Error::other("tail poisoned"))?;
        assert_eq!(tail.len(), TAIL_BYTES);
        drop(tail);
        assert!(
            output
                .exit_message("fallback".into())
                .ends_with("final encoder error")
        );
        Ok(())
    }

    #[test]
    fn debug_file_stops_growing_while_stderr_keeps_draining() -> std::io::Result<()> {
        let path = std::env::temp_dir().join(format!("sanser-output-{}.log", uuid::Uuid::new_v4()));
        let file = File::create(&path)?;
        let mut bytes = vec![b'x'; DEBUG_FILE_BYTES * 2];
        bytes.extend_from_slice(b"last error after file cap");
        let output = EngineOutput::start(Cursor::new(bytes), Some(file))?;
        assert!(
            output
                .completed
                .recv_timeout(Duration::from_secs(5))
                .is_ok()
        );
        assert_eq!(std::fs::metadata(&path)?.len(), DEBUG_FILE_BYTES as u64);
        assert!(
            output
                .exit_message("fallback".into())
                .ends_with("last error after file cap")
        );
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn chatty_child_exceeds_pipe_capacity_without_blocking() -> std::io::Result<()> {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sh")
            .args([
                "-c",
                "head -c 4194304 /dev/zero >&2; printf 'final child error' >&2",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("missing stderr"))?;
        let output = EngineOutput::start(stderr, None)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                return Err(std::io::Error::other("child stalled on output"));
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            output
                .exit_message("fallback".into())
                .ends_with("final child error")
        );
        Ok(())
    }
}
