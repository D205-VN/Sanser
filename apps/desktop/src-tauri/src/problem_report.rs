//! "Report a problem" bundle: one local zip with the diagnostics export, the
//! latest connection reports and a system/capability summary. Everything is
//! redacted before it is written, nothing is uploaded, and the only network
//! side effect the user can trigger is opening a prefilled GitHub issue page
//! in their own browser (no attachment, no personal data in the URL).

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
};

// Only the macOS and Windows helpers start processes; other targets (the Linux
// CI build) would otherwise report an unused import.
#[cfg(any(target_os = "macos", windows))]
use std::process::Command;

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};

use crate::{error::DesktopError, models::SANSER_VERSION};

pub const REDACTED: &str = "[REDACTED]";
const EMAIL: &str = "[EMAIL]";
const IP: &str = "[IP]";
const TOKEN: &str = "[TOKEN]";
const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_REPORT_FILES: usize = 5;
const MAX_STRING_CHARS: usize = 4096;
pub const ISSUE_URL_PREFIX: &str = "https://github.com/D205-VN/Sanser/issues/new?";
const MAX_ISSUE_URL_BYTES: usize = 6000;

/// Context that the redactor removes in addition to generic patterns. The
/// home directory and account name otherwise leak through file paths.
#[derive(Default, Clone)]
pub struct Redactor {
    literals: Vec<(String, &'static str)>,
}

impl Redactor {
    #[must_use]
    pub fn new(home: Option<&Path>, user_name: Option<&str>) -> Self {
        let mut literals = Vec::new();
        if let Some(home) = home.map(|path| path.to_string_lossy().into_owned())
            && home.len() > 3
        {
            literals.push((home, "~"));
        }
        if let Some(name) = user_name.filter(|name| name.len() >= 3) {
            literals.push((name.to_owned(), "[USER]"));
        }
        // Longest first so "/Users/name" is replaced before "name".
        literals.sort_by_key(|(literal, _)| std::cmp::Reverse(literal.len()));
        Self { literals }
    }

    #[must_use]
    pub fn text(&self, input: &str) -> String {
        let mut text = input.to_owned();
        // Paths first (they contain the user name); bare names after words
        // were classified, so "name.surname@example.com" is still an e-mail.
        for (literal, replacement) in self.literals.iter().filter(|(_, r)| *r == "~") {
            if text.contains(literal.as_str()) {
                text = text.replace(literal.as_str(), replacement);
            }
        }
        let mut output = String::with_capacity(text.len());
        let mut redact_next = false;
        let mut word = String::new();
        for character in text.chars() {
            if character.is_whitespace() {
                if !word.is_empty() {
                    output.push_str(&redact_word(&word, &mut redact_next));
                    word.clear();
                }
                output.push(character);
            } else {
                word.push(character);
            }
        }
        if !word.is_empty() {
            output.push_str(&redact_word(&word, &mut redact_next));
        }
        for (literal, replacement) in self.literals.iter().filter(|(_, r)| *r != "~") {
            if output.contains(literal.as_str()) {
                output = output.replace(literal.as_str(), replacement);
            }
        }
        if output.chars().count() > MAX_STRING_CHARS {
            output = output.chars().take(MAX_STRING_CHARS).collect();
            output.push('…');
        }
        output
    }

