/// Number of sequence numbers retained in one stream's replay window.
pub const REPLAY_WINDOW_SIZE: u64 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayDecision {
    Fresh,
    Duplicate,
    TooOld,
}

/// Fixed-memory replay filter for one authenticated `(session, stream, direction)` tuple.
#[derive(Clone, Debug, Default)]
pub struct ReplayWindow {
    highest: Option<u64>,
    seen: u128,
}

impl ReplayWindow {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            highest: None,
            seen: 0,
        }
    }

    /// Records a sequence only when it is fresh.
    pub fn observe(&mut self, sequence: u64) -> ReplayDecision {
        let Some(highest) = self.highest else {
            self.highest = Some(sequence);
            self.seen = 1;
            return ReplayDecision::Fresh;
        };

        if sequence > highest {
            let shift = sequence - highest;
            self.seen = if shift >= REPLAY_WINDOW_SIZE {
                1
            } else {
                (self.seen << shift) | 1
            };
            self.highest = Some(sequence);
            return ReplayDecision::Fresh;
        }

        let offset = highest - sequence;
        if offset >= REPLAY_WINDOW_SIZE {
            return ReplayDecision::TooOld;
        }
        let mask = 1_u128 << offset;
        if self.seen & mask != 0 {
            ReplayDecision::Duplicate
        } else {
            self.seen |= mask;
            ReplayDecision::Fresh
        }
    }

    #[must_use]
    pub const fn highest(&self) -> Option<u64> {
        self.highest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_duplicates_and_accepts_reordering_inside_window() {
        let mut window = ReplayWindow::new();
        assert_eq!(window.observe(10), ReplayDecision::Fresh);
        assert_eq!(window.observe(12), ReplayDecision::Fresh);
        assert_eq!(window.observe(11), ReplayDecision::Fresh);
        assert_eq!(window.observe(11), ReplayDecision::Duplicate);
    }

    #[test]
    fn rejects_packets_older_than_window() {
        let mut window = ReplayWindow::new();
        assert_eq!(window.observe(1), ReplayDecision::Fresh);
        assert_eq!(window.observe(200), ReplayDecision::Fresh);
        assert_eq!(window.observe(1), ReplayDecision::TooOld);
    }
}
