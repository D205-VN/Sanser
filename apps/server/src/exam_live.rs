//! Exam screenshots and "watch closely" requests. Memory only: a restart
//! forgets them, and nothing here reaches the database or the logs.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

/// A screenshot older than this is dropped: the student is gone or stuck.
pub const SCREEN_TTL: Duration = Duration::from_secs(180);
/// How long one "watch closely" request speeds up the screenshots.
pub const WATCH_FOR: Duration = Duration::from_secs(90);
const MAX_SCREENS: usize = 2_000;

#[derive(Clone)]
pub struct Screen {
    pub room_id: String,
    pub jpeg_base64: String,
    pub captured_at: i64,
    stored: Instant,
}

#[derive(Default)]
pub struct ExamLive {
    screens: Mutex<HashMap<String, Screen>>,
    watched: Mutex<HashMap<String, Instant>>,
}

impl ExamLive {
    pub async fn put_screen(
        &self,
        member_id: &str,
        room_id: &str,
        jpeg_base64: String,
        captured_at: i64,
    ) {
        let now = Instant::now();
        let mut screens = self.screens.lock().await;
        screens.retain(|_, screen| now.duration_since(screen.stored) < SCREEN_TTL);
        if screens.len() >= MAX_SCREENS && !screens.contains_key(member_id) {
            // Full of live rooms: drop the oldest picture rather than refuse.
            if let Some(oldest) = screens
                .iter()
                .min_by_key(|(_, screen)| screen.stored)
                .map(|(id, _)| id.clone())
            {
                screens.remove(&oldest);
            }
        }
        screens.insert(
            member_id.to_owned(),
            Screen {
                room_id: room_id.to_owned(),
                jpeg_base64,
                captured_at,
                stored: now,
            },
        );
    }

    /// Screens of this room captured after `since` (unix seconds).
    pub async fn room_screens(&self, room_id: &str, since: i64) -> Vec<(String, Screen)> {
        let now = Instant::now();
        let screens = self.screens.lock().await;
        screens
            .iter()
            .filter(|(_, screen)| {
                screen.room_id == room_id
                    && screen.captured_at > since
                    && now.duration_since(screen.stored) < SCREEN_TTL
            })
            .map(|(id, screen)| (id.clone(), screen.clone()))
            .collect()
    }

    pub async fn forget_room(&self, room_id: &str) {
        self.screens
            .lock()
            .await
            .retain(|_, screen| screen.room_id != room_id);
    }

    pub async fn forget_member(&self, member_id: &str) {
        self.screens.lock().await.remove(member_id);
        self.watched.lock().await.remove(member_id);
    }

    pub async fn watch(&self, member_id: &str) {
        let now = Instant::now();
        let mut watched = self.watched.lock().await;
        watched.retain(|_, until| *until > now);
        watched.insert(member_id.to_owned(), now + WATCH_FOR);
    }

    pub async fn is_watched(&self, member_id: &str) -> bool {
        self.watched
            .lock()
            .await
            .get(member_id)
            .is_some_and(|until| *until > Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn screens_are_scoped_to_their_room_and_newer_than_asked() {
        let live = ExamLive::default();
        live.put_screen("a", "room-1", "AAA".into(), 100).await;
        live.put_screen("b", "room-1", "BBB".into(), 200).await;
        live.put_screen("c", "room-2", "CCC".into(), 300).await;
        let mut ids: Vec<_> = live
            .room_screens("room-1", 150)
            .await
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["b"]);
        assert_eq!(live.room_screens("room-1", 0).await.len(), 2);
        live.forget_room("room-1").await;
        assert!(live.room_screens("room-1", 0).await.is_empty());
        assert_eq!(live.room_screens("room-2", 0).await.len(), 1);
    }

    #[tokio::test]
    async fn watching_is_per_member_and_forgotten_with_it() {
        let live = ExamLive::default();
        assert!(!live.is_watched("a").await);
        live.watch("a").await;
        assert!(live.is_watched("a").await);
        assert!(!live.is_watched("b").await);
        live.forget_member("a").await;
        assert!(!live.is_watched("a").await);
    }
}