    #[must_use]
    pub fn value(&self, value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let mut redacted = Map::with_capacity(fields.len());
                for (key, child) in fields {
                    let child = if sensitive_key(key) {
                        Value::String(REDACTED.into())
                    } else {
                        self.value(child)
                    };
                    redacted.insert(self.text(key), child);
                }
                Value::Object(redacted)
            }
            Value::Array(values) => {
                Value::Array(values.iter().map(|item| self.value(item)).collect())
            }
            Value::String(text) => Value::String(self.text(text)),
            primitive => primitive.clone(),
        }
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

const SENSITIVE_PARTS: [&str; 12] = [
    "password",
    "passphrase",
    "token",
    "credential",
    "secret",
    "authorization",
    "privatekey",
    "databaseurl",
    "apikey",
    "cookie",
    "email",
    "signature",
];
const SENSITIVE_KEYS: [&str; 14] = [
    "name",
    "username",
    "displayname",
    "devicename",
    "hostname",
    "computername",
    "viewername",
    "accountname",
    "account",
    "nonce",
    "mac",
    "proof",
    "key",
    "sessionkey",
];

/// Keys whose value is always removed regardless of content: secrets and
/// personal identifiers (account e-mail, user/device names, addresses).
#[must_use]
pub fn sensitive_key(key: &str) -> bool {
    let key = normalize_key(key);
    SENSITIVE_PARTS.iter().any(|needle| key.contains(needle))
        || SENSITIVE_KEYS.contains(&key.as_str())
}

fn is_trim(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '`'
    )
}

fn redact_word(word: &str, redact_next: &mut bool) -> String {
    let start = word
        .find(|character| !is_trim(character))
        .unwrap_or(word.len());
    let end = word
        .rfind(|character| !is_trim(character))
        .map_or(start, |index| {
            index + word[index..].chars().next().map_or(1, char::len_utf8)
        });
    if start >= end {
        return word.to_owned();
    }
    let (mut prefix, mut core, suffix) = (&word[..start], &word[start..end], &word[end..]);
    // "[v6]:port" — the opening bracket belongs to the address.
    let bracketed;
    if prefix.ends_with('[') && core.contains("]:") {
        bracketed = format!("[{core}");
        if ip_with_port(&bracketed).is_some() {
            prefix = &prefix[..prefix.len() - 1];
            core = &bracketed;
        }
    }
    if *redact_next {
        *redact_next = false;
        return format!("{prefix}{REDACTED}{suffix}");
    }
    let lowered = core.to_ascii_lowercase();
    if lowered == "bearer" || lowered.ends_with(":bearer") {
        *redact_next = true;
        return word.to_owned();
    }
    // An IPv6 literal with a trailing ':' (as in "address:") must not be cut
    // at the first colon, so classify the whole core before key=value forms.
    if let Some(replacement) = classify(core) {
        return format!("{prefix}{replacement}{suffix}");
    }
    for separator in ['=', ':'] {
        if let Some((key, value)) = core.split_once(separator)
            && !key.is_empty()
            && !key.contains('/')
            && !(separator == ':' && value.starts_with("//"))
        {
            if sensitive_key(key) && !value.is_empty() {
                return format!("{prefix}{key}{separator}{REDACTED}{suffix}");
            }
            if !value.is_empty() {
                let inner = redact_word(value, redact_next);
                if inner != value {
                    return format!("{prefix}{key}{separator}{inner}{suffix}");
                }
            }
        }
    }
    format!("{prefix}{core}{suffix}")
}

fn classify(core: &str) -> Option<String> {
    if is_email(core) {
        return Some(EMAIL.into());
    }
    if let Some(port) = ip_with_port(core) {
        return Some(match port {
            Some(port) => format!("{IP}:{port}"),
            None => IP.into(),
        });
    }
    if core.contains("://") {
        return redact_url(core);
    }
    if is_opaque_token(core) {
        return Some(TOKEN.into());
    }
    None
}

fn is_email(text: &str) -> bool {
    let text = text.strip_prefix("mailto:").unwrap_or(text);
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && local
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._%+-".contains(character))
        && domain
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character))
}

/// Returns `Some(port)` when the text is an IP literal, optionally with a
/// port (`1.2.3.4:5000`, `[fe80::1]:5000`, `fe80::1%en0`).
#[allow(clippy::option_option)] // None = not an IP, Some(None) = IP without port
fn ip_with_port(text: &str) -> Option<Option<u16>> {
    if let Some(rest) = text.strip_prefix('[') {
        let (address, tail) = rest.split_once(']')?;
        parse_ip(address)?;
        return match tail.strip_prefix(':') {
            None if tail.is_empty() => Some(None),
            Some(port) => port.parse().ok().map(Some),
            None => None,
        };
    }
    if parse_ip(text).is_some() {
        return Some(None);
    }
    let (address, port) = text.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    address.parse::<Ipv4Addr>().ok()?;
    Some(Some(port))
}

