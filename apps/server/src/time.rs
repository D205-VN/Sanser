pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn expires_at(ttl: std::time::Duration) -> i64 {
    now_unix().saturating_add(i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX))
}
