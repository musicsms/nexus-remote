use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const CTRL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const META: Self = Self(1 << 3);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Key {
        physical_code: u32,
        logical_code: u32,
        action: KeyAction,
        modifiers: Modifiers,
    },
    Text(String),
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    MouseWheel {
        delta_x: i32,
        delta_y: i32,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("text input exceeds {max_bytes} bytes")]
    TextTooLong { max_bytes: usize },
}

impl InputEvent {
    pub const MAX_TEXT_BYTES: usize = 4096;

    pub fn validate(&self) -> Result<(), InputError> {
        if let Self::Text(text) = self {
            if text.len() > Self::MAX_TEXT_BYTES {
                return Err(InputError::TextTooLong {
                    max_bytes: Self::MAX_TEXT_BYTES,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_semantic_mouse_event() {
        assert!(InputEvent::MouseMove { x: -10, y: 20 }.validate().is_ok());
    }

    #[test]
    fn bounds_text_input() {
        let event = InputEvent::Text("x".repeat(InputEvent::MAX_TEXT_BYTES + 1));
        assert_eq!(
            event.validate(),
            Err(InputError::TextTooLong {
                max_bytes: InputEvent::MAX_TEXT_BYTES
            })
        );
    }
}
