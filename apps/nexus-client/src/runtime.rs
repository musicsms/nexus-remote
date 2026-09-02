//! Bounded orchestration for the native viewer/controller.

use crate::session::{ClientError, ClientSession, ClientState};
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
    #[error("client transport could not connect: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("client session rejected the runtime operation: {0}")]
    Session(#[from] ClientError),
    #[cfg(windows)]
    #[error("client decoder or renderer failed: {0}")]
    Decoder(#[from] crate::decoder::DecoderError),
    #[error("client input datagram could not be sent: {0}")]
    SendDatagram(#[from] quinn::SendDatagramError),
    #[error("client runtime has already been shut down")]
    Shutdown,
    #[error("client window shutdown exceeded its deadline")]
    ShutdownTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSummary {
    pub received_frames: u64,
    pub sent_input_messages: u64,
}

#[cfg(windows)]
struct NativePipeline {
    decoder: crate::decoder::NativeFrameDecoder,
    renderer: crate::renderer::NativeFrameRenderer,
}

#[cfg(windows)]
impl NativePipeline {
    fn start(
        window: &WindowController,
        config: &WindowConfig,
    ) -> Result<Self, crate::decoder::DecoderError> {
        let handle = window
            .native_handle()
            .ok_or(crate::decoder::DecoderError::BackendUnavailable)?;
        Ok(Self {
            decoder: crate::decoder::NativeFrameDecoder::start(config.width, config.height)?,
            renderer: crate::renderer::NativeFrameRenderer::start_for_native_handle(handle)?,
        })
    }

    fn consume(&mut self, job: crate::DecodedFrameJob) -> Result<(), crate::decoder::DecoderError> {
        if let Some(surface) = self.decoder.decode(job)? {
            self.renderer.present(surface)?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
#[derive(Default)]
struct NativePipeline;

#[cfg(not(windows))]
impl NativePipeline {
    fn start(_window: &WindowController, _config: &WindowConfig) -> Self {
        Self
    }

    fn consume(&mut self, _job: crate::DecodedFrameJob) {}
}

/// Portable owner of client network, receiver, render handoff, input state,
/// and authenticated session lifecycle. Native handles remain private.
pub struct ClientRuntime {
    connection: Connection,
    session: ClientSession,
    receiver: ClientReceiver,
    window: WindowController,
    render_queue: RenderQueue,
    input: InputController,
    received_frames: u64,
    sent_input_messages: u64,
    latest_frame: Option<crate::DecodedFrameJob>,
    pipeline: NativePipeline,
    shutdown: bool,
}

impl ClientRuntime {
    /// Validates signed claims before opening a QUIC transport.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        server: SocketAddr,
        server_name: &str,
        mut session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
        monitor: MonitorInfo,
    ) -> Result<Self, ClientRuntimeError> {
        let now = session.clock().now();
        session.begin_connect(now)?;
        let connecting = match endpoint.connect(server, server_name) {
            Ok(connecting) => connecting,
            Err(error) => {
                let _ = session.expire();
                return Err(ClientRuntimeError::Connect(error));
            }
        };
        let connection = match connecting.await {
            Ok(connection) => connection,
            Err(error) => {
                let _ = session.expire();
                return Err(ClientRuntimeError::Transport(error));
            }
        };
        if let Err(error) = session.connected(session.clock().now()) {
            connection.close(0u32.into(), b"session rejected");
            let _ = session.expire();
            return Err(ClientRuntimeError::Session(error));
        }
        Self::build(
            connection,
            session,
            frame_key,
            nonce_domain,
            monitor,
            WindowConfig::default(),
        )
    }

    fn build(
        connection: Connection,
        session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
        monitor: MonitorInfo,
        window_config: WindowConfig,
    ) -> Result<Self, ClientRuntimeError> {
        let mut receiver = ClientReceiver::new(frame_key, nonce_domain);
        receiver.set_cursor_monitor(monitor)?;
        let input = InputController::new(monitor)?;
        let window = WindowController::start(window_config.clone())?;
        let render_queue = window.render_queue();
        let pipeline = NativePipeline::start(&window, &window_config);
        #[cfg(windows)]
        let pipeline = pipeline?;
        Ok(Self {
            connection,
            session,
            receiver,
            window,
            render_queue,
            input,
            received_frames: 0,
            sent_input_messages: 0,
            latest_frame: None,
            pipeline,
            shutdown: false,
        })
    }

    pub fn session_id(&self) -> &str {
        self.session.session_id()
    }

    pub fn session_state(&self) -> ClientState {
        self.session.state()
    }

    pub fn handle_window_event(&mut self, event: WindowEvent) -> Result<(), ClientRuntimeError> {
        if self.shutdown {
            return Err(ClientRuntimeError::Shutdown);
        }
        let closed = matches!(&event, WindowEvent::Closed);
        self.input.handle_window_event(event)?;
        if closed {
            self.fail_closed();
            return Err(ClientRuntimeError::Shutdown);
        }
        Ok(())
    }

    pub fn request_close(&self) -> Result<(), ClientRuntimeError> {
        self.window
            .try_send(WindowCommand::Close)
            .map_err(ClientRuntimeError::Window)
    }

    pub async fn run(&mut self) -> Result<RuntimeSummary, ClientRuntimeError> {
        if self.shutdown {
            return Err(ClientRuntimeError::Shutdown);
        }
        loop {
            tokio::select! {
                datagram = self.connection.read_datagram() => {
                    if let Err(error) = self.session.ensure_active(self.session.clock().now()).map_err(ClientRuntimeError::Session) {
                        self.fail_closed();
                        return Err(error);
                    }
                    let bytes = match datagram {
                        Ok(bytes) => bytes,
                        Err(quinn::ConnectionError::ConnectionClosed { .. }
                            | quinn::ConnectionError::ApplicationClosed { .. }
                            | quinn::ConnectionError::Reset
                            | quinn::ConnectionError::LocallyClosed) => {
                                let now = self.session.clock().now();
                                let _ = self.session.transport_lost(now);
                                break;
                            }
                        Err(error) => {
                            self.fail_closed();
                            return Err(ClientRuntimeError::Transport(error));
                        }
                    };
                    if let Err(error) = self.receiver.accept_datagram(&bytes) {
                        self.fail_closed();
                        return Err(ClientRuntimeError::Receiver(error));
                    }
                    if let Some(frame) = self.receiver.drain_latest_frame() {
                        self.latest_frame = Some(frame.clone());
                        if let Err(error) = self.window.render_latest(frame) {
                            self.fail_closed();
                            return Err(ClientRuntimeError::Window(error));
                        }
                        if let Some(render_job) = self.render_queue.take_latest() {
                            #[cfg(windows)]
                            if let Err(error) = self.pipeline.consume(render_job) {
                                self.fail_closed();
                                return Err(ClientRuntimeError::Decoder(error));
                            }
                            #[cfg(not(windows))]
                            self.pipeline.consume(render_job);
                        }
                        self.received_frames = self.received_frames.saturating_add(1);
                    }
                }
                _ = tokio::time::sleep(RUNTIME_POLL_INTERVAL) => {
                    if let Err(error) = self.session.ensure_active(self.session.clock().now()).map_err(ClientRuntimeError::Session) {
                        self.fail_closed();
                        return Err(error);
                    }
                    if let Err(error) = self.pump_window_events() {
                        self.fail_closed();
                        return Err(error);
                    }
                    if let Err(error) = self.pump_input() {
                        self.fail_closed();
                        return Err(error);
                    }
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
            if matches!(event, WindowEvent::Closed) {
                self.input.handle_window_event(event)?;
                self.fail_closed();
                return Err(ClientRuntimeError::Shutdown);
            }
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
        self.latest_frame.take()
    }

    pub fn received_frame_count(&self) -> u64 {
        self.received_frames
    }

    pub fn sent_input_count(&self) -> u64 {
        self.sent_input_messages
    }

    pub fn shutdown(&mut self, deadline: Instant) -> Result<(), ClientRuntimeError> {
        if self.shutdown {
            return Ok(());
        }
        self.fail_closed();
        self.window.shutdown(deadline).map_err(|error| match error {
            WindowError::ShutdownTimeout => ClientRuntimeError::ShutdownTimeout,
            other => ClientRuntimeError::Window(other),
        })
    }

    fn fail_closed(&mut self) {
        let _ = self.session.expire();
        self.input.shutdown();
        self.render_queue.shutdown();
        self.latest_frame = None;
        self.connection.close(0u32.into(), b"client runtime closed");
        self.shutdown = true;
        let _ = self
            .window
            .shutdown(Instant::now() + Duration::from_millis(250));
    }
}

impl Drop for ClientRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown(Instant::now() + Duration::from_millis(250));
    }
}
