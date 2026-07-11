use crate::{Nack, Packet, PacketFlags, PacketType};
use sanser_core::SessionId;
use std::collections::{BTreeMap, btree_map::Entry};
use thiserror::Error;

const MAX_REASSEMBLY_FRAMES: usize = 32;
const MAX_FRAGMENTS_PER_FRAME: usize = 4_096;
const MAX_REASSEMBLY_BYTES: usize = 64 * 1_048_576;
const MAX_REASSEMBLY_AGE_US: u64 = 2_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAssemblerConfig {
    pub max_frames: usize,
    pub max_fragments_per_frame: usize,
    pub max_bytes: usize,
    pub max_age_us: u64,
}

impl FrameAssemblerConfig {
    /// Validates that every configured limit is non-zero and within the
    /// protocol's hard safety bounds.
    ///
    /// # Errors
    ///
    /// Returns [`FrameAssemblerError::InvalidConfig`] when any limit is zero
    /// or exceeds its corresponding hard bound.
    pub fn validate(self) -> Result<Self, FrameAssemblerError> {
        if self.max_frames == 0
            || self.max_frames > MAX_REASSEMBLY_FRAMES
            || self.max_fragments_per_frame == 0
            || self.max_fragments_per_frame > MAX_FRAGMENTS_PER_FRAME
            || self.max_bytes == 0
            || self.max_bytes > MAX_REASSEMBLY_BYTES
            || self.max_age_us == 0
            || self.max_age_us > MAX_REASSEMBLY_AGE_US
        {
            return Err(FrameAssemblerError::InvalidConfig);
        }
        Ok(self)
    }
}

