use thiserror::Error;

const MAX_TARGET_US: u64 = 500_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JitterConfig {
    pub minimum_target_us: u64,
    pub maximum_target_us: u64,
    pub safety_margin_us: u64,
}

impl JitterConfig {
    pub fn validate(self) -> Result<Self, JitterConfigError> {
        if self.minimum_target_us == 0
            || self.minimum_target_us > self.maximum_target_us
            || self.maximum_target_us > MAX_TARGET_US
        {
            return Err(JitterConfigError::TargetRange {
                minimum_us: self.minimum_target_us,
                maximum_us: self.maximum_target_us,
            });
        }
        if self.safety_margin_us > self.maximum_target_us {
            return Err(JitterConfigError::SafetyMargin(self.safety_margin_us));
        }
        Ok(self)
    }
}

impl Default for JitterConfig {
    fn default() -> Self {
        Self {
            minimum_target_us: 20_000,
            maximum_target_us: 120_000,
            safety_margin_us: 10_000,
        }
    }
}

/// RFC3550-style integer jitter estimate with a clamped playback target.
#[derive(Clone, Debug)]
pub struct AdaptiveJitter {
    config: JitterConfig,
    previous_transit_us: Option<i128>,
    estimate_us_x16: u128,
}

impl AdaptiveJitter {
    pub fn new(config: JitterConfig) -> Result<Self, JitterConfigError> {
        Ok(Self {
            config: config.validate()?,
            previous_transit_us: None,
            estimate_us_x16: 0,
        })
    }

    pub fn observe(&mut self, sent_at_us: u64, arrived_at_us: u64) {
        let transit = i128::from(arrived_at_us) - i128::from(sent_at_us);
        if let Some(previous) = self.previous_transit_us {
            let delta = transit.abs_diff(previous);
            self.estimate_us_x16 = self
                .estimate_us_x16
                .saturating_add(delta)
                .saturating_sub(self.estimate_us_x16 / 16);
        }
        self.previous_transit_us = Some(transit);
    }

    #[must_use]
    pub fn estimated_jitter_us(&self) -> u64 {
        u64::try_from(self.estimate_us_x16 / 16).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn target_delay_us(&self) -> u64 {
        self.estimated_jitter_us()
            .saturating_mul(2)
            .saturating_add(self.config.safety_margin_us)
            .clamp(self.config.minimum_target_us, self.config.maximum_target_us)
    }
}

impl Default for AdaptiveJitter {
    fn default() -> Self {
        Self {
            config: JitterConfig::default(),
            previous_transit_us: None,
            estimate_us_x16: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum JitterConfigError {
    #[error("jitter target range {minimum_us}..={maximum_us}us is invalid")]
    TargetRange { minimum_us: u64, maximum_us: u64 },
    #[error("jitter safety margin {0}us exceeds the maximum target")]
    SafetyMargin(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_grows_but_remains_bounded() {
        let mut jitter = AdaptiveJitter::default();
        jitter.observe(0, 10_000);
        jitter.observe(10_000, 100_000);
        assert!(jitter.target_delay_us() > 20_000);
        for sequence in 0..100 {
            jitter.observe(sequence, sequence.saturating_mul(100_000));
        }
        assert!(jitter.target_delay_us() <= 120_000);
    }
}
