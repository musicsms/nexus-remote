//! Remote input receiver and semantic event dispatcher.
//! Part of Nexus Remote Desktop Platform.

use nexus_input::{InputEvent, KeyAction, Modifiers, MouseButton};
use nexus_protocol::{KeyEvent, MouseButton as ProtoMouseButton, MouseMove, MouseWheel, TextInput};
use prost::Message;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tracing::debug;

/// Errors occurring during input deserialization or dispatch.
#[derive(Debug, Error)]
pub enum InputHandlerError {
    #[error("Protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),

    #[error("Invalid input data: {0}")]
    InvalidData(String),
}

/// Dispatches remote inputs to OS subsystem or recording buffer.
pub struct HostInputHandler {
    events_received: AtomicU64,
}

impl Default for HostInputHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl HostInputHandler {
    /// Creates a new `HostInputHandler`.
    pub fn new() -> Self {
        Self {
            events_received: AtomicU64::new(0),
        }
    }

    /// Handles a raw serialized `KeyEvent` from QUIC control stream.
    pub fn handle_key_event(&self, bytes: &[u8]) -> Result<(), InputHandlerError> {
        let proto = KeyEvent::decode(bytes)?;
        let action = if proto.pressed {
            KeyAction::Down
        } else {
            KeyAction::Up
        };
        let event = InputEvent::Key {
            physical_code: proto.physical_code,
            logical_code: proto.logical_code,
            action,
            modifiers: Modifiers::from_bits(proto.modifiers),
        };
        self.dispatch_event(event);
        Ok(())
    }

    /// Handles a raw serialized `MouseMove` from QUIC control stream.
    pub fn handle_mouse_move(&self, bytes: &[u8]) -> Result<(), InputHandlerError> {
        let proto = MouseMove::decode(bytes)?;
        let event = InputEvent::MouseMove {
            x: proto.x,
            y: proto.y,
        };
        self.dispatch_event(event);
        Ok(())
    }

    /// Handles a raw serialized `MouseButton` from QUIC control stream.
    pub fn handle_mouse_button(&self, bytes: &[u8]) -> Result<(), InputHandlerError> {
        let proto = ProtoMouseButton::decode(bytes)?;
        let button = match proto.button {
            0 => MouseButton::Left,
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            3 => MouseButton::Back,
            4 => MouseButton::Forward,
            _ => MouseButton::Left,
        };
        let event = InputEvent::MouseButton {
            button,
            pressed: proto.pressed,
        };
        self.dispatch_event(event);
        Ok(())
    }

    /// Handles a raw serialized `MouseWheel` from QUIC control stream.
    pub fn handle_mouse_wheel(&self, bytes: &[u8]) -> Result<(), InputHandlerError> {
        let proto = MouseWheel::decode(bytes)?;
        let event = InputEvent::MouseWheel {
            delta_x: proto.delta_x,
            delta_y: proto.delta_y,
        };
        self.dispatch_event(event);
        Ok(())
    }

    /// Handles a raw serialized `TextInput` from QUIC control stream.
    pub fn handle_text_input(&self, bytes: &[u8]) -> Result<(), InputHandlerError> {
        let proto = TextInput::decode(bytes)?;
        let event = InputEvent::Text(proto.text);
        self.dispatch_event(event);
        Ok(())
    }

    fn dispatch_event(&self, event: InputEvent) {
        self.events_received.fetch_add(1, Ordering::Relaxed);
        debug!("Dispatched input event: {:?}", event);
    }

    /// Returns total count of processed input events.
    pub fn events_received(&self) -> u64 {
        self.events_received.load(Ordering::Relaxed)
    }
}