fn parse_ip(text: &str) -> Option<IpAddr> {
    let address = text
        .split_once('%')
        .map_or(text, |(address, _zone)| address);
    if let Ok(v4) = address.parse::<Ipv4Addr>() {
        return Some(IpAddr::V4(v4));
    }
    if address.matches(':').count() >= 2 {
        return address.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
    }
    None
}

fn redact_url(text: &str) -> Option<String> {
    let mut url = url::Url::parse(text).ok()?;
    let mut changed = false;
    if matches!(url.host(), Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))) {
        // url::Url refuses a non-IP host for some schemes; fall back to text.
        if url.set_host(Some("redacted-ip")).is_err() {
            return Some(REDACTED.into());
        }
        changed = true;
    }
    if !url.username().is_empty() || url.password().is_some() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        changed = true;
    }
    if url.query().is_some() {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| {
                let value = if sensitive_key(&key) {
                    changed = true;
                    REDACTED.to_owned()
                } else {
                    let redacted = Redactor::default().text(&value);
                    changed |= redacted != value;
                    redacted
                };
                (key.into_owned(), value)
            })
            .collect();
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(pairs);
    }
    changed.then(|| url.to_string())
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(index, character)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
}

/// Long opaque credentials (JWTs, base64/hex keys). UUIDs remain because
/// session identifiers are needed to correlate reports and are not secret.
fn is_opaque_token(text: &str) -> bool {
    if text.len() < 32 || is_uuid(text) || text.starts_with('/') || text.contains('\\') {
        return false;
    }
    let allowed = text
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_+/=.".contains(character));
    let letters = text
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    let digits = text.chars().any(|character| character.is_ascii_digit());
    // Dotted identifiers such as "sanser.desktop.module" are not tokens
    // unless one of their segments is itself long and opaque (a JWT).
    let long_segment = text.split(['.', '/']).any(|segment| segment.len() >= 20);
    allowed && letters && digits && long_segment
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemReport {
    pub path: String,
    pub folder: String,
}

/// Minimal STORED (uncompressed) zip writer. Reports are small text files;
/// avoiding a compression dependency keeps the bundle format auditable.
pub struct ZipWriter {
    data: Vec<u8>,
    central: Vec<u8>,
    entries: u16,
}

impl Default for ZipWriter {
    fn default() -> Self {
        Self::new()
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

impl ZipWriter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            central: Vec::new(),
            entries: 0,
        }
    }

    pub fn add(&mut self, name: &str, contents: &[u8]) -> Result<(), DesktopError> {
        let too_large = || DesktopError::Storage("report entry is too large".into());
        let size = u32::try_from(contents.len()).map_err(|_| too_large())?;
        let offset = u32::try_from(self.data.len()).map_err(|_| too_large())?;
        let name_len = u16::try_from(name.len()).map_err(|_| too_large())?;
        self.entries = self.entries.checked_add(1).ok_or_else(too_large)?;
        let crc = crc32(contents);
        // DOS date 2026-01-01 00:00; exact times are not needed in a report.
        let (time, date): (u16, u16) = (0, ((2026 - 1980) << 9) | (1 << 5) | 1);
        let mut local = Vec::with_capacity(30 + name.len());
        local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        local.extend_from_slice(&20_u16.to_le_bytes());
        local.extend_from_slice(&0x0800_u16.to_le_bytes()); // UTF-8 names
        local.extend_from_slice(&0_u16.to_le_bytes()); // stored
        local.extend_from_slice(&time.to_le_bytes());
        local.extend_from_slice(&date.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&size.to_le_bytes());
        local.extend_from_slice(&name_len.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        self.data.extend_from_slice(&local);
        self.data.extend_from_slice(contents);

        self.central
            .extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        self.central.extend_from_slice(&20_u16.to_le_bytes());
        self.central.extend_from_slice(&20_u16.to_le_bytes());
        self.central.extend_from_slice(&0x0800_u16.to_le_bytes());
        self.central.extend_from_slice(&0_u16.to_le_bytes());
        self.central.extend_from_slice(&time.to_le_bytes());
        self.central.extend_from_slice(&date.to_le_bytes());
        self.central.extend_from_slice(&crc.to_le_bytes());
        self.central.extend_from_slice(&size.to_le_bytes());
        self.central.extend_from_slice(&size.to_le_bytes());
        self.central.extend_from_slice(&name_len.to_le_bytes());
        self.central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
        self.central.extend_from_slice(&offset.to_le_bytes());
        self.central.extend_from_slice(name.as_bytes());
        Ok(())
    }

    pub fn finish(mut self) -> Result<Vec<u8>, DesktopError> {
        let too_large = || DesktopError::Storage("report is too large".into());
        let offset = u32::try_from(self.data.len()).map_err(|_| too_large())?;
        let size = u32::try_from(self.central.len()).map_err(|_| too_large())?;
        self.data.extend_from_slice(&self.central);
        self.data.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        self.data.extend_from_slice(&[0; 4]);
        self.data.extend_from_slice(&self.entries.to_le_bytes());
        self.data.extend_from_slice(&self.entries.to_le_bytes());
        self.data.extend_from_slice(&size.to_le_bytes());
        self.data.extend_from_slice(&offset.to_le_bytes());
        self.data.extend_from_slice(&0_u16.to_le_bytes());
        Ok(self.data)
    }
}

/// `YYYYMMDD-HHMMSS` in UTC, used for report and recording file names.
#[must_use]
pub fn utc_stamp(epoch_seconds: u64) -> String {
    let days = i64::try_from(epoch_seconds / 86_400).unwrap_or(0);
    let seconds = epoch_seconds % 86_400;
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

pub fn epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output();
    #[cfg(windows)]
    let output = crate::engine::native_command(Path::new("cmd"))
        .args(["/C", "ver"])
        .output();
    #[cfg(not(any(target_os = "macos", windows)))]
    let output: std::io::Result<std::process::Output> = Err(std::io::Error::other("unsupported"));
    output
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|version| !version.is_empty() && version.len() < 128)
        .unwrap_or_else(|| "unknown".into())
}

