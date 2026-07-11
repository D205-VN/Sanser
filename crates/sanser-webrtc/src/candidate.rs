use std::collections::{HashSet, VecDeque};
use thiserror::Error;

const MAX_CANDIDATE_LEN: usize = 2_048;
const MAX_CANDIDATES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CandidateType {
    Host,
    ServerReflexive,
    PeerReflexive,
    Relay,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct IceCandidate {
    raw: String,
    pub component: u8,
    pub transport: String,
    pub port: u16,
    pub candidate_type: CandidateType,
}

impl IceCandidate {
    /// Parses and validates one untrusted SDP ICE candidate line.
    ///
    /// # Errors
    ///
    /// Returns a specific [`CandidateError`] when the line is empty, oversized,
    /// contains forbidden control bytes, has malformed candidate fields, or
    /// declares an unsupported component, transport, address, port, or type.
    pub fn parse(raw: String) -> Result<Self, CandidateError> {
        if raw.is_empty()
            || raw.len() > MAX_CANDIDATE_LEN
            || raw
                .chars()
                .any(|character| matches!(character, '\r' | '\n' | '\0'))
        {
            return Err(CandidateError::InvalidEncoding);
        }
        let parts: Vec<_> = raw.split_ascii_whitespace().collect();
        if parts.len() < 8 || !parts[0].starts_with("candidate:") || parts[6] != "typ" {
            return Err(CandidateError::InvalidSyntax);
        }
        let component = parts[1]
            .parse::<u8>()
            .map_err(|_| CandidateError::InvalidComponent)?;
        if !matches!(component, 1 | 2) {
            return Err(CandidateError::InvalidComponent);
        }
        let transport = parts[2].to_ascii_lowercase();
        if !matches!(transport.as_str(), "udp" | "tcp") {
            return Err(CandidateError::InvalidTransport);
        }
        let _priority = parts[3]
            .parse::<u32>()
            .map_err(|_| CandidateError::InvalidPriority)?;
        if parts[4].is_empty() || parts[4].len() > 255 {
            return Err(CandidateError::InvalidAddress);
        }
        let port = parts[5]
            .parse::<u16>()
            .map_err(|_| CandidateError::InvalidPort)?;
        if port == 0 {
            return Err(CandidateError::InvalidPort);
        }
        let candidate_type = match parts[7] {
            "host" => CandidateType::Host,
            "srflx" => CandidateType::ServerReflexive,
            "prflx" => CandidateType::PeerReflexive,
            "relay" => CandidateType::Relay,
            _ => return Err(CandidateError::InvalidType),
        };
        Ok(Self {
            raw,
            component,
            transport,
            port,
            candidate_type,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateBufferConfig {
    pub capacity: usize,
}

impl Default for CandidateBufferConfig {
    fn default() -> Self {
        Self { capacity: 128 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidatePush {
    Added,
    Duplicate,
}

#[derive(Clone, Debug)]
pub struct CandidateBuffer {
    queue: VecDeque<IceCandidate>,
    fingerprints: HashSet<String>,
    capacity: usize,
}

impl CandidateBuffer {
    /// Creates a bounded candidate buffer with duplicate suppression.
    ///
    /// # Errors
    ///
    /// Returns [`CandidateError::InvalidCapacity`] when `capacity` is zero or
    /// exceeds the crate-wide maximum of 512 candidates.
    pub fn new(config: CandidateBufferConfig) -> Result<Self, CandidateError> {
        if config.capacity == 0 || config.capacity > MAX_CANDIDATES {
            return Err(CandidateError::InvalidCapacity(config.capacity));
        }
        Ok(Self {
            queue: VecDeque::with_capacity(config.capacity.min(128)),
            fingerprints: HashSet::with_capacity(config.capacity.min(128)),
            capacity: config.capacity,
        })
    }

    /// Enqueues a candidate unless the same raw candidate is already buffered.
    ///
    /// # Errors
    ///
    /// Returns [`CandidateError::BufferFull`] when a distinct candidate is
    /// pushed after the configured capacity has been reached. A duplicate is
    /// reported as [`CandidatePush::Duplicate`] and is not an error.
    pub fn push(&mut self, candidate: IceCandidate) -> Result<CandidatePush, CandidateError> {
        if self.fingerprints.contains(candidate.as_str()) {
            return Ok(CandidatePush::Duplicate);
        }
        if self.queue.len() == self.capacity {
            return Err(CandidateError::BufferFull);
        }
        self.fingerprints.insert(candidate.raw.clone());
        self.queue.push_back(candidate);
        Ok(CandidatePush::Added)
    }

    pub fn pop(&mut self) -> Option<IceCandidate> {
        let candidate = self.queue.pop_front()?;
        self.fingerprints.remove(candidate.as_str());
        Some(candidate)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CandidateError {
    #[error("ICE candidate encoding is invalid")]
    InvalidEncoding,
    #[error("ICE candidate syntax is invalid")]
    InvalidSyntax,
    #[error("ICE candidate component is invalid")]
    InvalidComponent,
    #[error("ICE candidate transport is invalid")]
    InvalidTransport,
    #[error("ICE candidate priority is invalid")]
    InvalidPriority,
    #[error("ICE candidate address is invalid")]
    InvalidAddress,
    #[error("ICE candidate port is invalid")]
    InvalidPort,
    #[error("ICE candidate type is invalid")]
    InvalidType,
    #[error("ICE candidate capacity {0} is outside 1..=512")]
    InvalidCapacity(usize),
    #[error("ICE candidate buffer is full")]
    BufferFull,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(port: u16) -> IceCandidate {
        IceCandidate::parse(format!(
            "candidate:1 1 UDP 2122260223 192.168.1.2 {port} typ host"
        ))
        .unwrap_or_else(|error| panic!("candidate parse failed: {error}"))
    }

    #[test]
    fn trickle_buffer_is_bounded_and_deduplicated() {
        let mut buffer = CandidateBuffer::new(CandidateBufferConfig { capacity: 1 })
            .unwrap_or_else(|error| panic!("buffer setup failed: {error}"));
        assert_eq!(buffer.push(candidate(5_000)), Ok(CandidatePush::Added));
        assert_eq!(buffer.push(candidate(5_000)), Ok(CandidatePush::Duplicate));
        assert_eq!(
            buffer.push(candidate(5_001)),
            Err(CandidateError::BufferFull)
        );
    }
}
