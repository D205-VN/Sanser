use sanser_protocol::PacketPriority;
use std::{array, collections::VecDeque};

const LANE_COUNT: usize = 6;
const PRIORITIES: [PacketPriority; LANE_COUNT] = [
    PacketPriority::ReliableInput,
    PacketPriority::RealtimeInput,
    PacketPriority::Audio,
    PacketPriority::VideoControl,
    PacketPriority::VideoPayload,
    PacketPriority::Diagnostics,
];

// Smooth weighted round-robin keeps interactive traffic responsive without
// allowing a permanently busy input lane to starve media or diagnostics. The
// absolute values are intentionally small; only their ratios are significant.
// The lane order is ReliableInput:RealtimeInput:Audio:VideoControl:
// VideoPayload:Diagnostics.
const SCHEDULING_WEIGHTS: [i32; LANE_COUNT] = [16, 12, 8, 6, 4, 2];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueCapacities {
    lanes: [usize; LANE_COUNT],
}

impl QueueCapacities {
    #[must_use]
    pub const fn new(
        reliable_input: usize,
        realtime_input: usize,
        audio: usize,
        video_control: usize,
        video_payload: usize,
        diagnostics: usize,
    ) -> Self {
        Self {
            lanes: [
                reliable_input,
                realtime_input,
                audio,
                video_control,
                video_payload,
                diagnostics,
            ],
        }
    }

    #[must_use]
    pub const fn for_priority(self, priority: PacketPriority) -> usize {
        self.lanes[lane(priority)]
    }
}

