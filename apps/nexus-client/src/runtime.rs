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
    #[error("authenticated session bootstrap is not available from non-secret configuration")]
    SessionBootstrapRequired,
    #[error("client window shutdown exceeded its deadline")]
    ShutdownTimeout,
    #[error("authenticated video stream dimensions are invalid")]
    InvalidStreamDimensions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeSummary {
    pub received_frames: u64,
    pub sent_input_messages: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoStreamConfig {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientConnectConfig {
    pub server: SocketAddr,
    pub server_name: String,
    pub monitor: MonitorInfo,
    pub stream: VideoStreamConfig,
}

impl VideoStreamConfig {
    pub fn new(width: u32, height: u32) -> Result<Self, ClientRuntimeError> {
        let config = Self { width, height };
        config.validate()?;
        Ok(config)
    }

    fn validate(self) -> Result<(), ClientRuntimeError> {
        if self.width == 0
            || self.height == 0
            || self.width > 7_680
            || self.height > 4_320
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return Err(ClientRuntimeError::InvalidStreamDimensions);
        }
        Ok(())
    }
}

#[cfg(windows)]
use std::sync::{mpsc, Arc, Condvar, Mutex};
#[cfg(windows)]
use std::thread::{self, JoinHandle};

#[cfg(windows)]
struct NativePipeline {
    pending: Arc<(Mutex<Option<crate::DecodedFrameJob>>, Condvar)>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    errors: mpsc::Receiver<crate::decoder::DecoderError>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(windows)]
impl NativePipeline {
    fn start(
        window: &WindowController,
        stream: VideoStreamConfig,
    ) -> Result<Self, crate::decoder::DecoderError> {
        let handle = window
            .native_handle()
            .ok_or(crate::decoder::DecoderError::BackendUnavailable)?;
        let pending = Arc::new((Mutex::new(None), Condvar::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_pending = Arc::clone(&pending);
        let worker_stop = Arc::clone(&stop);
        let (error_tx, errors) = mpsc::sync_channel(1);
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("nexus-client-pipeline".to_owned())
            .spawn(move || {
                let mut decoder =
                    match crate::decoder::NativeFrameDecoder::start(stream.width, stream.height) {
                        Ok(decoder) => decoder,
                        Err(error) => {
                            let _ = started_tx.send(Err(error));
                            return;
                        }
                    };
                let mut renderer =
                    match crate::renderer::NativeFrameRenderer::start_for_native_handle(handle) {
                        Ok(renderer) => renderer,
                        Err(error) => {
                            let _ = started_tx.send(Err(error));
                            return;
                        }
                    };
                let _ = started_tx.send(Ok(()));
                loop {
                    let job = {
                        let (lock, wake) = &*worker_pending;
                        let mut pending = lock.lock().expect("pipeline slot poisoned");
                        while pending.is_none()
                            && !worker_stop.load(std::sync::atomic::Ordering::Acquire)
                        {
                            pending = wake.wait(pending).expect("pipeline slot poisoned");
                        }
                        if worker_stop.load(std::sync::atomic::Ordering::Acquire) {
                            None
                        } else {
                            pending.take()
                        }
                    };
                    let Some(job) = job else { break };
                    match decoder.decode(job) {
                        Ok(Some(surface)) => {
                            if let Err(error) = renderer.present(surface) {
                                let _ = error_tx.try_send(error);
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let _ = error_tx.try_send(error);
                            break;
                        }
                    }
                }
            })
            .map_err(|_| crate::decoder::DecoderError::BackendUnavailable)?;
        match started_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(Self {
                pending,
                stop,
                errors,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                stop.store(true, std::sync::atomic::Ordering::Release);
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                stop.store(true, std::sync::atomic::Ordering::Release);
                let _ = worker.join();
                Err(crate::decoder::DecoderError::BackendUnavailable)
            }
        }
    }

    fn submit(&mut self, job: crate::DecodedFrameJob) -> Result<(), crate::decoder::DecoderError> {
        if let Some(error) = self.poll_error() {
            return Err(error);
        }
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err(crate::decoder::DecoderError::BackendLost);
        }
        let (lock, wake) = &*self.pending;
        *lock
            .lock()
            .map_err(|_| crate::decoder::DecoderError::BackendLost)? = Some(job);
        wake.notify_one();
        Ok(())
    }

    fn poll_error(&self) -> Option<crate::decoder::DecoderError> {
        self.errors.try_recv().ok()
    }

    fn shutdown(&mut self, deadline: Instant) -> bool {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.pending.1.notify_one();
        let Some(worker) = self.worker.take() else {
            return true;
        };
        if worker.is_finished() {
            let _ = worker.join();
            return true;
        }
        if Instant::now() < deadline {
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
        if worker.is_finished() {
            let _ = worker.join();
            true
        } else {
            let _ = thread::Builder::new()
                .name("nexus-client-pipeline-reaper".to_owned())
                .spawn(move || {
                    let _ = worker.join();
                });
            false
        }
    }
}

#[cfg(windows)]
impl Drop for NativePipeline {
    fn drop(&mut self) {
        let _ = self.shutdown(Instant::now() + Duration::from_millis(250));
    }
}

#[cfg(not(windows))]
#[derive(Default)]
struct NativePipeline;

#[cfg(not(windows))]
impl NativePipeline {
    fn start(_window: &WindowController, _stream: VideoStreamConfig) -> Self {
        Self
    }

    fn submit(&mut self, _job: crate::DecodedFrameJob) {}
    fn shutdown(&mut self, _deadline: Instant) -> bool {
        true
    }
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
    /// Enters the production runtime from the validated endpoint configuration.
    ///
    /// The control plane owns the authenticated capability, relay metadata,
    /// peer certificate, and frame key.  Since those values are deliberately
    /// not read from the process environment, this boundary reports the
    /// missing bootstrap instead of manufacturing an unauthenticated session.
    pub async fn run_configured(
        _configuration: ClientConfiguration,
    ) -> Result<RuntimeSummary, ClientRuntimeError> {
        Err(ClientRuntimeError::SessionBootstrapRequired)
    }

    /// Validates signed claims before opening a QUIC transport.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        config: ClientConnectConfig,
        mut session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
    ) -> Result<Self, ClientRuntimeError> {
        config.stream.validate()?;
        let now = session.clock().now();
        session.begin_connect(now)?;
        let connecting = match endpoint.connect(config.server, &config.server_name) {
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
            config.monitor,
            config.stream,
            WindowConfig::default(),
        )
    }

    /// Re-establishes the transport within the existing reconnect window.
    /// The session object is retained, so its established-duration limit and
    /// stable session ID are never reset by a reconnect.
    pub async fn reconnect(
        &mut self,
        endpoint: &quinn::Endpoint,
        server: SocketAddr,
        server_name: &str,
    ) -> Result<(), ClientRuntimeError> {
        if self.shutdown {
            return Err(ClientRuntimeError::Shutdown);
        }
        let now = self.session.clock().now();
        if let Err(error) = self.session.begin_connect(now) {
            self.fail_closed();
            return Err(ClientRuntimeError::Session(error));
        }
        let connecting = match endpoint.connect(server, server_name) {
            Ok(connecting) => connecting,
            Err(error) => {
                self.fail_closed();
                return Err(ClientRuntimeError::Connect(error));
            }
        };
        let connection = match connecting.await {
            Ok(connection) => connection,
            Err(error) => {
                self.fail_closed();
                return Err(ClientRuntimeError::Transport(error));
            }
        };
        if let Err(error) = self.session.connected(self.session.clock().now()) {
            connection.close(0u32.into(), b"reconnect rejected");
            self.fail_closed();
            return Err(ClientRuntimeError::Session(error));
        }
        self.connection.close(0u32.into(), b"transport replaced");
        self.connection = connection;
        self.input.clear_pending();
        self.render_queue.clear();
        self.latest_frame = None;
        Ok(())
    }

    fn build(
        connection: Connection,
        session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
        monitor: MonitorInfo,
        stream: VideoStreamConfig,
        window_config: WindowConfig,
    ) -> Result<Self, ClientRuntimeError> {
        let mut receiver = ClientReceiver::new(frame_key, nonce_domain);
        receiver.set_cursor_monitor(monitor)?;
        let input = InputController::new(monitor)?;
        let window = WindowController::start(window_config.clone())?;
        let render_queue = window.render_queue();
        let pipeline = NativePipeline::start(&window, stream);
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
                                if let Err(error) = self.session.transport_lost(now) {
                                    self.fail_closed();
                                    return Err(ClientRuntimeError::Session(error));
                                }
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
                            if let Err(error) = self.pipeline.submit(render_job) {
                                self.fail_closed();
                                return Err(ClientRuntimeError::Decoder(error));
                            }
                            #[cfg(not(windows))]
                            self.pipeline.submit(render_job);
                        }
                        self.received_frames = self.received_frames.saturating_add(1);
                    }
                }
                _ = tokio::time::sleep(RUNTIME_POLL_INTERVAL) => {
                    #[cfg(windows)]
                    if let Some(error) = self.pipeline.poll_error() {
                        self.fail_closed();
                        return Err(ClientRuntimeError::Decoder(error));
                    }
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
        if self.fail_closed_with_deadline(deadline) {
            Ok(())
        } else {
            Err(ClientRuntimeError::ShutdownTimeout)
        }
    }

    fn fail_closed(&mut self) {
        self.fail_closed_with_deadline(Instant::now() + Duration::from_millis(250));
    }

    fn fail_closed_with_deadline(&mut self, deadline: Instant) -> bool {
        let _ = self.session.expire();
        self.input.shutdown();
        self.render_queue.shutdown();
        self.latest_frame = None;
        self.connection.close(0u32.into(), b"client runtime closed");
        self.shutdown = true;
        let pipeline_stopped = self.pipeline.shutdown(deadline);
        let window_stopped = self.window.shutdown(deadline).is_ok();
        pipeline_stopped && window_stopped
    }
}

impl Drop for ClientRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown(Instant::now() + Duration::from_millis(250));
    }
}
