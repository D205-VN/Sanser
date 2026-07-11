use crate::{InputEvent, KeyAction, KeyEvent, Modifiers, MouseButton, MouseButtonEvent};
use std::collections::BTreeSet;
use thiserror::Error;

const MAX_PRESSED_KEYS: usize = 512;
const MAX_PRESSED_MOUSE_BUTTONS: usize = 32;

/// Tracks remote pressed state and generates deterministic releases on teardown.
#[derive(Clone, Debug, Default)]
pub struct PressedState {
    keys: BTreeSet<u16>,
    mouse_buttons: BTreeSet<MouseButton>,
}

impl PressedState {
    /// Applies a reliable key or mouse-button event to the pressed-state set.
    ///
    /// Movement, wheel, gamepad, and key-repeat events do not change the set.
    ///
    /// # Errors
    ///
    /// Returns [`PressedStateFull::Keys`] before inserting a 513th distinct
    /// key, or [`PressedStateFull::MouseButtons`] before inserting a 33rd
    /// distinct mouse button. Releases and duplicate presses remain accepted.
    pub fn observe(&mut self, event: &InputEvent) -> Result<(), PressedStateFull> {
        match event {
            InputEvent::Keyboard(key) => match key.action {
                KeyAction::Down => {
                    if !self.keys.contains(&key.hid_usage) && self.keys.len() == MAX_PRESSED_KEYS {
                        return Err(PressedStateFull::Keys);
                    }
                    self.keys.insert(key.hid_usage);
                }
                KeyAction::Up => {
                    self.keys.remove(&key.hid_usage);
                }
                KeyAction::Repeat => {}
            },
            InputEvent::MouseButton(button) => {
                if button.pressed {
                    if !self.mouse_buttons.contains(&button.button)
                        && self.mouse_buttons.len() == MAX_PRESSED_MOUSE_BUTTONS
                    {
                        return Err(PressedStateFull::MouseButtons);
                    }
                    self.mouse_buttons.insert(button.button);
                } else {
                    self.mouse_buttons.remove(&button.button);
                }
            }
            InputEvent::MouseMove(_) | InputEvent::Wheel(_) | InputEvent::Gamepad(_) => {}
        }
        Ok(())
    }

    /// Clears local state and returns reliable releases for focus/network/session loss.
    pub fn release_all(&mut self, captured_at_us: u64) -> Vec<InputEvent> {
        let mut releases = Vec::with_capacity(self.keys.len() + self.mouse_buttons.len());
        for hid_usage in std::mem::take(&mut self.keys) {
            releases.push(InputEvent::Keyboard(KeyEvent {
                hid_usage,
                action: KeyAction::Up,
                modifiers: Modifiers::empty(),
                captured_at_us,
            }));
        }
        for button in std::mem::take(&mut self.mouse_buttons) {
            releases.push(InputEvent::MouseButton(MouseButtonEvent {
                button,
                pressed: false,
                captured_at_us,
            }));
        }
        releases
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.mouse_buttons.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PressedStateFull {
    #[error("pressed-key tracker reached its 512-key bound")]
    Keys,
    #[error("pressed mouse-button tracker reached its 32-button bound")]
    MouseButtons,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_releases_every_pressed_key_and_button_once() {
        let mut state = PressedState::default();
        state
            .observe(&InputEvent::Keyboard(KeyEvent {
                hid_usage: 225,
                action: KeyAction::Down,
                modifiers: Modifiers::LEFT_SHIFT,
                captured_at_us: 1,
            }))
            .unwrap_or_else(|error| panic!("observe failed: {error}"));
        state
            .observe(&InputEvent::MouseButton(MouseButtonEvent {
                button: MouseButton::Left,
                pressed: true,
                captured_at_us: 2,
            }))
            .unwrap_or_else(|error| panic!("observe failed: {error}"));
        let releases = state.release_all(3);
        assert_eq!(releases.len(), 2);
        assert!(state.is_empty());
        assert!(state.release_all(4).is_empty());
    }
}
