//! Input primitives intended for native capture/injection paths, not frontend JS.

mod buffer;
mod event;
mod pressed;

pub use buffer::{InputBuffer, InputBufferConfig, InputQueueFull, PushOutcome};
pub use event::{
    GamepadState, InputEvent, InputValidationError, KeyAction, KeyEvent, Modifiers, MouseButton,
    MouseButtonEvent, MouseMove, MousePosition, WheelEvent,
};
pub use pressed::{PressedState, PressedStateFull};