impl Default for FrameAssemblerConfig {
    fn default() -> Self {
        Self {
            max_frames: 3,
            max_fragments_per_frame: 1_024,
            max_bytes: 16 * 1_048_576,
            max_age_us: 250_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedFrame {
    pub frame_id: u64,
    pub timestamp_us: u64,
    pub key_frame: bool,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FramePush {
    Pending { evicted_frame: Option<u64> },
    Duplicate,
    Complete(CompletedFrame),
}

#[derive(Clone, Debug)]
struct PartialFrame {
    created_at_us: u64,
    timestamp_us: u64,
    key_frame: bool,
    end_sequence: Option<u64>,
    fragments: BTreeMap<u64, Vec<u8>>,
    bytes: usize,
}

/// Count-, byte- and deadline-bounded reassembly for one authenticated session.
#[derive(Clone, Debug)]
pub struct FrameAssembler {
    session_id: SessionId,
    frames: BTreeMap<u64, PartialFrame>,
    total_bytes: usize,
    latest_completed_frame: Option<u64>,
    config: FrameAssemblerConfig,
}

impl FrameAssembler {
    /// Creates a bounded frame assembler for one authenticated session.
    ///
    /// # Errors
    ///
    /// Returns [`FrameAssemblerError::SessionMismatch`] for an empty session
    /// identifier, or [`FrameAssemblerError::InvalidConfig`] when `config`
    /// lies outside the hard safety bounds.
    pub fn new(
        session_id: SessionId,
        config: FrameAssemblerConfig,
    ) -> Result<Self, FrameAssemblerError> {
        if session_id.as_uuid().is_nil() {
            return Err(FrameAssemblerError::SessionMismatch);
        }
        Ok(Self {
            session_id,
            frames: BTreeMap::new(),
            total_bytes: 0,
            latest_completed_frame: None,
            config: config.validate()?,
        })
    }

    /// Adds one authenticated video fragment and returns the current frame
    /// reassembly outcome.
    ///
    /// # Errors
    ///
    /// Returns an error for non-video, wrong-session or stale packets; when a
    /// byte or fragment bound would be exceeded; when a frame contains more
    /// than one end marker; or if the internal frame state is inconsistent.
    pub fn push(&mut self, packet: Packet, now_us: u64) -> Result<FramePush, FrameAssemblerError> {
        if packet.header.packet_type != PacketType::Video {
            return Err(FrameAssemblerError::NotVideo);
        }
        if packet.header.session_id != self.session_id {
            return Err(FrameAssemblerError::SessionMismatch);
        }
        if self
            .latest_completed_frame
            .is_some_and(|latest| packet.header.frame_id <= latest)
        {
            return Err(FrameAssemblerError::StaleFrame);
        }
        self.expire(now_us);
        if packet.payload.len() > self.config.max_bytes {
            return Err(FrameAssemblerError::ByteCapacity);
        }

        let mut evicted_frame = None;
        if !self.frames.contains_key(&packet.header.frame_id)
            && self.frames.len() == self.config.max_frames
        {
            evicted_frame = self.evict_oldest();
        }
        while self.total_bytes.saturating_add(packet.payload.len()) > self.config.max_bytes {
            let Some(evicted) = self.evict_oldest() else {
                return Err(FrameAssemblerError::ByteCapacity);
            };
            evicted_frame = Some(evicted);
        }

        let frame = match self.frames.entry(packet.header.frame_id) {
            Entry::Vacant(entry) => entry.insert(PartialFrame {
                created_at_us: now_us,
                timestamp_us: packet.header.timestamp_us,
                key_frame: packet.header.flags.contains(PacketFlags::KEY_FRAME),
                end_sequence: None,
                fragments: BTreeMap::new(),
                bytes: 0,
            }),
            Entry::Occupied(entry) => entry.into_mut(),
        };
        if frame.fragments.contains_key(&packet.header.sequence) {
            return Ok(FramePush::Duplicate);
        }
        if frame.fragments.len() == self.config.max_fragments_per_frame {
            return Err(FrameAssemblerError::FragmentCapacity);
        }
        if packet.header.flags.contains(PacketFlags::END_OF_FRAME) {
            if frame.end_sequence.is_some() {
                return Err(FrameAssemblerError::MultipleEndFragments);
            }
            frame.end_sequence = Some(packet.header.sequence);
        }
        frame.bytes = frame.bytes.saturating_add(packet.payload.len());
        self.total_bytes = self.total_bytes.saturating_add(packet.payload.len());
        frame
            .fragments
            .insert(packet.header.sequence, packet.payload);

        if is_complete(frame) {
            let frame_id = packet.header.frame_id;
            let frame = self
                .frames
                .remove(&frame_id)
                .ok_or(FrameAssemblerError::InternalState)?;
            self.total_bytes = self.total_bytes.saturating_sub(frame.bytes);
            let mut payload = Vec::with_capacity(frame.bytes);
            for fragment in frame.fragments.into_values() {
                payload.extend_from_slice(&fragment);
            }
            self.latest_completed_frame = Some(frame_id);
            return Ok(FramePush::Complete(CompletedFrame {
                frame_id,
                timestamp_us: frame.timestamp_us,
                key_frame: frame.key_frame,
                payload,
            }));
        }
        Ok(FramePush::Pending { evicted_frame })
    }

    #[must_use]
    pub fn nack(&self, frame_id: u64) -> Option<Nack> {
        let frame = self.frames.get(&frame_id)?;
        let end = frame.end_sequence?;
        let start = *frame.fragments.first_key_value()?.0;
        let missing_sequences: Vec<_> = (start..=end)
            .filter(|sequence| !frame.fragments.contains_key(sequence))
            .take(crate::MAX_NACK_SEQUENCES)
            .collect();
        (!missing_sequences.is_empty()).then_some(Nack {
            frame_id,
            missing_sequences,
        })
    }

    pub fn expire(&mut self, now_us: u64) -> Vec<u64> {
        let expired: Vec<_> = self
            .frames
            .iter()
            .filter_map(|(frame_id, frame)| {
                (now_us.saturating_sub(frame.created_at_us) > self.config.max_age_us)
                    .then_some(*frame_id)
            })
            .collect();
        for frame_id in &expired {
            if let Some(frame) = self.frames.remove(frame_id) {
                self.total_bytes = self.total_bytes.saturating_sub(frame.bytes);
            }
        }
        expired
    }

    #[must_use]
    pub fn pending_frames(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub const fn buffered_bytes(&self) -> usize {
        self.total_bytes
    }

    fn evict_oldest(&mut self) -> Option<u64> {
        let frame_id = self
            .frames
            .iter()
            .min_by_key(|(frame_id, frame)| (frame.created_at_us, **frame_id))
            .map(|(frame_id, _)| *frame_id)?;
        let frame = self.frames.remove(&frame_id)?;
        self.total_bytes = self.total_bytes.saturating_sub(frame.bytes);
        Some(frame_id)
    }
}

fn is_complete(frame: &PartialFrame) -> bool {
    let Some(end) = frame.end_sequence else {
        return false;
    };
    let Some((&start, _)) = frame.fragments.first_key_value() else {
        return false;
    };
    let expected = end.saturating_sub(start).saturating_add(1);
    u64::try_from(frame.fragments.len()).is_ok_and(|count| count == expected)
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FrameAssemblerError {
    #[error("frame assembler configuration is invalid")]
    InvalidConfig,
    #[error("packet is not video")]
    NotVideo,
    #[error("packet belongs to another or empty session")]
    SessionMismatch,
    #[error("packet belongs to an already completed stale frame")]
    StaleFrame,
    #[error("frame exceeds bounded reassembly byte capacity")]
    ByteCapacity,
    #[error("frame exceeds bounded fragment capacity")]
    FragmentCapacity,
    #[error("frame contains multiple end fragments")]
    MultipleEndFragments,
    #[error("frame assembler internal state is inconsistent")]
    InternalState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PacketHeader;
    use sanser_core::StreamId;

    fn fragment(session_id: SessionId, sequence: u64, end: bool, payload: u8) -> Packet {
        Packet {
            header: PacketHeader {
                packet_type: PacketType::Video,
                session_id,
                stream_id: StreamId::VIDEO_PRIMARY,
                sequence,
                frame_id: 7,
                timestamp_us: 1,
                flags: if end {
                    PacketFlags::END_OF_FRAME
                } else {
                    PacketFlags::empty()
                },
                key_id: 1,
            },
            payload: vec![payload],
        }
    }

    #[test]
    fn reports_missing_fragment_then_reassembles_in_sequence_order() {
        let session = SessionId::new();
        let mut assembler = FrameAssembler::new(session, FrameAssemblerConfig::default())
            .unwrap_or_else(|error| panic!("assembler failed: {error}"));
        assert!(matches!(
            assembler.push(fragment(session, 10, false, 1), 0),
            Ok(FramePush::Pending { .. })
        ));
        assert!(matches!(
            assembler.push(fragment(session, 12, true, 3), 1),
            Ok(FramePush::Pending { .. })
        ));
        assert_eq!(
            assembler.nack(7).map(|nack| nack.missing_sequences),
            Some(vec![11])
        );
        let completed = assembler
            .push(fragment(session, 11, false, 2), 2)
            .unwrap_or_else(|error| panic!("push failed: {error}"));
        assert!(matches!(
            completed,
            FramePush::Complete(CompletedFrame { payload, .. }) if payload == [1, 2, 3]
        ));
    }

    #[test]
    fn expired_incomplete_frames_release_all_memory() {
        let session = SessionId::new();
        let mut assembler = FrameAssembler::new(
            session,
            FrameAssemblerConfig {
                max_age_us: 10,
                ..FrameAssemblerConfig::default()
            },
        )
        .unwrap_or_else(|error| panic!("assembler failed: {error}"));
        let _outcome = assembler
            .push(fragment(session, 1, false, 1), 1)
            .unwrap_or_else(|error| panic!("push failed: {error}"));
        assert_eq!(assembler.expire(12), vec![7]);
        assert_eq!(assembler.buffered_bytes(), 0);
    }
}
