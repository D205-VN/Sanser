use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use sqlx::PgPool;
use tokio::sync::{Mutex, broadcast};

use crate::{config::Config, models::EventEnvelope, websocket::SignalHub};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: PgPool,
    pub events: broadcast::Sender<EventEnvelope>,
    pub signaling: Arc<SignalHub>,
    pub general_limiter: RateLimiter,
    pub login_limiter: RateLimiter,
}

impl AppState {
    pub fn new(config: Config, pool: PgPool) -> Self {
        let general_limit = config.general_rate_limit_per_minute;
        let login_limit = config.login_rate_limit_per_ten_minutes;
        let (events, _) = broadcast::channel(256);
        Self {
            config: Arc::new(config),
            pool,
            events,
            signaling: Arc::new(SignalHub::default()),
            general_limiter: RateLimiter::new(general_limit, Duration::from_secs(60)),
            login_limiter: RateLimiter::new(login_limit, Duration::from_secs(600)),
        }
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, RateWindow>>>,
    max_requests: u32,
    window: Duration,
}

struct RateWindow {
    started: Instant,
    count: u32,
    last_seen: Instant,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    pub async fn check(&self, key: impl Into<String>) -> bool {
        let now = Instant::now();
        let mut windows = self.inner.lock().await;
        if windows.len() > 20_000 {
            let retention = self.window.saturating_mul(2);
            windows.retain(|_, entry| now.duration_since(entry.last_seen) < retention);
        }

        let entry = windows.entry(key.into()).or_insert(RateWindow {
            started: now,
            count: 0,
            last_seen: now,
        });
        if now.duration_since(entry.started) >= self.window {
            entry.started = now;
            entry.count = 0;
        }
        entry.last_seen = now;
        if entry.count >= self.max_requests {
            return false;
        }
        entry.count += 1;
        true
    }
}
