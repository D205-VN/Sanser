//! Viewer recording consent relay. The recording itself happens only on the
//! viewer; the server just carries "viewer X wants to record / is recording"
//! to the host and the host's allow/deny answer back. State is in memory and
//! short-lived: a viewer heartbeats its request while recording, so a crashed
//! viewer or a server restart clears the host's indicator within the TTL.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::Mutex;

pub const TTL: Duration = Duration::from_secs(30);
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsentState {
    None,
    Requested,
    Allowed,
    Denied,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Request,
    Stop,
    Allow,
    Deny,
}

impl Action {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "request" => Some(Self::Request),
            "stop" => Some(Self::Stop),
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentView {
    pub state: ConsentState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_device_id: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConsentError {
    /// The device is not allowed to perform this action for the session.
    Forbidden,
    /// The action does not apply to the current state.
    Conflict(&'static str),
}

struct Entry {
    state: ConsentState,
    viewer_name: String,
    viewer_device_id: String,
    updated: Instant,
}

pub struct Participants<'a> {
    pub requester_device_id: &'a str,
    pub host_device_id: &'a str,
    pub viewer_name: &'a str,
}

#[derive(Default)]
pub struct RecordingConsents {
    inner: Mutex<HashMap<String, Entry>>,
}

fn effective(entry: &Entry, now: Instant) -> ConsentState {
    if now.saturating_duration_since(entry.updated) > TTL {
        ConsentState::None
    } else {
        entry.state
    }
}

fn view(entry: Option<&Entry>, now: Instant) -> ConsentView {
    match entry {
        Some(entry) if effective(entry, now) != ConsentState::None => ConsentView {
            state: entry.state,
            viewer_name: Some(entry.viewer_name.clone()),
            viewer_device_id: Some(entry.viewer_device_id.clone()),
        },
        _ => ConsentView {
            state: ConsentState::None,
            viewer_name: None,
            viewer_device_id: None,
        },
    }
}

/// Bounded, printable display name; the device name comes from the database.
fn display_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect();
    if cleaned.trim().is_empty() {
        "Viewer".into()
    } else {
        cleaned
    }
}

impl RecordingConsents {
    pub async fn get(&self, session: &str, now: Instant) -> ConsentView {
        view(self.inner.lock().await.get(session), now)
    }

    pub async fn apply(
        &self,
        session: &str,
        participants: &Participants<'_>,
        actor_device_id: &str,
        action: Action,
        now: Instant,
    ) -> Result<ConsentView, ConsentError> {
        let viewer = actor_device_id == participants.requester_device_id;
        let host = actor_device_id == participants.host_device_id;
        let mut entries = self.inner.lock().await;
        if entries.len() >= MAX_ENTRIES {
            entries.retain(|_, entry| now.saturating_duration_since(entry.updated) <= TTL);
        }
        let current = entries
            .get(session)
            .map_or(ConsentState::None, |entry| effective(entry, now));
        let next = match action {
            Action::Request | Action::Stop if !viewer => return Err(ConsentError::Forbidden),
            Action::Allow | Action::Deny if !host => return Err(ConsentError::Forbidden),
            // A fresh denial is final for the TTL so a viewer cannot spam the
            // host; heartbeats keep an allowed recording alive.
            Action::Request => match current {
                ConsentState::Denied => return Ok(view(entries.get(session), now)),
                ConsentState::Allowed => ConsentState::Allowed,
                _ => ConsentState::Requested,
            },
            Action::Stop => ConsentState::Stopped,
            Action::Allow => match current {
                ConsentState::Requested | ConsentState::Allowed => ConsentState::Allowed,
                _ => return Err(ConsentError::Conflict("no recording request is pending")),
            },
            // The host may refuse a request or revoke a running recording.
            Action::Deny => match current {
                ConsentState::Requested | ConsentState::Allowed | ConsentState::Denied => {
                    ConsentState::Denied
                }
                _ => return Err(ConsentError::Conflict("no recording request is pending")),
            },
        };
        let entry = entries.entry(session.to_owned()).or_insert_with(|| Entry {
            state: next,
            viewer_name: String::new(),
            viewer_device_id: String::new(),
            updated: now,
        });
        entry.state = next;
        entry.updated = now;
        entry.viewer_name = display_name(participants.viewer_name);
        participants
            .requester_device_id
            .clone_into(&mut entry.viewer_device_id);
        Ok(view(Some(entry), now))
    }

