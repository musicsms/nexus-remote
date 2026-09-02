//! Bounded orchestration for the native viewer/controller.
//!
//! Tokio owns only QUIC I/O and the short polling timer here.  Video jobs cross
//! into the existing depth-one window/render handoff, while semantic input is
//! encoded by the portable controller before it is sent as a QUIC datagram.

use crate::{
    ClientReceiver, ClientReceiverError, InputController, InputControllerError, RenderQueue,
    RenderQueueError, WindowCommand, WindowConfig, WindowController, WindowError, WindowEvent,
};
use nexus_protocol::MonitorInfo;
use quinn::Connection;
use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};
use thiserror::Error;

const RUNTIME_POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClientConfigurationError {
    #[error("NEXUS_CLIENT_SERVER is required (for example 127.0.0.1:4433)")]
    MissingServer,
    #[error("NEXUS_CLIENT_SERVER is not a valid socket address")]
    InvalidServer,
    #[error("NEXUS_CLIENT_SERVER_NAME must not be empty or longer than 253 bytes")]
    InvalidServerName,
}

/// Non-secret process configuration. Capability, relay token, session key,
/// and certificate material are deliberately supplied by the authenticated
/// session bootstrap rather than loaded from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfiguration {
    pub server: SocketAddr,
    pub server_name: String,
}

impl ClientConfiguration {
    pub fn from_env() -> Result<Self, ClientConfigurationError> {
        let server = std::env::var("NEXUS_CLIENT_SERVER")
            .map_err(|_| ClientConfigurationError::MissingServer)?
            .parse()
            .map_err(|_| ClientConfigurationError::InvalidServer)?;
        let server_name =
            std::env::var("NEXUS_CLIENT_SERVER_NAME").unwrap_or_else(|_| "localhost".to_owned());
        if server_name.is_empty() || server_name.len() > 253 {
            return Err(ClientConfigurationError::InvalidServerName);
        }
        Ok(Self {
            server,
            server_name,
        })
    }
}