/// Builds the zip bytes. Separate from disk I/O so the contents can be tested.
pub fn build_bundle(
    redactor: &Redactor,
    diagnostics: &Value,
    reports: &[Value],
    capabilities: &Value,
    os_version: &str,
) -> Result<Vec<u8>, DesktopError> {
    let pretty = |value: &Value| {
        serde_json::to_vec_pretty(&redactor.value(value))
            .map_err(|error| DesktopError::Storage(error.to_string()))
    };
    let system = serde_json::json!({
        "app": "Sanser",
        "appVersion": SANSER_VERSION,
        "os": std::env::consts::OS,
        "osVersion": os_version,
        "arch": std::env::consts::ARCH,
        "capabilities": capabilities,
    });
    let mut zip = ZipWriter::new();
    zip.add(
        "README.txt",
        b"Sanser problem report\n\n\
This archive was created on this computer and has not been sent anywhere.\n\
Passwords, tokens, e-mail addresses, IP addresses, user and device names were\n\
replaced with [REDACTED], [EMAIL], [IP], [TOKEN] or [USER] before writing.\n\
Review the files before attaching the archive to a GitHub issue.\n\n\
system.json       app/OS version and runtime capability summary\n\
diagnostics.json  local event buffer and connection reports\n\
connection-reports/  latest native connection reports\n",
    )?;
    zip.add("system.json", &pretty(&system)?)?;
    zip.add("diagnostics.json", &pretty(diagnostics)?)?;
    for (index, report) in reports.iter().take(MAX_REPORT_FILES).enumerate() {
        zip.add(
            &format!("connection-reports/report-{}.json", index + 1),
            &pretty(report)?,
        )?;
    }
    zip.finish()
}

