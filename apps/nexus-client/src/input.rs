//! Portable Win32-event to semantic-control translation.
//!
//! This module deliberately contains no Windows handles.  The native window
//! thread emits [`WindowEvent`] values and this controller validates, bounds,
//! rate-limits, and frames them before they cross to the network task.

use crate::{ClientInputError, ClientInputSender, WindowEvent};
use nexus_protocol::MonitorInfo;
use platform_windows::CursorSnapshot;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use thiserror::Error;

pub const DEFAULT_INPUT_QUEUE_CAPACITY: usize = 64;
pub const DEFAULT_INPUT_RATE_PER_SECOND: usize = 240;
pub const MAX_INPUT_RATE_PER_SECOND: usize = 4096;
pub const MAX_INPUT_QUEUE_CAPACITY: usize = 1024;
pub const MAX_INPUT_CONTROL_BYTES: usize = 64 * 1024;

#[derive(Debug, Error, PartialEq)]
pub enum InputControllerError {
    #[error("input arrived while the client window is unfocused")]
    NotFocused,
    #[error("input controller has shut down")]
    Shutdown,
    #[error("input rate limit exceeded")]
    RateLimited,
    #[error("input queue is full")]
    QueueFull,
    #[error("input queue capacity is invalid")]
    InvalidQueueCapacity,
    #[error("input rate limit is invalid")]
    InvalidRateLimit,
    #[error("encoded control message is too large")]
    PayloadTooLarge,
    #[error("invalid monitor: {0}")]
    InvalidMonitor(#[from] nexus_protocol::MonitorInfoError),
    #[error("invalid cursor snapshot: {0}")]
    InvalidCursor(#[from] platform_windows::BackendError),
    #[error("input could not be encoded: {0}")]
    Encode(#[from] ClientInputError),
}

/// Bounded semantic input queue owned by the client/network boundary.
#[derive(Debug)]
pub struct InputController {
    monitor: MonitorInfo,
    focused: bool,
    shutdown: bool,
    queue: VecDeque<Vec<u8>>,
    queue_capacity: usize,
    max_events_per_second: usize,
    recent_events: VecDeque<Instant>,
}

impl InputController {
    pub fn new(monitor: MonitorInfo) -> Result<Self, InputControllerError> {
        Self::with_limits(
            monitor,
            DEFAULT_INPUT_RATE_PER_SECOND,
            DEFAULT_INPUT_QUEUE_CAPACITY,
        )
    }

    pub fn with_limits(
        monitor: MonitorInfo,
        max_events_per_second: usize,
        queue_capacity: usize,
    ) -> Result<Self, InputControllerError> {
        monitor.validate()?;
        if queue_capacity == 0 || queue_capacity > MAX_INPUT_QUEUE_CAPACITY {
            return Err(InputControllerError::InvalidQueueCapacity);
        }
        if max_events_per_second == 0 || max_events_per_second > MAX_INPUT_RATE_PER_SECOND {
            return Err(InputControllerError::InvalidRateLimit);
        }
        Ok(Self {
            monitor,
            focused: false,
            shutdown: false,
            queue: VecDeque::with_capacity(queue_capacity),
            queue_capacity,
            max_events_per_second,
            recent_events: VecDeque::new(),
        })
    }

    pub fn set_focused(&mut self, focused: bool) {
        if !self.shutdown {
            self.focused = focused;
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused && !self.shutdown
    }

    pub fn expire(&mut self) {
        self.shutdown = true;
        self.focused = false;
        self.queue.clear();
    }

    pub fn shutdown(&mut self) {
        self.expire();
    }

    /// Drops controls produced for a transport that has gone away while
    /// retaining the window focus state for a reconnect.
    pub(crate) fn clear_pending(&mut self) {
        self.queue.clear();
        self.recent_events.clear();
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    /// Translates a native-window event.  Accepting both an owned event and a
    /// borrowed event keeps this boundary convenient for event-loop callers.
    pub fn handle_window_event<E>(&mut self, event: E) -> Result<(), InputControllerError>
    where
        E: std::borrow::Borrow<WindowEvent>,
    {
        if self.shutdown {
            return Err(InputControllerError::Shutdown);
        }
        match event.borrow() {
            WindowEvent::FocusChanged(focused) | WindowEvent::Focused(focused) => {
                self.set_focused(*focused);
                Ok(())
            }
            WindowEvent::Closed => {
                self.expire();
                Ok(())
            }
            WindowEvent::Input(input) => self.enqueue_input(input),
            WindowEvent::Resized { .. }
            | WindowEvent::RenderRequested
            | WindowEvent::RenderRejected => Ok(()),
        }
    }

    fn enqueue_input(
        &mut self,
        input: &nexus_input::InputEvent,
    ) -> Result<(), InputControllerError> {
        if !self.focused {
            return Err(InputControllerError::NotFocused);
        }
        if self.queue.len() >= self.queue_capacity {
            return Err(InputControllerError::QueueFull);
        }

        let now = Instant::now();
        while self
            .recent_events
            .front()
            .is_some_and(|timestamp| now.duration_since(*timestamp) >= Duration::from_secs(1))
        {
            self.recent_events.pop_front();
        }
        if self.recent_events.len() >= self.max_events_per_second {
            return Err(InputControllerError::RateLimited);
        }

        let bounded = clamp_to_monitor(input, &self.monitor);
        let encoded = ClientInputSender::encode(bounded)?;
        if encoded.len() > MAX_INPUT_CONTROL_BYTES {
            return Err(InputControllerError::PayloadTooLarge);
        }
        self.recent_events.push_back(now);
        self.queue.push_back(encoded);
        Ok(())
    }

    pub fn try_next_control(&mut self) -> Option<Vec<u8>> {
        self.queue.pop_front()
    }

    pub fn pending_controls(&self) -> usize {
        self.queue.len()
    }

    pub fn validate_cursor_snapshot(
        &self,
        snapshot: &CursorSnapshot,
    ) -> Result<(), InputControllerError> {
        snapshot
            .validate()
            .map_err(InputControllerError::InvalidCursor)
    }

    pub fn monitor(&self) -> &MonitorInfo {
        &self.monitor
    }
}

fn clamp_to_monitor(
    input: &nexus_input::InputEvent,
    monitor: &MonitorInfo,
) -> nexus_input::InputEvent {
    let max_x = i64::from(monitor.origin_x) + i64::from(monitor.width) - 1;
    let max_y = i64::from(monitor.origin_y) + i64::from(monitor.height) - 1;
    match input {
        nexus_input::InputEvent::MouseMove { x, y } => nexus_input::InputEvent::MouseMove {
            x: i64::from(*x).clamp(i64::from(monitor.origin_x), max_x) as i32,
            y: i64::from(*y).clamp(i64::from(monitor.origin_y), max_y) as i32,
        },
        other => other.clone(),
    }
}