#[derive(Debug, Error)]
pub enum ClientRuntimeError {
    #[error("client window could not start: {0}")]
    Window(#[from] WindowError),
    #[error("client input could not be initialized or encoded: {0}")]
    Input(#[from] InputControllerError),
    #[error("authenticated video could not be received: {0}")]
    Receiver(#[from] ClientReceiverError),
    #[error("authenticated frame could not be queued for rendering: {0}")]
    RenderQueue(#[from] RenderQueueError),
    #[error("client transport closed: {0}")]
    Transport(#[from] quinn::ConnectionError),
    #[error("client input datagram could not be sent: {0}")]
    SendDatagram(#[from] quinn::SendDatagramError),
    #[error("client runtime has already been shut down")]
    Shutdown,
    #[error("client window shutdown exceeded its deadline")]
    ShutdownTimeout,
}

/// Result of a bounded runtime loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSummary {
    pub received_frames: u64,
    pub sent_input_messages: u64,
}

/// The portable owner of client network, receiver, render handoff, and input
/// state. Native HWND/D3D/MF handles remain private to their worker modules.
pub struct ClientRuntime {
    connection: Connection,
    receiver: ClientReceiver,
    window: WindowController,
    render_queue: RenderQueue,
    input: InputController,
    received_frames: u64,
    sent_input_messages: u64,
    latest_frame: Option<crate::DecodedFrameJob>,
    shutdown: bool,
}

impl ClientRuntime {
    /// Builds a runtime around an already authenticated QUIC connection.
    /// Session capability/relay-token verification must happen before this
    /// boundary is called by the control-plane connection owner.
    pub fn connect(
        connection: Connection,
        frame_key: [u8; 32],
        nonce_domain: u32,
        monitor: MonitorInfo,
    ) -> Result<Self, ClientRuntimeError> {
        Self::connect_with_window(
            connection,
            frame_key,
            nonce_domain,
            monitor,
            WindowConfig::default(),
        )
    }

    pub fn connect_with_window(
        connection: Connection,
        frame_key: [u8; 32],
        nonce_domain: u32,
        monitor: MonitorInfo,
        window_config: WindowConfig,
    ) -> Result<Self, ClientRuntimeError> {
        let mut receiver = ClientReceiver::new(frame_key, nonce_domain);
        receiver.set_cursor_monitor(monitor)?;
        let input = InputController::new(monitor)?;
        let window = WindowController::start(window_config)?;
        let render_queue = window.render_queue();
        Ok(Self {
            connection,
            receiver,
            window,
            render_queue,
            input,
            received_frames: 0,
            sent_input_messages: 0,
            latest_frame: None,
            shutdown: false,
        })
    }

    pub fn handle_window_event(&mut self, event: WindowEvent) -> Result<(), ClientRuntimeError> {
        if self.shutdown {
            return Err(ClientRuntimeError::Shutdown);
        }
        self.input.handle_window_event(event)?;
        Ok(())
    }

    /// Runs until the peer closes the QUIC connection or the runtime is shut
    /// down. Every queue operation is non-blocking and bounded.
    pub async fn run(&mut self) -> Result<RuntimeSummary, ClientRuntimeError> {
        if self.shutdown {
            return Err(ClientRuntimeError::Shutdown);
        }

        loop {
            tokio::select! {
                datagram = self.connection.read_datagram() => {
                    let bytes = match datagram {
                        Ok(bytes) => bytes,
                        Err(
                            quinn::ConnectionError::ConnectionClosed { .. }
                            | quinn::ConnectionError::ApplicationClosed { .. }
                            | quinn::ConnectionError::Reset
                            | quinn::ConnectionError::LocallyClosed,
                        ) => break,
                        Err(error) => return Err(ClientRuntimeError::Transport(error)),
                    };
                    self.receiver.accept_datagram(&bytes)?;
                    if let Some(frame) = self.receiver.drain_latest_frame() {
                        self.latest_frame = Some(frame.clone());
                        // Keep the window command boundary exercised for native
                        // builds; the shared queue is the depth-one handoff.
                        self.window.try_send(WindowCommand::Render(frame))?;
                        self.received_frames = self.received_frames.saturating_add(1);
                    }
                }
                _ = tokio::time::sleep(RUNTIME_POLL_INTERVAL) => {
                    self.pump_window_events()?;
                    self.pump_input()?;
                }
            }
        }

        Ok(RuntimeSummary {
            received_frames: self.received_frames,
            sent_input_messages: self.sent_input_messages,
        })
    }

    fn pump_window_events(&mut self) -> Result<(), ClientRuntimeError> {
        while let Some(event) = self.window.try_next_event() {
            self.input.handle_window_event(event)?;
        }
        Ok(())
    }

    fn pump_input(&mut self) -> Result<(), ClientRuntimeError> {
        if let Some(control) = self.input.try_next_control() {
            self.connection.send_datagram(control.into())?;
            self.sent_input_messages = self.sent_input_messages.saturating_add(1);
        }
        Ok(())
    }

    pub fn drain_latest_frame(&mut self) -> Option<crate::DecodedFrameJob> {
        self.latest_frame
            .take()
            .or_else(|| self.render_queue.take_latest())
    }

    pub fn received_frame_count(&self) -> u64 {
        self.received_frames
    }

    pub fn sent_input_count(&self) -> u64 {
        self.sent_input_messages
    }

    /// Requests transport closure and bounds native window teardown by the
    /// caller's deadline. Native workers retain their own joinable reapers if
    /// a driver call exceeds that deadline.
    pub fn shutdown(&mut self, deadline: Instant) -> Result<(), ClientRuntimeError> {
        if self.shutdown {
            return Ok(());
        }
        self.shutdown = true;
        self.input.shutdown();
        self.render_queue.shutdown();
        self.connection.close(0u32.into(), b"client shutdown");
        self.window.shutdown(deadline).map_err(|error| match error {
            WindowError::ShutdownTimeout => ClientRuntimeError::ShutdownTimeout,
            other => ClientRuntimeError::Window(other),
        })
    }
}

impl Drop for ClientRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown(Instant::now() + Duration::from_millis(250));
    }
}
