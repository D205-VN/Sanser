//! Mobile adapter for the exact same consent/integrity worker as desktop.
use sanser_file_transfer::{Action, Service};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
use uuid::Uuid;
struct Bridge {
    service: Service,
    root: PathBuf,
    output: Mutex<mpsc::Receiver<Vec<u8>>>,
    state: Mutex<(u32, bool)>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.service.close();
        let root = self.root.clone();
        let _ = std::thread::Builder::new()
            .name("sanser-selected-file-cleanup".into())
            .spawn(move || {
                if let Ok(entries) = std::fs::read_dir(root) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        let Some(name) = name.to_str() else {
                            continue;
                        };
                        let Some(id) = name
                            .strip_prefix("source-")
                            .map(|v| v.strip_suffix(".tmp").unwrap_or(v))
                        else {
                            continue;
                        };
                        if Uuid::parse_str(id).is_ok()
                            && entry.file_type().is_ok_and(|v| v.is_file())
                        {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            });
    }
}
static BRIDGES: OnceLock<Mutex<HashMap<u64, Arc<Bridge>>>> = OnceLock::new();
static SERIAL: AtomicU64 = AtomicU64::new(1);
fn bridges() -> &'static Mutex<HashMap<u64, Arc<Bridge>>> {
    BRIDGES.get_or_init(Mutex::default)
}
fn bridge(id: u64) -> Option<Arc<Bridge>> {
    bridges().lock().ok()?.get(&id).cloned()
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    out
}
fn decode(value: &[u8]) -> Option<Vec<u8>> {
    if value.len() > 16384 || !value.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |b| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    value
        .chunks_exact(2)
        .map(|p| Some((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect()
}
#[derive(Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Request {
    Status,
    Offer { source_id: Uuid, name: String },
    Accept { id: Uuid },
    Cancel { id: Uuid },
}
fn request(b: &Bridge, r: Request) -> Result<Value, String> {
    let result = match r {
        Request::Status => {
            return serde_json::to_value(b.service.snapshot())
                .map_err(|_| "File state unavailable".into());
        }
        Request::Offer { source_id, name } => b
            .service
            .offer_source(name, b.root.join(format!("source-{source_id}"))),
        Request::Accept { id } => b
            .service
            .action(Action::Accept { id }, Some(b.root.join("received"))),
        Request::Cancel { id } => b.service.action(Action::Cancel { id }, None),
    }
    .map_err(|e| e.to_string())?;
    serde_json::to_value(result).map_err(|_| "File state unavailable".into())
}
/// # Safety
/// `root` is a valid NUL-terminated UTF-8 path to a newly allocated private app workspace.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sanser_mobile_files_create(root: *const c_char) -> u64 {
    std::panic::catch_unwind(|| {
        if root.is_null() {
            return 0;
        }
        let raw = unsafe { CStr::from_ptr(root) }.to_bytes();
        if raw.len() > 4096 {
            return 0;
        }
        let Ok(text) = std::str::from_utf8(raw) else {
            return 0;
        };
        let root = PathBuf::from(text);
        if !root.is_absolute() || std::fs::create_dir_all(&root).is_err() {
            return 0;
        }
        let (sender, output) = mpsc::sync_channel(16);
        let Ok(service) = Service::start(sender) else {
            return 0;
        };
        let Ok(mut map) = bridges().lock() else {
            service.close();
            return 0;
        };
        if map.len() >= 4 {
            service.close();
            return 0;
        }
        let id = SERIAL.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            service.close();
            return 0;
        }
        map.insert(
            id,
            Arc::new(Bridge {
                service,
                root,
                output: Mutex::new(output),
                state: Mutex::new((0, false)),
            }),
        );
        id
    })
    .unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn sanser_mobile_files_destroy(id: u64) {
    if let Ok(mut map) = bridges().lock()
        && let Some(b) = map.remove(&id)
    {
        b.service.close();
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn sanser_mobile_files_state(id: u64, epoch: u32, available: i32) {
    if let Some(b) = bridge(id)
        && let Ok(mut state) = b.state.lock()
    {
        let available = available != 0;
        if *state != (epoch, available) {
            if state.0 != epoch {
                b.service.record("SNUSER_FILE_RESET");
            }
            b.service.record(&format!(
                "SNUSER_FILE_CAP\t{}\t{epoch}",
                u8::from(available)
            ));
            *state = (epoch, available);
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn sanser_mobile_files_failed(id: u64, epoch: u32) {
    if let Some(b) = bridge(id) {
        b.service.record(&format!("SNUSER_FILE_FAIL\t{epoch}"));
    }
}
/// # Safety
/// `data` contains `size` readable bytes for this call. No ownership is retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sanser_mobile_files_receive(
    id: u64,
    epoch: u32,
    data: *const u8,
    size: usize,
) {
    if data.is_null() || !(21..=8192).contains(&size) {
        return;
    }
    if let Some(b) = bridge(id) {
        let bytes = unsafe { std::slice::from_raw_parts(data, size) };
        b.service
            .record(&format!("SNUSER_FILE_DATA\t{epoch}\t{}", hex(bytes)));
    }
}
/// # Safety
/// `output` points to at least `capacity` writable bytes. A full 8192-byte buffer
/// is required; output is generation:u32 BE followed by an SFT1 application body.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sanser_mobile_files_next(
    id: u64,
    output: *mut u8,
    capacity: usize,
) -> usize {
    if output.is_null() || capacity < 8192 {
        return 0;
    }
    let Some(b) = bridge(id) else {
        return 0;
    };
    let Ok(receiver) = b.output.lock() else {
        return 0;
    };
    let Ok(command) = receiver.try_recv() else {
        return 0;
    };
    let Some(raw) = command.split(|v| *v == b'\t').nth(2) else {
        return 0;
    };
    let Some(raw) = raw.strip_suffix(b"\n") else {
        return 0;
    };
    let Some(bytes) = decode(raw) else {
        return 0;
    };
    if bytes.len() > capacity {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
    }
    bytes.len()
}
/// # Safety
/// `text` is valid NUL-terminated JSON. Free the result with sanser_mobile_p2p_free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sanser_mobile_files_request(id: u64, text: *const c_char) -> *mut c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if text.is_null() {
            return Err("Missing file action".to_string());
        }
        let raw = unsafe { CStr::from_ptr(text) }.to_bytes();
        if raw.len() > 4096 {
            return Err("File action too large".into());
        }
        let b = bridge(id).ok_or("File session ended")?;
        let r = serde_json::from_slice(raw).map_err(|_| "Invalid file action")?;
        request(&b, r)
    }))
    .unwrap_or_else(|_| Err("File operation failed".into()));
    let value = match result {
        Ok(v) => json!({"ok":true,"result":v}),
        Err(e) => json!({"ok":false,"error":e}),
    };
    CString::new(value.to_string()).map_or(std::ptr::null_mut(), CString::into_raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    struct Scope {
        root: PathBuf,
        handle: u64,
    }
    impl Scope {
        fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let root = std::env::temp_dir().join(format!("sanser-mobile-files-{}", Uuid::new_v4()));
            let text = CString::new(root.to_string_lossy().as_bytes())?;
            let handle = unsafe { sanser_mobile_files_create(text.as_ptr()) };
            assert_ne!(handle, 0);
            Ok(Self { root, handle })
        }
        fn call(&self, value: Value) -> Result<Value, Box<dyn std::error::Error>> {
            let text = CString::new(value.to_string())?;
            let response = unsafe { sanser_mobile_files_request(self.handle, text.as_ptr()) };
            assert!(!response.is_null());
            let answer: Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(response) }.to_bytes())?;
            unsafe { crate::sanser_mobile_p2p_free(response) };
            Ok(answer)
        }
        fn status(&self) -> Result<Value, Box<dyn std::error::Error>> {
            Ok(self.call(json!({"operation":"status"}))?["result"].clone())
        }
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            sanser_mobile_files_destroy(self.handle);
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn route(from: &Scope, to: &Scope, epoch: u32) {
        let mut bytes = [0_u8; 8192];
        for _ in 0..4 {
            let n =
                unsafe { sanser_mobile_files_next(from.handle, bytes.as_mut_ptr(), bytes.len()) };
            if n < 25 {
                break;
            };
            unsafe { sanser_mobile_files_receive(to.handle, epoch, bytes[4..n].as_ptr(), n - 4) };
        }
    }
    #[test]
    fn selected_file_streams_through_c_abi_with_shared_consent_and_integrity()
    -> Result<(), Box<dyn std::error::Error>> {
        let a = Scope::new()?;
        let b = Scope::new()?;
        sanser_mobile_files_state(a.handle, 12, 1);
        sanser_mobile_files_state(b.handle, 34, 1);
        let id = Uuid::new_v4();
        let source = a.root.join(format!("source-{id}"));
        let content = "Tiếng Việt 😀\n".repeat(1200);
        std::fs::write(&source, &content)?;
        let answer =
            a.call(json!({"operation":"offer","sourceId":id,"name":"tiếng Việt 😀.txt"}))?;
        assert_eq!(answer["ok"], true);
        assert!(
            a.call(json!({"operation":"offer","sourceId":"../escape","name":"unsafe"}))?["ok"]
                == false
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut accepted = false;
        while Instant::now() < deadline {
            route(&a, &b, 34);
            route(&b, &a, 12);
            let status = b.status()?;
            if !accepted && status["incoming"]["state"] == "offered" {
                assert!(
                    !b.root.join("received").exists(),
                    "must not save before acceptance"
                );
                let response =
                    b.call(json!({"operation":"accept","id":status["incoming"]["id"]}))?;
                assert_eq!(response["ok"], true);
                accepted = true;
            }
            if a.status()?["outgoing"]["state"] == "completed" {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(a.status()?["outgoing"]["state"], "completed");
        let status = b.status()?;
        let saved = status["incoming"]["savedPath"]
            .as_str()
            .ok_or("saved file")?;
        assert_eq!(std::fs::read_to_string(saved)?, content);
        assert!(!source.exists(), "selected cache copy must be released");
        assert_eq!(status["incoming"]["name"], "tiếng Việt 😀.txt");
        sanser_mobile_files_destroy(b.handle);
        std::thread::sleep(Duration::from_millis(120));
        assert_eq!(
            std::fs::read_to_string(saved)?,
            content,
            "verified receipt must survive disconnect"
        );
        // A dead handle cannot target a later session.
        sanser_mobile_files_destroy(a.handle);
        assert_eq!(a.call(json!({"operation":"status"}))?["ok"], false);
        Ok(())
    }
}
