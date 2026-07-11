use crate::sanitize_value;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use thiserror::Error;

const MAX_LOG_ENTRIES: usize = 100_000;
const MAX_LOG_BYTES: usize = 64 * 1_024 * 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEvent {
    pub timestamp_us: u64,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub fields: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogBufferConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for LogBufferConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_bytes: 4 * 1_024 * 1_024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BoundedLogBuffer {
    events: VecDeque<(usize, LogEvent)>,
    bytes: usize,
    config: LogBufferConfig,
}

impl BoundedLogBuffer {
    /// Creates a log buffer with entry-count and encoded-size limits.
    ///
    /// # Errors
    ///
    /// Returns [`LogBufferError::EntryCapacity`] or
    /// [`LogBufferError::ByteCapacity`] when the corresponding limit is zero or
    /// exceeds the supported maximum.
    pub fn new(config: LogBufferConfig) -> Result<Self, LogBufferError> {
        if config.max_entries == 0 || config.max_entries > MAX_LOG_ENTRIES {
            return Err(LogBufferError::EntryCapacity(config.max_entries));
        }
        if config.max_bytes == 0 || config.max_bytes > MAX_LOG_BYTES {
            return Err(LogBufferError::ByteCapacity(config.max_bytes));
        }
        Ok(Self {
            events: VecDeque::with_capacity(config.max_entries.min(1_024)),
            bytes: 0,
            config,
        })
    }

    /// Sanitizes and appends an event, returning the number of evicted entries.
    ///
    /// # Errors
    ///
    /// Returns [`LogBufferError::Serialization`] when the sanitized event cannot
    /// be encoded, or [`LogBufferError::EventTooLarge`] when one event exceeds
    /// the configured byte capacity.
    pub fn push(&mut self, mut event: LogEvent) -> Result<usize, LogBufferError> {
        event.message = crate::sanitize_text(&event.message);
        event.fields = sanitize_value(&event.fields);
        let bytes = serde_json::to_vec(&event)
            .map_err(|_| LogBufferError::Serialization)?
            .len();
        if bytes > self.config.max_bytes {
            return Err(LogBufferError::EventTooLarge(bytes));
        }
        let mut dropped = 0;
        while self.events.len() >= self.config.max_entries
            || self.bytes.saturating_add(bytes) > self.config.max_bytes
        {
            let Some((stale_bytes, _)) = self.events.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(stale_bytes);
            dropped += 1;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.events.push_back((bytes, event));
        Ok(dropped)
    }

    #[must_use]
    pub fn events(&self) -> impl ExactSizeIterator<Item = &LogEvent> {
        self.events.iter().map(|(_, event)| event)
    }

    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Default for BoundedLogBuffer {
    fn default() -> Self {
        Self {
            events: VecDeque::with_capacity(1_024),
            bytes: 0,
            config: LogBufferConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LogBufferError {
    #[error("log entry capacity {0} is outside 1..=100000")]
    EntryCapacity(usize),
    #[error("log byte capacity {0} is outside 1..=67108864")]
    ByteCapacity(usize),
    #[error("sanitized log event size {0} exceeds the buffer capacity")]
    EventTooLarge(usize),
    #[error("log event serialization failed")]
    Serialization,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn buffer_is_bounded_and_sanitizes_before_retaining() {
        let mut buffer = BoundedLogBuffer::new(LogBufferConfig {
            max_entries: 1,
            max_bytes: 1_024,
        })
        .unwrap_or_else(|error| panic!("buffer setup failed: {error}"));
        let event = |timestamp_us| LogEvent {
            timestamp_us,
            level: LogLevel::Info,
            target: "test".to_owned(),
            message: "Bearer unsafe-token".to_owned(),
            fields: json!({"password": "unsafe-password"}),
        };
        buffer
            .push(event(1))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(buffer.push(event(2)), Ok(1));
        let rendered = serde_json::to_string(&buffer.events().next())
            .unwrap_or_else(|error| panic!("serialization failed: {error}"));
        assert!(!rendered.contains("unsafe"));
    }
}
