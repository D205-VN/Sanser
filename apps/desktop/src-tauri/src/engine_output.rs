//! Continuously drain child stderr while retaining a bounded error tail.
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

const TAIL_BYTES: usize = 8 * 1024;
const DEBUG_FILE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) struct EngineOutput {
    tail: Arc<Mutex<VecDeque<u8>>>,
    completed: mpsc::Receiver<()>,
}

impl EngineOutput {
    pub(crate) fn start(
        reader: impl Read + Send + 'static,
        log: Option<File>,
    ) -> std::io::Result<Self> {
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_BYTES)));
        let worker_tail = Arc::clone(&tail);
        let (complete, completed) = mpsc::channel();
        thread::Builder::new()
            .name("sanser-engine-output".into())
            .spawn(move || {
                drain(reader, log, &worker_tail);
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

fn drain(mut reader: impl Read, mut log: Option<File>, tail: &Mutex<VecDeque<u8>>) {
    let mut buffer = [0_u8; 4096];
    let mut written = 0;
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
