use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MousePosition {
    Absolute {
        normalized_x: f64,
        normalized_y: f64,
    },
    Relative {
        delta_x: f64,
        delta_y: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MouseMove {
    pub position: MousePosition,
    pub captured_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Extra(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MouseButtonEvent {
    pub button: MouseButton,
    pub pressed: bool,
    pub captured_at_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WheelEvent {
    pub horizontal: f64,
    pub vertical: f64,
    pub captured_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Down,
    Up,
    Repeat,
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    pub struct Modifiers: u16 {
        const LEFT_SHIFT = 1 << 0;
        const RIGHT_SHIFT = 1 << 1;
        const LEFT_CONTROL = 1 << 2;
        const RIGHT_CONTROL = 1 << 3;
        const LEFT_ALT = 1 << 4;
        const RIGHT_ALT = 1 << 5;
        const LEFT_META = 1 << 6;
        const RIGHT_META = 1 << 7;
        const CAPS_LOCK = 1 << 8;
        const NUM_LOCK = 1 << 9;
    }
}

/// USB HID usage code plus location-independent modifier state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub hid_usage: u16,
    pub action: KeyAction,
    pub modifiers: Modifiers,
    pub captured_at_us: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GamepadState {
    pub controller_id: u32,
    pub connected: bool,
    pub buttons: u32,
    pub axes: [f32; 6],
    pub captured_at_us: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum InputEvent {
    MouseMove(MouseMove),
    MouseButton(MouseButtonEvent),
    Wheel(WheelEvent),
    Keyboard(KeyEvent),
    Gamepad(GamepadState),
}

impl InputEvent {
    /// Validates untrusted input values before native injection or buffering.
    ///
    /// # Errors
    ///
    /// Returns an [`InputValidationError`] when mouse coordinates or wheel
    /// deltas are non-finite or out of range, an extra mouse button is zero, a
    /// keyboard HID usage is zero, or a gamepad identifier or axis is invalid.
    pub fn validate(&self) -> Result<(), InputValidationError> {
        match self {
            Self::MouseMove(movement) => match movement.position {
                MousePosition::Absolute {
                    normalized_x,
                    normalized_y,
                } => {
                    if !normalized_x.is_finite()
                        || !normalized_y.is_finite()
                        || !(0.0..=1.0).contains(&normalized_x)
                        || !(0.0..=1.0).contains(&normalized_y)
                    {
                        return Err(InputValidationError::MousePosition);
                    }
                }
                MousePosition::Relative { delta_x, delta_y } => {
                    if !delta_x.is_finite()
                        || !delta_y.is_finite()
                        || delta_x.abs() > 32_768.0
                        || delta_y.abs() > 32_768.0
                    {
                        return Err(InputValidationError::MousePosition);
                    }
                }
            },
            Self::MouseButton(button) => {
                if matches!(button.button, MouseButton::Extra(0)) {
                    return Err(InputValidationError::MouseButton);
                }
            }
            Self::Wheel(wheel) => {
                if !wheel.horizontal.is_finite()
                    || !wheel.vertical.is_finite()
                    || wheel.horizontal.abs() > 10_000.0
                    || wheel.vertical.abs() > 10_000.0
                {
                    return Err(InputValidationError::Wheel);
                }
            }
            Self::Keyboard(key) => {
                if key.hid_usage == 0 {
                    return Err(InputValidationError::HidUsage);
                }
            }
            Self::Gamepad(state) => {
                if state.controller_id > 15
                    || state
                        .axes
                        .iter()
                        .any(|axis| !axis.is_finite() || !(-1.0..=1.0).contains(axis))
                {
                    return Err(InputValidationError::Gamepad);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum InputValidationError {
    #[error("mouse position is not finite or is outside the supported range")]
    MousePosition,
    #[error("mouse button is invalid")]
    MouseButton,
    #[error("wheel delta is not finite or is outside the supported range")]
    Wheel,
    #[error("keyboard HID usage is invalid")]
    HidUsage,
    #[error("gamepad identifier or axis is invalid")]
    Gamepad,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_finite_network_input() {
        let event = InputEvent::MouseMove(MouseMove {
            position: MousePosition::Relative {
                delta_x: f64::NAN,
                delta_y: 0.0,
            },
            captured_at_us: 1,
        });
        assert_eq!(event.validate(), Err(InputValidationError::MousePosition));
    }
}
