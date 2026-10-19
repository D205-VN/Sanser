//! Session-scoped group voice consent. Microphone defaults off and is never persisted.
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct VoiceStatus {
    pub joined: bool,
    pub microphone: bool,
    pub deafened: bool,
    pub capture: bool,
    pub code: String,
}
impl Default for VoiceStatus {
    fn default() -> Self {
        Self {
            joined: false,
            microphone: false,
            deafened: false,
            capture: false,
            code: "idle".into(),
        }
    }
}
pub(crate) type ObservedVoice = std::sync::Arc<std::sync::Mutex<VoiceStatus>>;

pub(crate) fn parse_status(line: &[u8]) -> Option<VoiceStatus> {
    let line = std::str::from_utf8(line)
        .ok()?
        .strip_prefix("SNVOICE_STATUS ")?;
    let mut values = std::collections::HashMap::new();
    for field in line.split_ascii_whitespace() {
        let (key, value) = field.split_once('=')?;
        if values.insert(key, value).is_some() {
            return None;
        }
    }
    let bit = |key| match values.get(key).copied() {
        Some("0") => Some(false),
        Some("1") => Some(true),
        _ => None,
    };
    let code = *values.get("code")?;
    if !matches!(
        code,
        "idle"
            | "listening"
            | "starting"
            | "running"
            | "permission-requested"
            | "device-changed"
            | "deafened"
            | "permission-denied"
            | "no-device"
            | "unavailable"
            | "stopped"
    ) {
        return None;
    }
    Some(VoiceStatus {
        joined: bit("joined")?,
        microphone: bit("mic")?,
        deafened: bit("deafen")?,
        capture: bit("capture")?,
        code: code.into(),
    })
}

pub(crate) fn config_command(joined: bool, microphone: bool, deafened: bool) -> String {
    format!(
        "voice-config\t{}\t{}\t{}\n",
        u8::from(joined),
        u8::from(joined && microphone && !deafened),
        u8::from(joined && deafened)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_consent_never_opens_microphone_when_leaving_or_deafened() {
        assert_eq!(
            config_command(true, false, false),
            "voice-config\t1\t0\t0\n"
        );
        assert_eq!(
            config_command(false, true, false),
            "voice-config\t0\t0\t0\n"
        );
        assert_eq!(config_command(true, true, true), "voice-config\t1\t0\t1\n");
        assert_eq!(config_command(true, true, false), "voice-config\t1\t1\t0\n");
    }
    #[test]
    fn native_permission_status_is_strict_and_does_not_fabricate_capture()
    -> Result<(), &'static str> {
        let status = parse_status(
            b"SNVOICE_STATUS joined=1 mic=1 deafen=0 capture=0 code=permission-denied",
        )
        .ok_or("missing native voice status")?;
        assert!(!status.capture);
        assert_eq!(status.code, "permission-denied");
        for code in ["permission-requested", "device-changed", "deafened"] {
            let line = format!("SNVOICE_STATUS joined=1 mic=0 deafen=0 capture=0 code={code}");
            assert_eq!(
                parse_status(line.as_bytes())
                    .ok_or("missing lifecycle status")?
                    .code,
                code
            );
        }

        for bad in [
            b"SNVOICE_STATUS joined=1 mic=1 deafen=0 capture=2 code=running".as_slice(),
            b"SNVOICE_STATUS joined=1 mic=1 mic=0 deafen=0 capture=1 code=running",
            b"SNVOICE_STATUS joined=1 mic=1 deafen=0 capture=1 code=secret",
        ] {
            assert!(parse_status(bad).is_none());
        }
        Ok(())
    }
}
