use sanser_protocol::PacketPriority;
use std::{array, collections::VecDeque};

const LANE_COUNT: usize = 6;

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
#[derive(Clone, Debug)]
pub struct BoundedPriorityQueue<T> {
    queues: [VecDeque<T>; LANE_COUNT],
    capacities: QueueCapacities,
}

impl<T> BoundedPriorityQueue<T> {
    #[must_use]
    pub fn new(capacities: QueueCapacities) -> Self {
        Self {
            queues: array::from_fn(|_| VecDeque::new()),
            capacities,
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

    /// Pops the highest-priority available item.
    pub fn pop(&mut self) -> Option<(PacketPriority, T)> {
        const PRIORITIES: [PacketPriority; LANE_COUNT] = [
            PacketPriority::ReliableInput,
            PacketPriority::RealtimeInput,
            PacketPriority::Audio,
            PacketPriority::VideoControl,
            PacketPriority::VideoPayload,
            PacketPriority::Diagnostics,
        ];
        for priority in PRIORITIES {
            if let Some(item) = self.queues[lane(priority)].pop_front() {
                return Some((priority, item));
            }
        }
        None
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
    fn realtime_lanes_drop_oldest_but_reliable_input_never_drops() {
        let capacities = QueueCapacities::new(1, 1, 1, 1, 1, 1);
        let mut queue = BoundedPriorityQueue::new(capacities);
        assert_eq!(queue.push(PacketPriority::RealtimeInput, 1), Ok(None));
        assert_eq!(queue.push(PacketPriority::RealtimeInput, 2), Ok(Some(1)));
        assert_eq!(queue.push(PacketPriority::ReliableInput, 3), Ok(None));
        assert_eq!(
            queue.push(PacketPriority::ReliableInput, 4),
            Err(QueueFull(4))
        );
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
}
