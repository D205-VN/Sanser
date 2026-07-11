use crate::{GamepadState, InputEvent, MouseMove};
use std::collections::VecDeque;
use thiserror::Error;

const MAX_RELIABLE_EVENTS: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputBufferConfig {
    pub reliable_capacity: usize,
}

impl Default for InputBufferConfig {
    fn default() -> Self {
        Self {
            reliable_capacity: 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushOutcome {
    Enqueued,
    ReplacedStale,
    Unchanged,
}

#[derive(Clone, Debug, Error, PartialEq)]
#[error("reliable input queue is full")]
pub struct InputQueueFull(pub InputEvent);

/// Reliable events and latest-state events are held in separate bounded lanes.
#[derive(Clone, Debug)]
pub struct InputBuffer {
    reliable: VecDeque<InputEvent>,
    latest_mouse: Option<MouseMove>,
    latest_gamepad: Option<GamepadState>,
    reliable_capacity: usize,
}

impl InputBuffer {
    /// Creates separate bounded lanes for reliable and latest-state input.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidInputBufferConfig`] when `reliable_capacity` is zero or
    /// greater than the maximum of 16,384 queued reliable events.
    pub fn new(config: InputBufferConfig) -> Result<Self, InvalidInputBufferConfig> {
        if config.reliable_capacity == 0 || config.reliable_capacity > MAX_RELIABLE_EVENTS {
            return Err(InvalidInputBufferConfig(config.reliable_capacity));
        }
        Ok(Self {
            reliable: VecDeque::with_capacity(config.reliable_capacity.min(256)),
            latest_mouse: None,
            latest_gamepad: None,
            reliable_capacity: config.reliable_capacity,
        })
    }

    /// Adds an input event using reliable queuing or latest-state replacement.
    ///
    /// # Errors
    ///
    /// Returns [`InputQueueFull`] with ownership of the rejected event when the
    /// reliable lane is at capacity. Mouse movement and gamepad state use their
    /// own single-value lanes and therefore do not produce this error.
    pub fn push(&mut self, event: InputEvent) -> Result<PushOutcome, InputQueueFull> {
        match event {
            InputEvent::MouseMove(movement) => {
                let outcome = if self.latest_mouse.replace(movement).is_some() {
                    PushOutcome::ReplacedStale
                } else {
                    PushOutcome::Enqueued
                };
                Ok(outcome)
            }
            InputEvent::Gamepad(state) => {
                if self.latest_gamepad.as_ref() == Some(&state) {
                    return Ok(PushOutcome::Unchanged);
                }
                let outcome = if self.latest_gamepad.replace(state).is_some() {
                    PushOutcome::ReplacedStale
                } else {
                    PushOutcome::Enqueued
                };
                Ok(outcome)
            }
            reliable => {
                if self.reliable.len() >= self.reliable_capacity {
                    return Err(InputQueueFull(reliable));
                }
                self.reliable.push_back(reliable);
                Ok(PushOutcome::Enqueued)
            }
        }
    }

    /// Reliable key/button/wheel events always leave before lossy state updates.
    pub fn pop(&mut self) -> Option<InputEvent> {
        self.reliable
            .pop_front()
            .or_else(|| self.latest_mouse.take().map(InputEvent::MouseMove))
            .or_else(|| self.latest_gamepad.take().map(InputEvent::Gamepad))
    }

    pub fn discard_lossy_state(&mut self) {
        self.latest_mouse = None;
        self.latest_gamepad = None;
    }

    pub fn clear(&mut self) {
        self.reliable.clear();
        self.discard_lossy_state();
    }

    #[must_use]
    pub fn reliable_len(&self) -> usize {
        self.reliable.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reliable.is_empty() && self.latest_mouse.is_none() && self.latest_gamepad.is_none()
    }
}

impl Default for InputBuffer {
    fn default() -> Self {
        // The default is statically valid.
        Self {
            reliable: VecDeque::with_capacity(256),
            latest_mouse: None,
            latest_gamepad: None,
            reliable_capacity: InputBufferConfig::default().reliable_capacity,
        }
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("reliable input capacity {0} is outside 1..={MAX_RELIABLE_EVENTS}")]
pub struct InvalidInputBufferConfig(pub usize);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyAction, KeyEvent, Modifiers, MouseButton, MouseButtonEvent, MousePosition};

    fn mouse(sequence: u16) -> InputEvent {
        InputEvent::MouseMove(MouseMove {
            position: MousePosition::Relative {
                delta_x: f64::from(sequence),
                delta_y: 0.0,
            },
            captured_at_us: u64::from(sequence),
        })
    }

    #[test]
    fn mouse_movement_is_latest_state_wins() {
        let mut buffer = InputBuffer::default();
        assert_eq!(buffer.push(mouse(1)), Ok(PushOutcome::Enqueued));
        assert_eq!(buffer.push(mouse(2)), Ok(PushOutcome::ReplacedStale));
        assert_eq!(buffer.pop(), Some(mouse(2)));
        assert!(buffer.is_empty());
    }

    #[test]
    fn clicks_and_keys_preserve_order_and_are_not_dropped() {
        let mut buffer = InputBuffer::new(InputBufferConfig {
            reliable_capacity: 2,
        })
        .unwrap_or_else(|error| panic!("{error}"));
        let click = InputEvent::MouseButton(MouseButtonEvent {
            button: MouseButton::Left,
            pressed: true,
            captured_at_us: 1,
        });
        let key = InputEvent::Keyboard(KeyEvent {
            hid_usage: 4,
            action: KeyAction::Up,
            modifiers: Modifiers::empty(),
            captured_at_us: 2,
        });
        buffer
            .push(click.clone())
            .unwrap_or_else(|_| panic!("unexpected full queue"));
        buffer
            .push(key.clone())
            .unwrap_or_else(|_| panic!("unexpected full queue"));
        assert_eq!(buffer.push(mouse(3)), Ok(PushOutcome::Enqueued));
        assert_eq!(buffer.pop(), Some(click));
        assert_eq!(buffer.pop(), Some(key));
        assert_eq!(buffer.pop(), Some(mouse(3)));
    }

    #[test]
    fn full_reliable_queue_returns_event_to_apply_backpressure() {
        let mut buffer = InputBuffer::new(InputBufferConfig {
            reliable_capacity: 1,
        })
        .unwrap_or_else(|error| panic!("{error}"));
        let first = InputEvent::Keyboard(KeyEvent {
            hid_usage: 4,
            action: KeyAction::Down,
            modifiers: Modifiers::empty(),
            captured_at_us: 1,
        });
        let second = InputEvent::Keyboard(KeyEvent {
            hid_usage: 5,
            action: KeyAction::Down,
            modifiers: Modifiers::empty(),
            captured_at_us: 2,
        });
        assert_eq!(buffer.push(first), Ok(PushOutcome::Enqueued));
        assert_eq!(buffer.push(second.clone()), Err(InputQueueFull(second)));
    }
}
