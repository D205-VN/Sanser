use std::collections::BTreeMap;
use thiserror::Error;

const MAX_WINDOW_PACKETS: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetransmissionConfig {
    pub max_packets: usize,
    pub max_age_us: u64,
}

impl RetransmissionConfig {
    /// Validates packet-count and retention-age limits.
    ///
    /// # Errors
    ///
    /// Returns [`WindowConfigError::PacketCapacity`] when the packet bound is
    /// zero or too large, or [`WindowConfigError::MaxAge`] when the retention
    /// age is zero or exceeds ten seconds.
    pub fn validate(self) -> Result<Self, WindowConfigError> {
        if self.max_packets == 0 || self.max_packets > MAX_WINDOW_PACKETS {
            return Err(WindowConfigError::PacketCapacity(self.max_packets));
        }
        if self.max_age_us == 0 || self.max_age_us > 10_000_000 {
            return Err(WindowConfigError::MaxAge(self.max_age_us));
        }
        Ok(self)
    }
}

impl Default for RetransmissionConfig {
    fn default() -> Self {
        Self {
            max_packets: 512,
            max_age_us: 250_000,
        }
    }
}

#[derive(Clone, Debug)]
struct Entry<T> {
    sent_at_us: u64,
    value: T,
}

/// A packet-count and deadline-bounded retransmission cache.
#[derive(Clone, Debug)]
pub struct RetransmissionWindow<T> {
    entries: BTreeMap<u64, Entry<T>>,
    config: RetransmissionConfig,
}

impl<T> RetransmissionWindow<T> {
    /// Creates an empty bounded retransmission window.
    ///
    /// # Errors
    ///
    /// Returns a [`WindowConfigError`] when `config` contains an invalid
    /// packet-count or retention-age limit.
    pub fn new(config: RetransmissionConfig) -> Result<Self, WindowConfigError> {
        Ok(Self {
            entries: BTreeMap::new(),
            config: config.validate()?,
        })
    }

    pub fn insert(&mut self, sequence: u64, sent_at_us: u64, value: T) -> Option<T> {
        self.expire(sent_at_us);
        let replaced = self
            .entries
            .insert(sequence, Entry { sent_at_us, value })
            .map(|entry| entry.value);
        while self.entries.len() > self.config.max_packets {
            let _removed = self.entries.pop_first();
        }
        replaced
    }

    #[must_use]
    pub fn get(&self, sequence: u64, now_us: u64) -> Option<&T> {
        self.entries.get(&sequence).and_then(|entry| {
            (now_us.saturating_sub(entry.sent_at_us) <= self.config.max_age_us)
                .then_some(&entry.value)
        })
    }

    pub fn acknowledge_through(&mut self, sequence: u64) {
        let retained = self.entries.split_off(&sequence.saturating_add(1));
        self.entries = retained;
    }

    pub fn expire(&mut self, now_us: u64) {
        let max_age = self.config.max_age_us;
        self.entries
            .retain(|_, entry| now_us.saturating_sub(entry.sent_at_us) <= max_age);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum WindowConfigError {
    #[error("retransmission packet capacity {0} is outside 1..={MAX_WINDOW_PACKETS}")]
    PacketCapacity(usize),
    #[error("retransmission age {0}us is outside 1..=10000000us")]
    MaxAge(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_by_count_and_refuses_expired_frames() {
        let mut window = RetransmissionWindow::new(RetransmissionConfig {
            max_packets: 2,
            max_age_us: 100,
        })
        .unwrap_or_else(|error| panic!("{error}"));
        window.insert(1, 0, "one");
        window.insert(2, 1, "two");
        window.insert(3, 2, "three");
        assert_eq!(window.get(1, 2), None);
        assert_eq!(window.get(2, 50), Some(&"two"));
        assert_eq!(window.get(2, 102), None);
    }

    #[test]
    fn cumulative_ack_removes_only_acknowledged_sequences() {
        let mut window = RetransmissionWindow::new(RetransmissionConfig::default())
            .unwrap_or_else(|error| panic!("{error}"));
        window.insert(10, 0, 10);
        window.insert(11, 0, 11);
        window.acknowledge_through(10);
        assert_eq!(window.get(10, 0), None);
        assert_eq!(window.get(11, 0), Some(&11));
    }
}