impl Default for QueueCapacities {
    fn default() -> Self {
        // Video payload remains deliberately tiny: stale frames increase latency.
        Self::new(1_024, 2, 64, 128, 8, 64)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueueFull<T>(pub T);

/// Six physically separate queues prevent video payload from blocking input.
///
/// Reliable input and video-control lanes reject new data when full. Realtime
/// input, audio, video payload and diagnostics use drop-oldest semantics so
/// latency cannot grow without bound.
///
/// [`Self::pop`] uses smooth weighted round-robin scheduling. Input receives
/// the largest share and wins equal-score ties, while every continuously
/// non-empty lane has a positive weight and therefore makes bounded progress.
/// Empty lanes consume no bandwidth, so video can use the full scheduler when
/// interactive and control traffic is idle.
#[derive(Clone, Debug)]
pub struct BoundedPriorityQueue<T> {
    queues: [VecDeque<T>; LANE_COUNT],
    capacities: QueueCapacities,
    scheduler_scores: [i32; LANE_COUNT],
}

impl<T> BoundedPriorityQueue<T> {
    #[must_use]
    pub fn new(capacities: QueueCapacities) -> Self {
        Self {
            queues: array::from_fn(|_| VecDeque::new()),
            capacities,
            scheduler_scores: [0; LANE_COUNT],
        }
    }

    /// Returns the evicted oldest item for latency-sensitive lanes.
    ///
    /// # Errors
    ///
    /// Returns [`QueueFull`] with ownership of `item` when the selected lane
    /// has zero capacity or is a reliable lane that is already full.
    pub fn push(&mut self, priority: PacketPriority, item: T) -> Result<Option<T>, QueueFull<T>> {
        let index = lane(priority);
        let capacity = self.capacities.lanes[index];
        if capacity == 0 {
            return Err(QueueFull(item));
        }
        if self.queues[index].len() < capacity {
            self.queues[index].push_back(item);
            return Ok(None);
        }
        if is_drop_oldest(priority) {
            let dropped = self.queues[index].pop_front();
            self.queues[index].push_back(item);
            Ok(dropped)
        } else {
            Err(QueueFull(item))
        }
    }

    /// Pops one item using smooth weighted round-robin scheduling.
    ///
    /// Higher-priority lanes receive a larger service share and win score
    /// ties, but lower-priority lanes cannot be starved by sustained input.
    /// FIFO ordering is preserved within each lane.
    pub fn pop(&mut self) -> Option<(PacketPriority, T)> {
        let mut active_weight = 0;
        let mut selected = None;

        for (index, weight) in SCHEDULING_WEIGHTS.into_iter().enumerate() {
            if self.queues[index].is_empty() {
                // An idle lane must not bank credit and release a burst later.
                self.scheduler_scores[index] = 0;
                continue;
            }

            self.scheduler_scores[index] += weight;
            active_weight += weight;

            if selected
                .is_none_or(|best| self.scheduler_scores[index] > self.scheduler_scores[best])
            {
                selected = Some(index);
            }
        }

        let selected = selected?;
        self.scheduler_scores[selected] -= active_weight;
        let item = self.queues[selected].pop_front()?;

        // A completely idle scheduler starts the next burst without carrying
        // debt from a previous, unrelated burst.
        if self.is_empty() {
            self.scheduler_scores = [0; LANE_COUNT];
        }

        Some((PRIORITIES[selected], item))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.queues.iter().map(VecDeque::len).sum()
    }

    #[must_use]
    pub fn len_for(&self, priority: PacketPriority) -> usize {
        self.queues[lane(priority)].len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(VecDeque::is_empty)
    }

    pub fn clear(&mut self) {
        for queue in &mut self.queues {
            queue.clear();
        }
        self.scheduler_scores = [0; LANE_COUNT];
    }
}

impl<T> Default for BoundedPriorityQueue<T> {
    fn default() -> Self {
        Self::new(QueueCapacities::default())
    }
}

const fn lane(priority: PacketPriority) -> usize {
    priority as usize - 1
}

const fn is_drop_oldest(priority: PacketPriority) -> bool {
    matches!(
        priority,
        PacketPriority::RealtimeInput
            | PacketPriority::Audio
            | PacketPriority::VideoPayload
            | PacketPriority::Diagnostics
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lane_keeps_its_overflow_policy() {
        let capacities = QueueCapacities::new(1, 1, 1, 1, 1, 1);
        let mut queue = BoundedPriorityQueue::new(capacities);
        for priority in [PacketPriority::ReliableInput, PacketPriority::VideoControl] {
            assert_eq!(queue.push(priority, 1), Ok(None));
            assert_eq!(queue.push(priority, 2), Err(QueueFull(2)));
        }
        for priority in [
            PacketPriority::RealtimeInput,
            PacketPriority::Audio,
            PacketPriority::VideoPayload,
            PacketPriority::Diagnostics,
        ] {
            assert_eq!(queue.push(priority, 1), Ok(None));
            assert_eq!(queue.push(priority, 2), Ok(Some(1)));
        }
    }

    #[test]
    fn video_cannot_head_of_line_block_input() {
        let mut queue = BoundedPriorityQueue::default();
        queue
            .push(PacketPriority::VideoPayload, "video")
            .unwrap_or_else(|_| panic!("video lane unexpectedly full"));
        queue
            .push(PacketPriority::ReliableInput, "key-up")
            .unwrap_or_else(|_| panic!("input lane unexpectedly full"));
        assert_eq!(queue.pop(), Some((PacketPriority::ReliableInput, "key-up")));
    }

    #[test]
    fn all_lanes_remain_bounded() {
        let capacities = QueueCapacities::new(1, 2, 3, 4, 5, 6);
        let mut queue = BoundedPriorityQueue::new(capacities);
        for value in 0..100 {
            let _result = queue.push(PacketPriority::VideoPayload, value);
        }
        assert_eq!(queue.len_for(PacketPriority::VideoPayload), 5);
    }

    #[test]
    fn sustained_input_cannot_starve_any_lane() {
        let capacities = QueueCapacities::new(2, 2, 2, 2, 2, 2);
        let mut queue = BoundedPriorityQueue::new(capacities);
        for priority in PRIORITIES {
            queue
                .push(priority, priority)
                .unwrap_or_else(|_| panic!("lane unexpectedly full"));
        }

        let fairness_window: usize = SCHEDULING_WEIGHTS
            .iter()
            .map(|weight| {
                usize::try_from(*weight)
                    .unwrap_or_else(|_| panic!("scheduling weights must be positive"))
            })
            .sum();
        let observation_length = fairness_window * 8;
        let mut last_service = [None; LANE_COUNT];
        for sequence in 0..observation_length {
            let (priority, _) = queue
                .pop()
                .unwrap_or_else(|| panic!("continuously replenished queue became empty"));
            let index = lane(priority);
            if let Some(previous) = last_service[index] {
                assert!(
                    sequence - previous <= fairness_window,
                    "{priority:?} exceeded its fairness window"
                );
            }
            last_service[index] = Some(sequence);
            queue
                .push(priority, priority)
                .unwrap_or_else(|_| panic!("replenished lane unexpectedly full"));
        }

        for (index, last) in last_service.into_iter().enumerate() {
            let last = last.unwrap_or_else(|| panic!("{:?} was starved", PRIORITIES[index]));
            assert!(
                observation_length - last <= fairness_window,
                "{:?} stopped making progress",
                PRIORITIES[index]
            );
        }
    }

    #[test]
    fn input_latency_stays_bounded_while_all_lanes_are_busy() {
        let capacities = QueueCapacities::new(2, 2, 2, 2, 2, 2);
        let mut queue = BoundedPriorityQueue::new(capacities);
        for priority in PRIORITIES {
            queue
                .push(priority, priority)
                .unwrap_or_else(|_| panic!("lane unexpectedly full"));
        }

        let mut consecutive_non_input = 0;
        let mut worst_non_input_run = 0;
        for _ in 0..512 {
            let (priority, _) = queue
                .pop()
                .unwrap_or_else(|| panic!("continuously replenished queue became empty"));
            queue
                .push(priority, priority)
                .unwrap_or_else(|_| panic!("replenished lane unexpectedly full"));

            if matches!(
                priority,
                PacketPriority::ReliableInput | PacketPriority::RealtimeInput
            ) {
                consecutive_non_input = 0;
            } else {
                consecutive_non_input += 1;
                worst_non_input_run = worst_non_input_run.max(consecutive_non_input);
            }
        }

        assert!(
            worst_non_input_run <= 2,
            "input was delayed behind {worst_non_input_run} consecutive packets"
        );
    }

    #[test]
    fn fifo_order_is_preserved_inside_each_lane() {
        let capacities = QueueCapacities::new(4, 4, 4, 4, 4, 4);
        let mut queue = BoundedPriorityQueue::new(capacities);
        for value in 0..4 {
            queue
                .push(PacketPriority::Audio, value)
                .unwrap_or_else(|_| panic!("audio lane unexpectedly full"));
        }

        let values: Vec<_> = (0..4)
            .filter_map(|_| queue.pop().map(|(_, value)| value))
            .collect();
        assert_eq!(values, vec![0, 1, 2, 3]);
    }
}
