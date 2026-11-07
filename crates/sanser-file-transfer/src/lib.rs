//! Consented cross-platform file transfer. Disk work/hashing lives on this worker,
//! never on the native UDP, stdout reader or global `EngineManager` lock.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("File storage failed: {0}")]
    Storage(#[from] std::io::Error),
}
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const MAX_FILE: u64 = 25 * 1024 * 1024;
const CHUNK: usize = 4096;
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub available: bool,
    pub incoming: Option<Transfer>,
    pub outgoing: Option<Transfer>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: Uuid,
    pub name: String,
    pub size: u64,
    pub bytes: u64,
    pub state: String,
    pub error: Option<String>,
    pub saved_path: Option<String>,
}
impl Transfer {
    fn new(id: Uuid, name: String, size: u64) -> Self {
        Self {
            id,
            name,
            size,
            bytes: 0,
            state: "offered".into(),
            error: None,
            saved_path: None,
        }
    }
    fn active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "offered" | "sending" | "receiving" | "verifying"
        )
    }
}
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Offer {
        name: String,
        size: u64,
    },
    Accept {
        id: Uuid,
    },
    Chunk {
        id: Uuid,
        offset: u64,
        bytes: Vec<u8>,
    },
    Finish {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
}
enum Event {
    Capability(bool, u32),
    Packet(u32, Vec<u8>),
    Failed(u32),
    Reset,
    Source(
        u32,
        Instant,
        String,
        PathBuf,
        mpsc::SyncSender<Result<Snapshot, TransferError>>,
    ),
    Action(
        u32,
        Instant,
        Action,
        Option<PathBuf>,
        mpsc::SyncSender<Result<Snapshot, TransferError>>,
    ),
}
#[derive(Clone)]
pub struct Service {
    events: mpsc::SyncSender<Event>,
    snapshot: Arc<Mutex<Snapshot>>,
    stopped: Arc<AtomicBool>,
    overflow: Arc<AtomicBool>,
    epoch: Arc<AtomicU32>,
}
impl Service {
    /// Start an isolated file worker.
    /// # Errors
    /// Returns an I/O error if the worker thread cannot be started.
    pub fn start(pipe: mpsc::SyncSender<Vec<u8>>) -> std::io::Result<Self> {
        let (events, receiver) = mpsc::sync_channel(32);
        let service = Self {
            events,
            snapshot: Arc::default(),
            stopped: Arc::default(),
            overflow: Arc::default(),
            epoch: Arc::default(),
        };
        let state = service.clone();
        thread::Builder::new()
            .name("sanser-file-transfer".into())
            .spawn(move || {
                let mut worker = Worker::new(pipe);
                while !state.stopped.load(Ordering::Acquire) {
                    if state.overflow.swap(false, Ordering::AcqRel) {
                        worker.reset("File queue is busy; retry the transfer");
                    }
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(Event::Source(epoch, deadline, name, path, reply)) => {
                            let result = if state.stopped.load(Ordering::Acquire)
                                || epoch != worker.epoch
                                || Instant::now() > deadline
                            {
                                Err(invalid("File action expired or connection changed"))
                            } else {
                                worker
                                    .offer_source(name, path)
                                    .map(|()| worker.view.clone())
                            };
                            let _ = reply.try_send(result);
                        }
                        Ok(Event::Action(epoch, deadline, action, directory, reply)) => {
                            let result = if state.stopped.load(Ordering::Acquire)
                                || epoch != worker.epoch
                                || Instant::now() > deadline
                            {
                                Err(invalid("File action expired or connection changed"))
                            } else {
                                worker
                                    .action(action, directory)
                                    .map(|()| worker.view.clone())
                            };
                            let _ = reply.try_send(result);
                        }
                        Ok(Event::Capability(value, epoch)) => {
                            worker.epoch = epoch;
                            worker.view.available = value;
                            if !value {
                                worker.reset("File connection ended");
                            }
                        }
                        Ok(Event::Packet(epoch, bytes)) => {
                            if epoch == worker.epoch && !state.stopped.load(Ordering::Acquire) {
                                worker.receive(&bytes);
                            }
                        }
                        Ok(Event::Failed(epoch)) => {
                            if epoch == worker.epoch {
                                worker.reset("Native file queue is busy; retry the transfer");
                            }
                        }
                        Ok(Event::Reset) => {
                            worker.view.available = false;
                            worker.reset("Connection changed; transfer cancelled");
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    worker.expire();
                    worker.pump_source();
                    if let Ok(mut snapshot) = state.snapshot.lock() {
                        *snapshot = worker.view.clone();
                    }
                }
                worker.reset("Session ended; transfer cancelled");
                worker.view.available = false;
                if let Ok(mut snapshot) = state.snapshot.lock() {
                    *snapshot = worker.view.clone();
                }
            })?;
        Ok(service)
    }
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.snapshot.lock().map(|v| v.clone()).unwrap_or_default();
        if self.stopped.load(Ordering::Acquire) {
            snapshot.available = false;
        }
        snapshot
    }
    pub fn close(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub fn record(&self, line: &str) {
        let event = match line.trim_end_matches(['\r', '\n']) {
            "SNUSER_FILE_RESET" => {
                self.epoch.store(0, Ordering::Release);
                Some(Event::Reset)
            }
            value if value.starts_with("SNUSER_FILE_FAIL\t") => value
                .strip_prefix("SNUSER_FILE_FAIL\t")
                .and_then(|v| v.parse().ok())
                .map(Event::Failed),
            value if value.starts_with("SNUSER_FILE_CAP\t") => {
                let fields: Vec<_> = value.split('\t').collect();
                if fields.len() == 3 && matches!(fields[1], "0" | "1") {
                    fields[2].parse().ok().map(|epoch| {
                        self.epoch.store(epoch, Ordering::Release);
                        Event::Capability(fields[1] == "1", epoch)
                    })
                } else {
                    None
                }
            }
            value => value
                .strip_prefix("SNUSER_FILE_DATA\t")
                .and_then(|v| v.split_once('\t'))
                .and_then(|(epoch, data)| {
                    Some(Event::Packet(epoch.parse().ok()?, decode_hex(data)?))
                }),
        };
        if let Some(event) = event
            && self.events.try_send(event).is_err()
        {
            self.overflow.store(true, Ordering::Release);
        }
    }
    /// Apply an explicit UI action to this service's authenticated generation.
    /// # Errors
    /// Rejects invalid, expired, busy or unpermitted file actions and I/O errors.
    pub fn action(
        &self,
        action: Action,
        directory: Option<PathBuf>,
    ) -> Result<Snapshot, TransferError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(invalid("File session ended"));
        }
        // Bound a JS payload before it enters the worker's queue.
        if let Action::Offer { name, size } = &action
            && (!safe_name(name) || *size > MAX_FILE)
        {
            return Err(invalid(
                "Choose a safe file name and a file no larger than 25 MB",
            ));
        }
        if let Action::Chunk { bytes, .. } = &action
            && bytes.len() > CHUNK
        {
            return Err(invalid("File chunk is too large"));
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.events
            .try_send(Event::Action(
                self.epoch.load(Ordering::Acquire),
                Instant::now() + Duration::from_secs(4),
                action,
                directory,
                reply,
            ))
            .map_err(|_| invalid("File worker is busy"))?;
        result
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| invalid("File operation timed out"))?
    }
    /// The platform picker must first copy a user-selected regular file into an
    /// owned private workspace. This API is not exposed to arbitrary UI paths.
    /// The owned cache copy is removed on completion/cancellation; never pass
    /// the user's original file here.
    /// # Errors
    /// Rejects closed sessions, invalid sources, full queues and storage errors.
    pub fn offer_source(&self, name: String, path: PathBuf) -> Result<Snapshot, TransferError> {
        if !safe_name(&name) || self.stopped.load(Ordering::Acquire) {
            return Err(invalid("Invalid file selection"));
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.events
            .try_send(Event::Source(
                self.epoch.load(Ordering::Acquire),
                Instant::now() + Duration::from_secs(4),
                name,
                path,
                reply,
            ))
            .map_err(|_| invalid("File worker is busy"))?;
        result
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| invalid("File operation timed out"))?
    }
}
fn invalid(message: &str) -> TransferError {
    TransferError::InvalidRequest(message.into())
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 180
        && name.trim() == name
        && !name.ends_with('.')
        && name != "."
        && name != ".."
        && !name.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                || "/\\:<>\"|?*".contains(c)
        })
}
fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() > 16384 || !value.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |c| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|p| Some((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}
fn packet(op: u8, id: Uuid, body: &[u8]) -> Vec<u8> {
    let mut bytes = b"SFT1".to_vec();
    bytes.push(op);
    bytes.extend(id.as_bytes());
    bytes.extend(body);
    bytes
}
fn number(bytes: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.try_into().ok()?))
}
struct Upload {
    hash: Sha256,
    pending: Option<u64>,
    touched: Instant,
    source: Option<Source>,
}
struct Source {
    file: Option<File>,
    path: PathBuf,
}
impl Drop for Source {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
struct Download {
    hash: Sha256,
    file: Option<File>,
    temporary: PathBuf,
    destination: PathBuf,
    touched: Instant,
}
impl Drop for Download {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.temporary);
    }
}
struct Worker {
    epoch: u32,
    view: Snapshot,
    pipe: mpsc::SyncSender<Vec<u8>>,
    upload: Option<Upload>,
    download: Option<Download>,
    offered_at: Option<Instant>,
    retired: VecDeque<Uuid>,
}
impl Worker {
    fn new(pipe: mpsc::SyncSender<Vec<u8>>) -> Self {
        Self {
            epoch: 0,
            view: Snapshot::default(),
            pipe,
            upload: None,
            download: None,
            offered_at: None,
            retired: VecDeque::new(),
        }
    }
    fn send(&self, op: u8, id: Uuid, body: &[u8]) -> Result<(), TransferError> {
        let command = format!(
            "file-send\t{}\t{}\n",
            Uuid::new_v4(),
            encode_hex(
                &[
                    self.epoch.to_be_bytes().as_slice(),
                    packet(op, id, body).as_slice()
                ]
                .concat()
            )
        );
        self.pipe
            .try_send(command.into_bytes())
            .map_err(|_| invalid("Native file queue is busy or closed"))
    }
    fn retire(&mut self, id: Uuid) {
        self.retired.push_back(id);
        if self.retired.len() > 32 {
            self.retired.pop_front();
        }
    }
    fn cancel(&mut self, id: Uuid, reason: &str) {
        let mut found = false;
        for transfer in [&mut self.view.incoming, &mut self.view.outgoing]
            .into_iter()
            .flatten()
        {
            if transfer.id == id && transfer.active() {
                transfer.state = "cancelled".into();
                transfer.error = Some(reason.into());
                found = true;
            }
        }
        if found {
            if self.view.incoming.as_ref().is_some_and(|v| v.id == id) {
                self.download = None;
                self.offered_at = None;
            }
            if self.view.outgoing.as_ref().is_some_and(|v| v.id == id) {
                self.upload = None;
            }
            self.retire(id);
            let _ = self.send(7, id, &[]);
        }
    }
    fn reset(&mut self, reason: &str) {
        let ids: Vec<_> = [&self.view.incoming, &self.view.outgoing]
            .into_iter()
            .flatten()
            .filter(|v| v.active())
            .map(|v| v.id)
            .collect();
        for id in ids {
            self.cancel(id, reason);
        }
    }
    fn action(&mut self, action: Action, directory: Option<PathBuf>) -> Result<(), TransferError> {
        if let Action::Cancel { id } = action {
            self.cancel(id, "Transfer cancelled");
            return Ok(());
        }
        if !self.view.available {
            return Err(invalid("File transfer is unavailable for this session"));
        }
        match action {
            Action::Offer { name, size } => self.offer(name, size),
            Action::Accept { id } => self.accept(
                id,
                &directory.ok_or_else(|| invalid("Downloads directory is unavailable"))?,
            ),
            Action::Chunk { id, offset, bytes } => self.chunk(id, offset, &bytes),
            Action::Finish { id } => self.finish(id),
            Action::Cancel { .. } => Ok(()),
        }
    }
    fn offer(&mut self, name: String, size: u64) -> Result<(), TransferError> {
        if !safe_name(&name) || size > MAX_FILE {
            return Err(invalid(
                "Choose a safe file name and a file no larger than 25 MB",
            ));
        }
        if self.view.outgoing.as_ref().is_some_and(Transfer::active) {
            return Err(invalid("An upload is already active"));
        }
        let id = Uuid::new_v4();
        let mut body = size.to_be_bytes().to_vec();
        body.extend(name.as_bytes());
        self.send(1, id, &body)?;
        self.view.outgoing = Some(Transfer::new(id, name, size));
        self.upload = Some(Upload {
            hash: Sha256::new(),
            pending: None,
            touched: Instant::now(),
            source: None,
        });
        Ok(())
    }
    fn offer_source(&mut self, name: String, path: PathBuf) -> Result<(), TransferError> {
        if !self.view.available {
            return Err(invalid("File connection is unavailable"));
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_FILE {
            return Err(invalid("Choose a regular file no larger than 25 MB"));
        }
        let file = File::open(&path)?;
        self.offer(name, metadata.len())?;
        if let Some(upload) = self.upload.as_mut() {
            upload.source = Some(Source {
                file: Some(file),
                path,
            });
        }
        Ok(())
    }
    fn pump_source(&mut self) {
        let Some(view) = self.view.outgoing.as_ref().filter(|v| v.state == "sending") else {
            return;
        };
        let id = view.id;
        let offset = view.bytes;
        let size = view.size;
        let Some(upload) = self
            .upload
            .as_mut()
            .filter(|v| v.pending.is_none() && v.source.is_some())
        else {
            return;
        };
        let result = if offset == size {
            self.finish(id)
        } else {
            let mut bytes =
                vec![0; usize::try_from((size - offset).min(CHUNK as u64)).unwrap_or(CHUNK)];
            let read = upload
                .source
                .as_mut()
                .and_then(|v| v.file.as_mut())
                .ok_or_else(|| invalid("Selected file is closed"))
                .and_then(|file| {
                    file.seek(SeekFrom::Start(offset))?;
                    file.read_exact(&mut bytes)?;
                    Ok(())
                });
            read.and_then(|()| self.chunk(id, offset, &bytes))
        };
        if result.is_err() {
            self.cancel(id, "Selected file could not be read or queued");
        }
    }
    fn accept(&mut self, id: Uuid, directory: &Path) -> Result<(), TransferError> {
        let transfer = self
            .view
            .incoming
            .as_ref()
            .filter(|v| v.id == id && v.state == "offered")
            .ok_or_else(|| invalid("File invitation is no longer pending"))?;
        // Accept is explicit; merely receiving an offer creates no directories/files.
        let directory = directory.join("Sanser");
        fs::create_dir_all(&directory)?;
        if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            return Err(invalid(
                "Choose a Downloads directory without a Sanser symlink",
            ));
        }
        let temporary = directory.join(format!(".{}.part", Uuid::new_v4()));
        let destination = directory.join(format!("{}-{}", Uuid::new_v4(), transfer.name));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary)?;
        let download = Download {
            hash: Sha256::new(),
            file: Some(file),
            temporary,
            destination,
            touched: Instant::now(),
        };
        self.send(2, id, &[])?;
        self.download = Some(download);
        if let Some(view) = self.view.incoming.as_mut() {
            view.state = "receiving".into();
        }
        self.offered_at = None;
        Ok(())
    }
    fn chunk(&mut self, id: Uuid, offset: u64, bytes: &[u8]) -> Result<(), TransferError> {
        let transfer = self
            .view
            .outgoing
            .as_ref()
            .filter(|v| v.id == id && v.state == "sending")
            .ok_or_else(|| invalid("Upload has not been accepted"))?;
        let upload = self
            .upload
            .as_ref()
            .ok_or_else(|| invalid("Upload is closed"))?;
        if bytes.is_empty()
            || bytes.len() > CHUNK
            || offset != transfer.bytes
            || upload.pending.is_some()
            || offset + bytes.len() as u64 > transfer.size
        {
            return Err(invalid("Unexpected file chunk or offset"));
        }
        let end = offset + bytes.len() as u64;
        let mut body = offset.to_be_bytes().to_vec();
        body.extend(bytes);
        self.send(3, id, &body)?;
        if let Some(upload) = self.upload.as_mut() {
            upload.hash.update(bytes);
            upload.pending = Some(end);
            upload.touched = Instant::now();
        }
        Ok(())
    }
    fn finish(&mut self, id: Uuid) -> Result<(), TransferError> {
        let transfer = self
            .view
            .outgoing
            .as_ref()
            .filter(|v| v.id == id && v.state == "sending" && v.bytes == v.size)
            .ok_or_else(|| invalid("Upload still has missing bytes"))?;
        let upload = self
            .upload
            .as_ref()
            .filter(|v| v.pending.is_none())
            .ok_or_else(|| invalid("Upload is waiting for acknowledgement"))?;
        self.send(5, transfer.id, &upload.hash.clone().finalize())?;
        if let Some(view) = self.view.outgoing.as_mut() {
            view.state = "verifying".into();
        }
        if let Some(upload) = self.upload.as_mut() {
            upload.touched = Instant::now();
        }
        Ok(())
    }
    fn receive(&mut self, bytes: &[u8]) {
        if !self.view.available
            || bytes.len() < 21
            || bytes.len() > CHUNK + 29
            || &bytes[..4] != b"SFT1"
        {
            return;
        }
        let Ok(id) = Uuid::from_slice(&bytes[5..21]) else {
            return;
        };
        if id.is_nil() {
            return;
        }
        let body = &bytes[21..];
        let result = match bytes[4] {
            1 => {
                self.invite(id, body);
                Ok(())
            }
            2 if body.is_empty() => {
                self.accepted(id);
                Ok(())
            }
            3 => self.receive_chunk(id, body),
            4 => {
                self.ack(id, body);
                Ok(())
            }
            5 if body.len() == 32 => self.complete(id, body),
            6 if body.is_empty() => {
                self.confirmed(id);
                Ok(())
            }
            7 if body.is_empty() => {
                self.cancel(id, "Other computer cancelled the transfer");
                Ok(())
            }
            _ => Ok(()),
        };
        if result.is_err() {
            self.cancel(id, "File could not be saved or verified");
        }
    }
    fn invite(&mut self, id: Uuid, body: &[u8]) {
        if self.retired.contains(&id)
            || self.view.incoming.as_ref().is_some_and(|v| v.id == id)
            || self.view.outgoing.as_ref().is_some_and(|v| v.id == id)
        {
            return;
        }
        let valid = body
            .get(..8)
            .and_then(number)
            .zip(body.get(8..).and_then(|v| std::str::from_utf8(v).ok()));
        if let Some((size, name)) = valid
            && size <= MAX_FILE
            && safe_name(name)
            && !self.view.incoming.as_ref().is_some_and(Transfer::active)
        {
            self.view.incoming = Some(Transfer::new(id, name.into(), size));
            self.offered_at = Some(Instant::now());
        } else {
            let _ = self.send(7, id, &[]);
        }
    }
    fn accepted(&mut self, id: Uuid) {
        if let Some(view) = self
            .view
            .outgoing
            .as_mut()
            .filter(|v| v.id == id && v.state == "offered")
        {
            view.state = "sending".into();
            if let Some(upload) = self.upload.as_mut() {
                upload.touched = Instant::now();
            }
        }
    }
    fn ack(&mut self, id: Uuid, body: &[u8]) {
        let Some(end) = number(body) else {
            return;
        };
        if let (Some(view), Some(upload)) = (
            self.view
                .outgoing
                .as_mut()
                .filter(|v| v.id == id && v.state == "sending"),
            self.upload.as_mut(),
        ) && upload.pending == Some(end)
        {
            view.bytes = end;
            upload.pending = None;
            upload.touched = Instant::now();
        }
    }
    fn receive_chunk(&mut self, id: Uuid, body: &[u8]) -> Result<(), TransferError> {
        let Some(view) = self
            .view
            .incoming
            .as_mut()
            .filter(|v| v.id == id && v.state == "receiving")
        else {
            return Ok(());
        };
        let download = self
            .download
            .as_mut()
            .ok_or_else(|| invalid("Download is closed"))?;
        let offset = body
            .get(..8)
            .and_then(number)
            .ok_or_else(|| invalid("Invalid file offset"))?;
        let data = &body[8..];
        if data.is_empty()
            || data.len() > CHUNK
            || offset != view.bytes
            || offset + data.len() as u64 > view.size
        {
            return Err(invalid("Unexpected download offset"));
        }
        download
            .file
            .as_mut()
            .ok_or_else(|| invalid("Download is closed"))?
            .write_all(data)?;
        download.hash.update(data);
        view.bytes += data.len() as u64;
        download.touched = Instant::now();
        let end = view.bytes;
        self.send(4, id, &end.to_be_bytes())
    }
    fn complete(&mut self, id: Uuid, digest: &[u8]) -> Result<(), TransferError> {
        let Some(view) = self
            .view
            .incoming
            .as_mut()
            .filter(|v| v.id == id && v.state == "receiving")
        else {
            return Ok(());
        };
        let download = self
            .download
            .as_mut()
            .ok_or_else(|| invalid("Download is closed"))?;
        if view.bytes != view.size || download.hash.clone().finalize().as_slice() != digest {
            return Err(invalid("File integrity check failed"));
        }
        download
            .file
            .as_ref()
            .ok_or_else(|| invalid("Download is closed"))?
            .sync_all()?;
        download.file.take();
        // Atomic publication with no overwrite. Failure leaves only our owned
        // partial file, removed by Drop; never touch a pre-existing destination.
        fs::hard_link(&download.temporary, &download.destination)?;
        view.saved_path = Some(download.destination.to_string_lossy().into_owned());
        view.state = "completed".into();
        self.download = None;
        self.retire(id);
        self.send(6, id, &[])
    }
    fn confirmed(&mut self, id: Uuid) {
        if let Some(view) = self
            .view
            .outgoing
            .as_mut()
            .filter(|v| v.id == id && v.state == "verifying")
        {
            view.state = "completed".into();
            self.upload = None;
            self.retire(id);
        }
    }
    fn expire(&mut self) {
        let now = Instant::now();
        if let Some(view) = &self.view.outgoing
            && view.active()
            && self.upload.as_ref().is_some_and(|v| {
                now.duration_since(v.touched)
                    > Duration::from_secs(if view.state == "offered" { 90 } else { 30 })
            })
        {
            let id = view.id;
            self.cancel(id, "File transfer timed out");
        }
        if let Some(view) = &self.view.incoming
            && view.active()
            && (self
                .offered_at
                .is_some_and(|v| now.duration_since(v) > Duration::from_secs(90))
                || self
                    .download
                    .as_ref()
                    .is_some_and(|v| now.duration_since(v.touched) > Duration::from_secs(30)))
        {
            let id = view.id;
            self.cancel(id, "File transfer timed out");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Result<Self, std::io::Error> {
            let path = std::env::temp_dir().join(format!("sanser-file-test-{}", Uuid::new_v4()));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn worker() -> (Worker, mpsc::Receiver<Vec<u8>>) {
        let (pipe, receiver) = mpsc::sync_channel(32);
        let mut worker = Worker::new(pipe);
        worker.view.available = true;
        (worker, receiver)
    }
    fn forward(
        receiver: &mpsc::Receiver<Vec<u8>>,
        worker: &mut Worker,
    ) -> Result<(), Box<dyn std::error::Error>> {
        while let Ok(bytes) = receiver.try_recv() {
            let line = String::from_utf8(bytes)?;
            let data = line
                .trim_end()
                .split('\t')
                .nth(2)
                .and_then(decode_hex)
                .ok_or("file frame")?;
            worker.receive(&data[4..]);
        }
        Ok(())
    }
    #[test]
    fn consent_chunk_ack_and_integrity() -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (mut sender, outgoing) = worker();
        let (mut receiver, incoming) = worker();
        let content = "Tiếng Việt 😀\n".repeat(700).into_bytes();
        sender.action(
            Action::Offer {
                name: "test tiếng Việt.txt".into(),
                size: content.len() as u64,
            },
            None,
        )?;
        let id = sender.view.outgoing.as_ref().ok_or("upload")?.id;
        forward(&outgoing, &mut receiver)?;
        assert_eq!(
            receiver.view.incoming.as_ref().ok_or("offer")?.state,
            "offered"
        );
        assert!(
            !directory.0.join("Sanser").exists(),
            "no file created before consent"
        );
        assert!(sender.chunk(id, 0, &content[..100]).is_err());
        receiver.action(Action::Accept { id }, Some(directory.0.clone()))?;
        forward(&incoming, &mut sender)?;
        let mut offset = 0;
        for chunk in content.chunks(CHUNK) {
            sender.chunk(id, offset, chunk)?;
            assert!(
                sender.chunk(id, offset, chunk).is_err(),
                "only one application chunk in flight"
            );
            assert_eq!(sender.view.outgoing.as_ref().ok_or("upload")?.bytes, offset);
            forward(&outgoing, &mut receiver)?;
            forward(&incoming, &mut sender)?;
            offset += chunk.len() as u64;
            assert_eq!(sender.view.outgoing.as_ref().ok_or("upload")?.bytes, offset);
        }
        sender.finish(id)?;
        forward(&outgoing, &mut receiver)?;
        forward(&incoming, &mut sender)?;
        let view = receiver.view.incoming.as_ref().ok_or("download")?;
        assert_eq!(view.state, "completed");
        assert_eq!(
            sender.view.outgoing.as_ref().ok_or("upload")?.state,
            "completed"
        );
        assert_eq!(
            fs::read(view.saved_path.as_ref().ok_or("saved path")?)?,
            content
        );
        assert!(fs::read_dir(directory.0.join("Sanser"))?.all(|v| {
            v.is_ok_and(|entry| entry.path().extension().is_none_or(|ext| ext != "part"))
        }));
        Ok(())
    }
    #[test]
    fn reverse_transfer_and_empty_file() -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (mut a, a_wire) = worker();
        let (mut b, b_wire) = worker();
        b.offer("empty.txt".into(), 0)?;
        let id = b.view.outgoing.as_ref().ok_or("upload")?.id;
        forward(&b_wire, &mut a)?;
        a.accept(id, &directory.0)?;
        forward(&a_wire, &mut b)?;
        b.finish(id)?;
        forward(&b_wire, &mut a)?;
        forward(&a_wire, &mut b)?;
        assert_eq!(b.view.outgoing.as_ref().ok_or("upload")?.state, "completed");
        assert_eq!(
            fs::metadata(
                a.view
                    .incoming
                    .as_ref()
                    .and_then(|v| v.saved_path.as_ref())
                    .ok_or("saved")?
            )?
            .len(),
            0
        );
        Ok(())
    }
    #[test]
    fn filenames_sizes_and_unsolicited_chunks_cannot_write()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (mut receiver, wire) = worker();
        for name in [
            "../escape",
            "a/b",
            "a\\b",
            "C:bad",
            "bad\u{202e}name",
            "bad\0name",
            "a\n",
            "trailing.",
            " x",
            "",
        ] {
            assert!(!safe_name(name));
            let mut body = 1_u64.to_be_bytes().to_vec();
            body.extend(name.as_bytes());
            receiver.receive(&packet(1, Uuid::new_v4(), &body));
            assert!(receiver.view.incoming.is_none());
            while wire.try_recv().is_ok() {}
        }
        assert!(receiver.offer("large.txt".into(), MAX_FILE + 1).is_err());
        assert!(
            receiver
                .action(
                    Action::Chunk {
                        id: Uuid::new_v4(),
                        offset: 0,
                        bytes: vec![1]
                    },
                    None
                )
                .is_err()
        );
        receiver.receive(&packet(3, Uuid::new_v4(), &[0; 30]));
        assert!(!directory.0.join("Sanser").exists());
        Ok(())
    }
    fn accepted_download(
        receiver: &mut Worker,
        size: u64,
        directory: &Directory,
    ) -> Result<Uuid, TransferError> {
        let id = Uuid::new_v4();
        let mut body = size.to_be_bytes().to_vec();
        body.extend(b"sample.txt");
        receiver.receive(&packet(1, id, &body));
        receiver.accept(id, &directory.0)?;
        Ok(id)
    }
    #[test]
    fn cancellation_hash_failure_and_route_reset_remove_partials()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (mut receiver, _wire) = worker();
        for op in [7, 5, 0] {
            let id = accepted_download(&mut receiver, 1, &directory)?;
            let partial = receiver
                .download
                .as_ref()
                .ok_or("partial")?
                .temporary
                .clone();
            let mut data = 0_u64.to_be_bytes().to_vec();
            data.push(42);
            receiver.receive(&packet(3, id, &data));
            if op == 0 {
                receiver.reset("route changed");
            } else {
                receiver.receive(&packet(op, id, if op == 5 { &[0; 32] } else { &[] }));
            }
            assert!(!partial.exists());
            assert_eq!(
                receiver.view.incoming.as_ref().ok_or("download")?.state,
                "cancelled"
            );
        }
        Ok(())
    }
    #[test]
    fn existing_destination_is_never_overwritten() -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (mut receiver, _wire) = worker();
        let id = accepted_download(&mut receiver, 0, &directory)?;
        let download = receiver.download.as_ref().ok_or("download")?;
        let destination = download.destination.clone();
        let partial = download.temporary.clone();
        fs::write(&destination, "keep existing")?;
        receiver.receive(&packet(5, id, &Sha256::digest([])));
        assert_eq!(fs::read_to_string(destination)?, "keep existing");
        assert!(!partial.exists());
        assert_eq!(
            receiver.view.incoming.as_ref().ok_or("download")?.state,
            "cancelled"
        );
        Ok(())
    }
    #[test]
    fn malformed_order_bounded_queue_expiry_and_session_close()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (mut worker, _wire) = worker();
        let id = accepted_download(&mut worker, 2, &directory)?;
        let mut data = 1_u64.to_be_bytes().to_vec();
        data.push(42);
        worker.receive(&packet(3, id, &data));
        assert!(worker.download.is_none());
        worker.offer("slow.txt".into(), 1)?;
        if let Some(upload) = worker.upload.as_mut() {
            upload.touched = Instant::now()
                .checked_sub(Duration::from_secs(91))
                .ok_or("test clock")?;
        }
        worker.expire();
        assert!(worker.upload.is_none());
        let (pipe, _) = mpsc::sync_channel(1);
        let service = Service::start(pipe)?;
        service.close();
        assert!(!service.snapshot().available);
        assert!(
            service
                .action(
                    Action::Offer {
                        name: "late.txt".into(),
                        size: 0
                    },
                    None
                )
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn native_generation_guards_stale_messages_and_close_cleans_download()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::new()?;
        let (pipe, wire) = mpsc::sync_channel(32);
        let service = Service::start(pipe)?;
        let wait = |condition: &dyn Fn() -> bool| -> Result<(), Box<dyn std::error::Error>> {
            let until = Instant::now() + Duration::from_secs(2);
            while !condition() && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            if condition() {
                Ok(())
            } else {
                Err("file worker did not reach expected state".into())
            }
        };
        service.record("SNUSER_FILE_CAP\t1\t7");
        wait(&|| service.snapshot().available)?;
        let offered = service.action(
            Action::Offer {
                name: "scope.txt".into(),
                size: 1,
            },
            None,
        )?;
        assert!(offered.outgoing.is_some());
        let line = String::from_utf8(wire.recv_timeout(Duration::from_secs(1))?)?;
        let body = line
            .trim_end()
            .split('\t')
            .nth(2)
            .and_then(decode_hex)
            .ok_or("wire frame")?;
        assert_eq!(&body[..4], &7_u32.to_be_bytes());
        service.record("SNUSER_FILE_RESET");
        service.record("SNUSER_FILE_CAP\t1\t8");
        wait(&|| {
            service
                .snapshot()
                .outgoing
                .is_some_and(|v| v.state == "cancelled")
        })?;
        let id = Uuid::new_v4();
        let mut body = 0_u64.to_be_bytes().to_vec();
        body.extend(b"receive.txt");
        let offer = encode_hex(&packet(1, id, &body));
        let old_offer = encode_hex(&packet(1, Uuid::new_v4(), &body));
        service.record(&format!("SNUSER_FILE_DATA\t7\t{old_offer}"));
        service.record(&format!("SNUSER_FILE_DATA\t8\t{offer}"));
        wait(&|| service.snapshot().incoming.is_some_and(|v| v.id == id))?;
        service.action(Action::Accept { id }, Some(directory.0.clone()))?;
        let paths: Vec<_> =
            fs::read_dir(directory.0.join("Sanser"))?.collect::<Result<Vec<_>, _>>()?;
        assert_eq!(paths.len(), 1);
        service.close();
        wait(&|| !paths[0].path().exists())?;
        assert!(!service.snapshot().available);
        Ok(())
    }
}
