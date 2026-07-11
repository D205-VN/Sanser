use crate::SessionMetrics;
use std::collections::VecDeque;
use thiserror::Error;

const MAX_HISTORY_SAMPLES: usize = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryConfig {
    pub max_samples: usize,
}

impl HistoryConfig {
    /// Validates the configured history capacity.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryConfigError`] when `max_samples` is zero or exceeds the
    /// supported maximum.
    pub fn validate(self) -> Result<Self, HistoryConfigError> {
        if self.max_samples == 0 || self.max_samples > MAX_HISTORY_SAMPLES {
            return Err(HistoryConfigError(self.max_samples));
        }
        Ok(self)
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { max_samples: 600 }
    }
}

#[derive(Clone, Debug)]
pub struct DiagnosticsHistory {
    samples: VecDeque<SessionMetrics>,
    config: HistoryConfig,
}

impl DiagnosticsHistory {
    /// Creates a bounded metrics history.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryConfigError`] when the requested sample capacity is
    /// outside the supported range.
    pub fn new(config: HistoryConfig) -> Result<Self, HistoryConfigError> {
        Ok(Self {
            samples: VecDeque::with_capacity(config.max_samples.min(1_024)),
            config: config.validate()?,
        })
    }

    /// Adds a sample and returns the stale sample if the bound was reached.
    pub fn push(&mut self, sample: SessionMetrics) -> Option<SessionMetrics> {
        let evicted = (self.samples.len() == self.config.max_samples)
            .then(|| self.samples.pop_front())
            .flatten();
        self.samples.push_back(sample);
        evicted
    }

    #[must_use]
    pub fn samples(&self) -> impl ExactSizeIterator<Item = &SessionMetrics> {
        self.samples.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

impl Default for DiagnosticsHistory {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(600),
            config: HistoryConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("diagnostics history capacity {0} is outside 1..=10000")]
pub struct HistoryConfigError(pub usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_oldest_sample_at_capacity() {
        let mut history = DiagnosticsHistory::new(HistoryConfig { max_samples: 2 })
            .unwrap_or_else(|error| panic!("history config failed: {error}"));
        for sampled_at_us in 1..=3 {
            history.push(SessionMetrics {
                sampled_at_us,
                ..SessionMetrics::default()
            });
        }
        let timestamps: Vec<_> = history
            .samples()
            .map(|sample| sample.sampled_at_us)
            .collect();
        assert_eq!(timestamps, [2, 3]);
    }
}