    /// Clears state when the session ends.
    pub async fn remove(&self, session: &str) {
        self.inner.lock().await.remove(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWER: &str = "viewer-device";
    const HOST: &str = "host-device";

    fn participants() -> Participants<'static> {
        Participants {
            requester_device_id: VIEWER,
            host_device_id: HOST,
            viewer_name: "Office\u{7} Mac",
        }
    }

    #[tokio::test]
    async fn request_allow_heartbeat_and_stop() {
        let consents = RecordingConsents::default();
        let now = Instant::now();
        assert_eq!(consents.get("s", now).await.state, ConsentState::None);
        let requested = consents
            .apply("s", &participants(), VIEWER, Action::Request, now)
            .await
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(requested.state, ConsentState::Requested);
        assert_eq!(requested.viewer_name.as_deref(), Some("Office Mac"));
        let allowed = consents
            .apply("s", &participants(), HOST, Action::Allow, now)
            .await;
        assert_eq!(allowed.map(|v| v.state), Ok(ConsentState::Allowed));
        // Heartbeat keeps the allowed state alive past the original TTL.
        let later = now + TTL - Duration::from_secs(1);
        let beat = consents
            .apply("s", &participants(), VIEWER, Action::Request, later)
            .await;
        assert_eq!(beat.map(|v| v.state), Ok(ConsentState::Allowed));
        assert_eq!(
            consents
                .get("s", later + TTL - Duration::from_secs(1))
                .await
                .state,
            ConsentState::Allowed
        );
        let stopped = consents
            .apply("s", &participants(), VIEWER, Action::Stop, later)
            .await;
        assert_eq!(stopped.map(|v| v.state), Ok(ConsentState::Stopped));
    }

    #[tokio::test]
    async fn only_the_right_device_can_act() {
        let consents = RecordingConsents::default();
        let now = Instant::now();
        for (actor, action) in [
            (HOST, Action::Request),
            ("other", Action::Request),
            (VIEWER, Action::Allow),
            (VIEWER, Action::Deny),
            ("other", Action::Stop),
        ] {
            assert_eq!(
                consents
                    .apply("s", &participants(), actor, action, now)
                    .await,
                Err(ConsentError::Forbidden)
            );
        }
        assert!(matches!(
            consents
                .apply("s", &participants(), HOST, Action::Allow, now)
                .await,
            Err(ConsentError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn denial_sticks_and_host_can_revoke() {
        let consents = RecordingConsents::default();
        let now = Instant::now();
        let _ = consents
            .apply("s", &participants(), VIEWER, Action::Request, now)
            .await;
        let _ = consents
            .apply("s", &participants(), HOST, Action::Allow, now)
            .await;
        let revoked = consents
            .apply("s", &participants(), HOST, Action::Deny, now)
            .await;
        assert_eq!(revoked.map(|v| v.state), Ok(ConsentState::Denied));
        let retry = consents
            .apply("s", &participants(), VIEWER, Action::Request, now)
            .await;
        assert_eq!(retry.map(|v| v.state), Ok(ConsentState::Denied));
        // After the TTL the viewer may ask again.
        let later = now + TTL + Duration::from_secs(1);
        let again = consents
            .apply("s", &participants(), VIEWER, Action::Request, later)
            .await;
        assert_eq!(again.map(|v| v.state), Ok(ConsentState::Requested));
    }

    #[tokio::test]
    async fn stale_requests_expire_and_remove_clears() {
        let consents = RecordingConsents::default();
        let now = Instant::now();
        let _ = consents
            .apply("s", &participants(), VIEWER, Action::Request, now)
            .await;
        assert_eq!(
            consents.get("s", now + TTL + Duration::from_secs(1)).await,
            ConsentView {
                state: ConsentState::None,
                viewer_name: None,
                viewer_device_id: None
            }
        );
        let _ = consents
            .apply("s", &participants(), VIEWER, Action::Request, now)
            .await;
        consents.remove("s").await;
        assert_eq!(consents.get("s", now).await.state, ConsentState::None);
    }

    #[test]
    fn actions_parse_strictly() {
        assert_eq!(Action::parse("allow"), Some(Action::Allow));
        assert_eq!(Action::parse("Allow"), None);
        assert_eq!(display_name("   "), "Viewer");
        assert_eq!(display_name(&"x".repeat(100)).len(), 64);
    }
}