fn report_folder(app: &AppHandle) -> Result<PathBuf, DesktopError> {
    Ok(app
        .path()
        .download_dir()
        .map_err(|_| DesktopError::Storage("Downloads directory is unavailable".into()))?
        .join("Sanser")
        .join("Reports"))
}

pub fn create(
    app: &AppHandle,
    diagnostics: &str,
    capabilities: &str,
) -> Result<ProblemReport, DesktopError> {
    if diagnostics.len() > MAX_INPUT_BYTES || capabilities.len() > MAX_INPUT_BYTES {
        return Err(DesktopError::InvalidRequest(
            "report input is too large".into(),
        ));
    }
    let mut diagnostics: Value = serde_json::from_str(diagnostics)
        .map_err(|_| DesktopError::InvalidRequest("diagnostics must be valid JSON".into()))?;
    let capabilities: Value = serde_json::from_str(capabilities)
        .map_err(|_| DesktopError::InvalidRequest("capabilities must be valid JSON".into()))?;
    let reports = app
        .path()
        .app_log_dir()
        .ok()
        .map_or_else(Vec::new, |directory| {
            crate::connection_report::recent(&directory.join("connections"))
        });
    if let Some(object) = diagnostics.as_object_mut() {
        object.insert("connectionReports".into(), Value::Array(reports.clone()));
    }
    let home = app.path().home_dir().ok();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok();
    let redactor = Redactor::new(home.as_deref(), user.as_deref());
    let bytes = build_bundle(
        &redactor,
        &diagnostics,
        &reports,
        &capabilities,
        &os_version(),
    )?;
    let folder = report_folder(app)?;
    fs::create_dir_all(&folder)?;
    let path = unique_path(
        &folder,
        &format!("sanser-report-{}Z", utc_stamp(epoch_seconds())),
        "zip",
    );
    fs::write(&path, bytes)?;
    reveal(&path);
    Ok(ProblemReport {
        path: redactor.text(&path.to_string_lossy()),
        folder: redactor.text(&folder.to_string_lossy()),
    })
}

