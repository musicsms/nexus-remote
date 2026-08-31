use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const CTRL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const META: Self = Self(1 << 3);
    pub const fn bits(self) -> u32 {
        self.0 as u32
    }
    pub const fn from_bits(bits: u32) -> Self {
        Self((bits & 0x0f) as u8)
    }
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
    #[error("physical key code cannot be represented as a 16-bit value")]
    InvalidPhysicalKeyCode,
    #[error("mouse wheel delta is outside the allowed range")]
    WheelDeltaOutOfRange,
}

impl InputEvent {
    pub const MAX_TEXT_BYTES: usize = 4096;

    pub fn validate(&self) -> Result<(), InputError> {
        match self {
            Self::Text(text) if text.len() > Self::MAX_TEXT_BYTES => Err(InputError::TextTooLong {
                max_bytes: Self::MAX_TEXT_BYTES,
            }),
            Self::Key { physical_code, .. } if *physical_code > u16::MAX as u32 => {
                Err(InputError::InvalidPhysicalKeyCode)
            }
            Self::MouseWheel { delta_x, delta_y }
                if !(-120_000..=120_000).contains(delta_x)
                    || !(-120_000..=120_000).contains(delta_y) =>
            {
                Err(InputError::WheelDeltaOutOfRange)
            }
            _ => Ok(()),
        }
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

    #[test]
    fn modifier_bits_round_trip_canonically() {
        assert_eq!(Modifiers::SHIFT.bits(), 1);
        assert_eq!(Modifiers::CTRL.bits(), 2);
        assert_eq!(Modifiers::from_bits(0xF1).bits(), 1);
    }

    #[test]
    fn rejects_unrepresentable_physical_key_code() {
        let event = InputEvent::Key {
            physical_code: 0x1_0000,
            logical_code: 0,
            action: KeyAction::Down,
            modifiers: Modifiers::NONE,
        };
        assert_eq!(event.validate(), Err(InputError::InvalidPhysicalKeyCode));
    }

    #[test]
    fn rejects_extreme_wheel_delta() {
        let event = InputEvent::MouseWheel {
            delta_x: 0,
            delta_y: i32::MAX,
        };
        assert_eq!(event.validate(), Err(InputError::WheelDeltaOutOfRange));
    }
}
