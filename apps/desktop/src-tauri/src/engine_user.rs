//! Private user payloads must be removed before stdout reaches diagnostics.
use crate::error::DesktopError;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    io::{self, BufRead, BufReader, Read, Write},
    sync::{Arc, Mutex, Weak, mpsc},
    thread,
    time::Duration,
};
use uuid::Uuid;

pub const CLIPBOARD_BYTES: usize = 8192;
const MAX_LINE: usize = CLIPBOARD_BYTES * 2 + 128;
#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub unicode_input: bool,
    pub clipboard: bool,
    pub local_clipboard: bool,
}
struct Reply {
    status: u8,
    text: String,
}
#[derive(Default)]
struct State {
    capabilities: Capabilities,
    pending: HashMap<Uuid, mpsc::SyncSender<Reply>>,
}
#[derive(Default)]
pub struct UserControl {
    state: Mutex<State>,
    files: Mutex<Option<crate::file_transfer::Service>>,
}
impl Drop for UserControl {
    fn drop(&mut self) {
        self.close();
    }
}
pub struct Request {
    id: Uuid,
    control: Weak<UserControl>,
    reply: mpsc::Receiver<Reply>,
}
impl Drop for Request {
    fn drop(&mut self) {
        if let Some(control) = self.control.upgrade()
            && let Ok(mut state) = control.state.lock()
        {
            state.pending.remove(&self.id);
        }
    }
}
fn invalid() -> DesktopError {
    DesktopError::InvalidRequest("Unsupported or oversized text payload".into())
}
fn valid_text(text: &str, clipboard: bool) -> bool {
    (clipboard || !text.is_empty())
        && text.len() <= if clipboard { CLIPBOARD_BYTES } else { 1024 }
        && (clipboard || text.encode_utf16().count() <= 256)
        && text
            .chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\t' || (clipboard && c == '\r'))
}
fn hex(text: &str) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}
fn unhex(text: &str) -> Option<String> {
    if text.len() > CLIPBOARD_BYTES * 2 || !text.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let bytes = text
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect::<Option<Vec<_>>>()?;
    let value = String::from_utf8(bytes).ok()?;
    valid_text(&value, true).then_some(value)
}
impl UserControl {
    pub fn connect_files(&self, pipe: mpsc::SyncSender<Vec<u8>>) -> io::Result<()> {
        let service = crate::file_transfer::Service::start(pipe)?;
        let Ok(mut files) = self.files.lock() else {
            service.close();
            return Err(io::Error::other("File controls unavailable"));
        };
        if let Some(old) = files.replace(service) {
            old.close();
        }
        Ok(())
    }
    pub fn files(&self) -> Option<crate::file_transfer::Service> {
        self.files.lock().ok().and_then(|v| v.clone())
    }
    pub fn capabilities(&self) -> Capabilities {
        self.state
            .lock()
            .map(|s| s.capabilities)
            .unwrap_or_default()
    }
    pub fn prepare(
        self: &Arc<Self>,
        operation: &str,
        text: &str,
    ) -> Result<(Request, Vec<u8>), DesktopError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DesktopError::Process("Native controls unavailable".into()))?;
        let name = match operation {
            "text" if state.capabilities.unicode_input && valid_text(text, false) => "user-text",
            "clipboard-read" if state.capabilities.clipboard && text.is_empty() => "clipboard-read",
            "clipboard-write" if state.capabilities.clipboard && valid_text(text, true) => {
                "clipboard-write"
            }
            "clipboard-local-read" if state.capabilities.local_clipboard && text.is_empty() => {
                "clipboard-local-read"
            }
            "clipboard-local-write"
                if state.capabilities.local_clipboard && valid_text(text, true) =>
            {
                "clipboard-local-write"
            }
            _ => return Err(invalid()),
        };
        if state.pending.len() >= 4 {
            return Err(DesktopError::Process(
                "Another text operation is still pending".into(),
            ));
        }
        let id = Uuid::new_v4();
        let (sender, reply) = mpsc::sync_channel(1);
        state.pending.insert(id, sender);
        let command = if operation == "clipboard-read" || operation == "clipboard-local-read" {
            format!("{name}\t{id}\n")
        } else {
            format!("{name}\t{id}\t{}\n", hex(text))
        };
        Ok((
            Request {
                id,
                control: Arc::downgrade(self),
                reply,
            },
            command.into_bytes(),
        ))
    }
    fn observe(&self, line: &[u8]) {
        let Ok(line) = std::str::from_utf8(line) else {
            return;
        };
        // A public numeric record can precede the atomic private fprintf call.
        let Some(position) = line.find("SNUSER_") else {
            return;
        };
        if line[position..].starts_with("SNUSER_FILE_") {
            if let Some(files) = self.files() {
                files.record(&line[position..]);
            }
            return;
        }
        let fields = line[position..]
            .trim_end_matches(['\n', '\r'])
            .split('\t')
            .collect::<Vec<_>>();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if matches!(fields.len(), 3 | 4)
            && fields[0] == "SNUSER_CAP"
            && matches!(fields[1], "0" | "1")
            && matches!(fields[2], "0" | "1")
            && (fields.len() == 3 || matches!(fields[3], "0" | "1"))
        {
            state.capabilities = Capabilities {
                unicode_input: fields[1] == "1",
                clipboard: fields[2] == "1",
                local_clipboard: fields.get(3).is_some_and(|value| *value == "1"),
            };
        } else if fields.len() == 4 && fields[0] == "SNUSER_REPLY" {
            let (Ok(id), Ok(status), Some(text)) = (
                Uuid::parse_str(fields[1]),
                fields[2].parse::<u8>(),
                unhex(fields[3]),
            ) else {
                return;
            };
            if status > 5 {
                return;
            }
            if let Some(sender) = state.pending.remove(&id) {
                let _ = sender.try_send(Reply { status, text });
            }
        }
    }
    fn close(&self) {
        if let Some(files) = self.files() {
            files.close();
        }
        if let Ok(mut state) = self.state.lock() {
            state.capabilities = Capabilities::default();
            for (_, sender) in state.pending.drain() {
                let _ = sender.try_send(Reply {
                    status: 3,
                    text: String::new(),
                });
            }
        }
    }
}
impl Request {
    pub fn wait(self) -> Result<String, DesktopError> {
        let reply = self
            .reply
            .recv_timeout(Duration::from_secs(11))
            .map_err(|_| {
                DesktopError::Process("Text operation timed out; check the connection".into())
            })?;
        match reply.status {
            0 => Ok(reply.text),
            1 => Err(DesktopError::Unavailable(
                "Host has not allowed clipboard access for this session".into(),
            )),
            2 => Err(DesktopError::Process("Clipboard is busy; try again".into())),
            4 => Err(DesktopError::InvalidRequest(
                "Clipboard text must be at most 8 KB and contain supported characters".into(),
            )),
            5 => Err(DesktopError::Process("Clipboard operation expired".into())),
            _ => Err(DesktopError::Unavailable(
                "Native text or clipboard operation is unavailable".into(),
            )),
        }
    }
}
pub struct PrivateOutput<R> {
    reader: BufReader<R>,
    control: Arc<UserControl>,
    public: VecDeque<u8>,
    ended: bool,
}
impl<R: Read> PrivateOutput<R> {
    pub fn new(reader: R, control: Arc<UserControl>) -> Self {
        Self {
            reader: BufReader::new(reader),
            control,
            public: VecDeque::new(),
            ended: false,
        }
    }
    fn next_line(&mut self) -> io::Result<bool> {
        let mut line = Vec::new();
        let mut oversized = false;
        let mut any = false;
        loop {
            let data = self.reader.fill_buf()?;
            if data.is_empty() {
                self.ended = true;
                self.control.close();
                break;
            }
            any = true;
            let count = data
                .iter()
                .position(|b| *b == b'\n')
                .map_or(data.len(), |i| i + 1);
            let newline = data[count - 1] == b'\n';
            if line.len() + count <= MAX_LINE {
                line.extend_from_slice(&data[..count]);
            } else {
                oversized = true;
            }
            self.reader.consume(count);
            if newline {
                break;
            }
        }
        if !oversized {
            if line.windows(7).any(|part| part == b"SNUSER_") {
                self.control.observe(&line);
            } else if line.len() <= 4096 {
                self.public.extend(line);
            }
        }
        Ok(any)
    }
}
impl<R: Read> Read for PrivateOutput<R> {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        while self.public.is_empty() && !self.ended {
            if !self.next_line()? {
                break;
            }
        }
        let count = target.len().min(self.public.len());
        for byte in &mut target[..count] {
            *byte = self.public.pop_front().unwrap_or_default();
        }
        Ok(count)
    }
}
impl<R> Drop for PrivateOutput<R> {
    fn drop(&mut self) {
        self.control.close();
    }
}
// All stdin writers use one bounded worker. Clipboard writes never hold the
// global engine-state mutex while waiting for a child process pipe to drain.
pub fn command_writer(
    mut writer: impl Write + Send + 'static,
) -> io::Result<mpsc::SyncSender<Vec<u8>>> {
    let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(16);
    thread::Builder::new()
        .name("sanser-native-commands".into())
        .spawn(move || {
            while let Ok(command) = receiver.recv() {
                if command.len() > MAX_LINE
                    || writer
                        .write_all(&command)
                        .and_then(|()| writer.flush())
                        .is_err()
                {
                    break;
                }
            }
        })?;
    Ok(sender)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_clipboard_is_explicit_and_independent_of_remote_permission()
    -> Result<(), Box<dyn std::error::Error>> {
        let control = Arc::new(UserControl::default());
        control.observe(b"SNUSER_CAP\t1\t0\n");
        assert!(control.prepare("clipboard-local-read", "").is_err());
        control.observe(b"SNUSER_CAP\t1\t0\t1\n");
        assert!(control.prepare("clipboard-read", "").is_err());
        let (_request, command) = control.prepare("clipboard-local-read", "")?;
        assert!(String::from_utf8(command)?.starts_with("clipboard-local-read\t"));
        Ok(())
    }
    #[test]
    fn private_payload_never_reaches_public_diagnostics() -> Result<(), Box<dyn std::error::Error>>
    {
        let control = Arc::new(UserControl::default());
        control.observe(b"SNUSER_CAP\t1\t1\n");
        let (request, _) = control.prepare("clipboard-read", "")?;
        let secret = "private clipboard ế😀";
        let input = format!(
            "SNV2_FIRST_FRAME decoded=1\npublic-prefixSNUSER_REPLY\t{}\t0\t{}\nSNUSER_BAD\tplaintext-secret\nSNUSER_FILE_DATA\t7\tsecret-file-content\nSNUSER_FILE_BAD\tprivate-file-name\npublic-tail\n",
            request.id,
            hex(secret)
        );
        let mut filtered = PrivateOutput::new(input.as_bytes(), control);
        let mut public = String::new();
        filtered.read_to_string(&mut public)?;
        assert_eq!(request.wait()?, secret);
        assert_eq!(public, "SNV2_FIRST_FRAME decoded=1\npublic-tail\n");
        Ok(())
    }
    #[test]
    fn invalid_or_oversized_private_records_are_always_removed()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = format!(
            "SNUSER_REPLY\tnot-a-uuid\t0\tsecret\nSNUSER_{}\nvisible\n",
            "x".repeat(MAX_LINE * 2)
        );
        let mut filtered = PrivateOutput::new(input.as_bytes(), Arc::default());
        let mut public = String::new();
        filtered.read_to_string(&mut public)?;
        assert_eq!(public, "visible\n");
        Ok(())
    }
    #[test]
    fn permission_and_size_checks_precede_all_payload_commands()
    -> Result<(), Box<dyn std::error::Error>> {
        let control = Arc::new(UserControl::default());
        assert!(control.prepare("text", "ế").is_err());
        control.observe(b"SNUSER_CAP\t1\t1\n");
        assert!(control.prepare("text", &"a".repeat(257)).is_err());
        assert!(control.prepare("text", "a\0b").is_err());
        assert!(
            control
                .prepare("clipboard-write", &"a".repeat(CLIPBOARD_BYTES + 1))
                .is_err()
        );
        let (_ticket, command) = control.prepare("text", "Tiếng Việt 😀")?;
        let command = String::from_utf8(command)?;
        assert!(!command.contains("Tiếng"));
        assert!(command.starts_with("user-text\t"));
        Ok(())
    }
    #[test]
    fn close_rejects_pending_requests_and_late_replies_cannot_target_next_child()
    -> Result<(), Box<dyn std::error::Error>> {
        let old = Arc::new(UserControl::default());
        old.observe(b"SNUSER_CAP\t1\t1\n");
        let (ticket, _) = old.prepare("clipboard-read", "")?;
        let id = ticket.id;
        old.close();
        assert!(ticket.wait().is_err());
        let next = Arc::new(UserControl::default());
        old.observe(format!("SNUSER_REPLY\t{id}\t0\t61\n").as_bytes());
        assert!(!next.capabilities().clipboard);
        assert!(next.prepare("clipboard-read", "").is_err());
        Ok(())
    }
}
