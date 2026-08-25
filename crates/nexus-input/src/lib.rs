//! OS-independent semantic keyboard and pointer input events.

mod events;

pub use events::{InputError, InputEvent, KeyAction, Modifiers, MouseButton};

pub fn init() {
    // Initializer stub for nexus-input
}
