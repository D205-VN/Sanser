#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelKind {
    ReliableInput,
    RealtimeInput,
    Control,
    Clipboard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataChannelPolicy {
    pub ordered: bool,
    pub max_retransmits: Option<u16>,
}

impl DataChannelPolicy {
    #[must_use]
    pub const fn for_kind(kind: ChannelKind) -> Self {
        match kind {
            ChannelKind::RealtimeInput => Self {
                ordered: false,
                max_retransmits: Some(0),
            },
            ChannelKind::ReliableInput | ChannelKind::Control | ChannelKind::Clipboard => Self {
                ordered: true,
                max_retransmits: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_are_reliable_but_movement_is_latest_state() {
        assert_eq!(
            DataChannelPolicy::for_kind(ChannelKind::ReliableInput),
            DataChannelPolicy {
                ordered: true,
                max_retransmits: None
            }
        );
        assert_eq!(
            DataChannelPolicy::for_kind(ChannelKind::RealtimeInput).max_retransmits,
            Some(0)
        );
    }
}
