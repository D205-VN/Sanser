//! Exam screenshots and "watch closely" requests. Memory only: a restart
//! forgets them, and nothing here reaches the database or the logs.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

/// A screenshot older than this is dropped: the student is gone or stuck.
pub const SCREEN_TTL: Duration = Duration::from_secs(180);
/// How long one "watch closely" request speeds up the screenshots.
pub const WATCH_FOR: Duration = Duration::from_secs(90);
/// A heartbeat lets its seat send screens this long without the database.
pub const SEAT_PASS: Duration = Duration::from_secs(30);
const MAX_SCREENS: usize = 2_000;

#[derive(Clone)]
pub struct Screen {
    pub room_id: String,
    pub jpeg_base64: String,
    pub captured_at: i64,
    /// Increases with every stored screen, so a supervisor never misses two
    /// pictures taken within the same second.
    pub seq: u64,
    stored: Instant,
}

struct SeatPass {
    user_id: String,
    room_id: String,
    until: Instant,
}

#[derive(Default)]
pub struct ExamLive {
    screens: Mutex<HashMap<String, Screen>>,
    watched: Mutex<HashMap<String, Instant>>,
    seats: Mutex<HashMap<String, SeatPass>>,
    next_seq: AtomicU64,
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
                seq: self.next_seq.fetch_add(1, Ordering::Relaxed) + 1,
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

    /// Screens of this room stored after sequence number `after`.
    pub async fn room_screens_after(&self, room_id: &str, after: u64) -> Vec<(String, Screen)> {
        let now = Instant::now();
        let screens = self.screens.lock().await;
        screens
            .iter()
            .filter(|(_, screen)| {
                screen.room_id == room_id
                    && screen.seq > after
                    && now.duration_since(screen.stored) < SCREEN_TTL
            })
            .map(|(id, screen)| (id.clone(), screen.clone()))
            .collect()
    }

    /// Called after a heartbeat checked the seat in the database.
    pub async fn allow_seat(&self, member_id: &str, user_id: &str, room_id: &str) {
        let now = Instant::now();
        let mut seats = self.seats.lock().await;
        seats.retain(|_, pass| pass.until > now);
        seats.insert(
            member_id.to_owned(),
            SeatPass {
                user_id: user_id.to_owned(),
                room_id: room_id.to_owned(),
                until: now + SEAT_PASS,
            },
        );
    }

    /// The room a seat may send screens to, if a recent heartbeat allowed it.
    pub async fn seat_room(&self, member_id: &str, user_id: &str) -> Option<String> {
        let now = Instant::now();
        self.seats
            .lock()
            .await
            .get(member_id)
            .filter(|pass| pass.user_id == user_id && pass.until > now)
            .map(|pass| pass.room_id.clone())
    }

    pub async fn forget_room(&self, room_id: &str) {
        self.screens
            .lock()
            .await
            .retain(|_, screen| screen.room_id != room_id);
        self.seats
            .lock()
            .await
            .retain(|_, pass| pass.room_id != room_id);
    }

    pub async fn forget_member(&self, member_id: &str) {
        self.screens.lock().await.remove(member_id);
        self.watched.lock().await.remove(member_id);
        self.seats.lock().await.remove(member_id);
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
    async fn sequence_numbers_keep_two_screens_in_one_second() {
        let live = ExamLive::default();
        live.put_screen("a", "room-1", "A1".into(), 100).await;
        let first = live.room_screens_after("room-1", 0).await;
        let seen = first[0].1.seq;
        live.put_screen("a", "room-1", "A2".into(), 100).await;
        let next = live.room_screens_after("room-1", seen).await;
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].1.jpeg_base64, "A2");
        assert!(
            live.room_screens_after("room-1", next[0].1.seq)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_seat_pass_belongs_to_its_user_and_ends_with_the_member() {
        let live = ExamLive::default();
        assert_eq!(live.seat_room("a", "student").await, None);
        live.allow_seat("a", "student", "room-1").await;
        assert_eq!(
            live.seat_room("a", "student").await.as_deref(),
            Some("room-1")
        );
        assert_eq!(live.seat_room("a", "someone-else").await, None);
        live.forget_member("a").await;
        assert_eq!(live.seat_room("a", "student").await, None);
        live.allow_seat("b", "student", "room-2").await;
        live.forget_room("room-2").await;
        assert_eq!(live.seat_room("b", "student").await, None);
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