/// Picks `<stem>.<ext>`, or `<stem>-2.<ext>` … when the name already exists.
#[must_use]
pub fn unique_path(folder: &Path, stem: &str, extension: &str) -> PathBuf {
    let first = folder.join(format!("{stem}.{extension}"));
    if !first.exists() {
        return first;
    }
    (2..1000)
        .map(|index| folder.join(format!("{stem}-{index}.{extension}")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

/// Shows the file in Finder/Explorer. Best effort: failure leaves the path
/// visible in the UI.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    {
        let mut argument = std::ffi::OsString::from("/select,");
        argument.push(path.as_os_str());
        let _ = Command::new("explorer").arg(argument).spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}

pub fn open_folder(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/open").arg(path).spawn();
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(path).spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}

/// Accepts only the project's "new issue" page with plain query parameters.
pub fn validate_issue_url(url: &str) -> Result<url::Url, DesktopError> {
    let invalid =
        || DesktopError::InvalidRequest("only the Sanser GitHub issue page can be opened".into());
    if !url.starts_with(ISSUE_URL_PREFIX) || url.len() > MAX_ISSUE_URL_BYTES {
        return Err(invalid());
    }
    let parsed = url::Url::parse(url).map_err(|_| invalid())?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.path() != "/D205-VN/Sanser/issues/new"
        || parsed.fragment().is_some()
        || parsed.username() != ""
        || parsed.password().is_some()
    {
        return Err(invalid());
    }
    for (key, value) in parsed.query_pairs() {
        if !matches!(key.as_ref(), "title" | "body" | "labels") {
            return Err(invalid());
        }
        // The body is a template; reject anything that looks personal.
        let redacted = Redactor::default().text(&value);
        if redacted != value {
            return Err(DesktopError::InvalidRequest(
                "the issue text must not contain personal data".into(),
            ));
        }
    }
    Ok(parsed)
}

pub fn open_issue(url: &str) -> Result<(), DesktopError> {
    let parsed = validate_issue_url(url)?;
    #[cfg(target_os = "macos")]
    let result = Command::new("/usr/bin/open").arg(parsed.as_str()).spawn();
    #[cfg(windows)]
    let result = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", parsed.as_str()])
        .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let result: std::io::Result<std::process::Child> = {
        let _ = parsed;
        Err(std::io::Error::other(
            "opening a browser is unsupported on this platform",
        ))
    };
    result
        .map(|_| ())
        .map_err(|error| DesktopError::Process(format!("unable to open the browser: {error}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn redact(text: &str) -> String {
        Redactor::new(Some(Path::new("/Users/alice")), Some("alice")).text(text)
    }

    #[test]
    fn redacts_emails_ips_tokens_and_home_paths() {
        let input = "user alice.smith+x@example.co.uk connected from 203.0.113.7:50000 \
                     via [2001:db8::1]:443 and fe80::1%en0; Authorization: Bearer abc.def.ghi \
                     token=s3cr3t log /Users/alice/Library/Logs/sanser.log \
                     jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let output = redact(input);
        for leaked in [
            "alice.smith",
            "example.co.uk",
            "203.0.113.7",
            "2001:db8",
            "fe80",
            "abc.def.ghi",
            "s3cr3t",
            "/Users/alice",
            "eyJhbGci",
        ] {
            assert!(!output.contains(leaked), "leaked {leaked}: {output}");
        }
        assert!(output.contains("[EMAIL]"));
        assert!(output.contains("[IP]:50000"));
        assert!(output.contains("~/Library/Logs/sanser.log"));
        assert!(output.contains("[TOKEN]"));
        assert!(output.contains("token=[REDACTED]"));
    }

    #[test]
    fn keeps_useful_non_personal_text() {
        let text = "RTT 12.5 ms at 14:03:22, session 6f1c2d3e-0a1b-4c5d-8e9f-001122334455 codec h264 v2.3.16 \
                    module sanser.desktop.engine 1920x1080";
        assert_eq!(redact(text), text);
        assert_eq!(redact("line one\n\tline two"), "line one\n\tline two");
    }

    #[test]
    fn redacts_urls_with_ip_hosts_and_secret_queries() {
        let output = redact(
            "GET http://192.168.1.20:8080/api?token=abc&mode=x https://user:pw@example.com/x",
        );
        assert!(!output.contains("192.168.1.20"), "{output}");
        assert!(!output.contains("abc"), "{output}");
        assert!(!output.contains(":pw@"), "{output}");
        assert!(output.contains("mode=x"), "{output}");
    }

    #[test]
    fn redacts_sensitive_json_keys_recursively() {
        let value = json!({
            "accessToken": "a",
            "account": {"email": "bob@example.com", "displayName": "Bob"},
            "device": {"name": "Bob's MacBook", "routeAddress": "10.0.0.5"},
            "events": [{"message": "peer 10.0.0.9 joined", "rttMs": 4}],
            "sessionId": "6f1c2d3e-0a1b-4c5d-8e9f-001122334455"
        });
        let rendered = Redactor::default().value(&value).to_string();
        for leaked in ["bob@", "Bob", "10.0.0.5", "10.0.0.9", "\"a\""] {
            assert!(!rendered.contains(leaked), "leaked {leaked}: {rendered}");
        }
        assert!(rendered.contains("6f1c2d3e-0a1b-4c5d-8e9f-001122334455"));
        assert!(rendered.contains("\"rttMs\":4"));
    }

    fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], &0x0605_4b50_u32.to_le_bytes());
        let count = u16::from_le_bytes([zip[end + 10], zip[end + 11]]);
        let mut offset = u32::from_le_bytes(zip[end + 16..end + 20].try_into().unwrap()) as usize;
        let mut result = Vec::new();
        for _ in 0..count {
            assert_eq!(&zip[offset..offset + 4], &0x0201_4b50_u32.to_le_bytes());
            let crc = u32::from_le_bytes(zip[offset + 16..offset + 20].try_into().unwrap());
            let size =
                u32::from_le_bytes(zip[offset + 20..offset + 24].try_into().unwrap()) as usize;
            let name_len = u16::from_le_bytes([zip[offset + 28], zip[offset + 29]]) as usize;
            let local =
                u32::from_le_bytes(zip[offset + 42..offset + 46].try_into().unwrap()) as usize;
            let name =
                String::from_utf8(zip[offset + 46..offset + 46 + name_len].to_vec()).unwrap();
            let local_name = u16::from_le_bytes([zip[local + 26], zip[local + 27]]) as usize;
            let data = zip[local + 30 + local_name..local + 30 + local_name + size].to_vec();
            assert_eq!(crc32(&data), crc);
            result.push((name, data));
            offset += 46 + name_len;
        }
        result
    }

    #[test]
    fn crc_matches_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn bundle_contains_redacted_files() {
        let redactor = Redactor::new(Some(Path::new("/Users/alice")), Some("alice"));
        let diagnostics =
            json!({"events": [{"message": "login alice@example.com from 198.51.100.4"}]});
        let reports = vec![json!({"route": "direct", "peer": "198.51.100.4:9000"}); 7];
        let capabilities = json!({"hostEngine": {"state": "available"}});
        let zip = build_bundle(&redactor, &diagnostics, &reports, &capabilities, "15.1").unwrap();
        let files = entries(&zip);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names[..3],
            ["README.txt", "system.json", "diagnostics.json"]
        );
        assert_eq!(names.len(), 3 + MAX_REPORT_FILES);
        let all: String = files
            .iter()
            .map(|(_, data)| String::from_utf8_lossy(data).into_owned())
            .collect();
        assert!(!all.contains("alice@"));
        assert!(!all.contains("198.51.100.4"));
        assert!(all.contains("\"appVersion\""));
        assert!(all.contains("\"osVersion\": \"15.1\""));
        assert!(all.contains("available"));
    }

    #[test]
    fn utc_stamp_formats_civil_dates() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(1_795_000_000), "20261118-110640");
        assert_eq!(utc_stamp(951_782_400), "20000229-000000");
    }

    #[test]
    fn issue_url_validation_only_allows_the_template_page() {
        let good = format!(
            "{ISSUE_URL_PREFIX}title=Problem%3A%20&body=Version%3A%202.3.16%0AOS%3A%20macos"
        );
        assert!(validate_issue_url(&good).is_ok());
        for bad in [
            "https://github.com/D205-VN/Sanser/issues/new/../../evil?title=x".to_owned(),
            "https://github.com.evil.com/D205-VN/Sanser/issues/new?title=x".to_owned(),
            format!("{ISSUE_URL_PREFIX}title=x#frag"),
            format!("{ISSUE_URL_PREFIX}assignees=me"),
            format!("{ISSUE_URL_PREFIX}body=contact%20me%20at%20a%40b.com"),
            format!("{ISSUE_URL_PREFIX}body=host%2010.1.2.3"),
            "file:///etc/passwd".to_owned(),
        ] {
            assert!(validate_issue_url(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn unique_path_does_not_overwrite() {
        let folder =
            std::env::temp_dir().join(format!("sanser-report-test-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let first = unique_path(&folder, "report", "zip");
        fs::write(&first, b"x").unwrap();
        let second = unique_path(&folder, "report", "zip");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("report-2.zip"));
        let _ = fs::remove_dir_all(folder);
    }
}
