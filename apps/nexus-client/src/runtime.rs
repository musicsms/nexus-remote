//! Bounded orchestration for the native viewer/controller.

use crate::session::{
    ClientError, ClientSession, ClientState, ClientVerification, RelayTokenMetadata, SessionPolicy,
    DEFAULT_MAX_SESSION_DURATION,
};
use crate::{
    ClientReceiver, ClientReceiverError, InputController, InputControllerError, RenderQueue,
    RenderQueueError, WindowCommand, WindowConfig, WindowController, WindowError, WindowEvent,
};
use ed25519_dalek::VerifyingKey;
use nexus_common::{SystemClock, UnixTimestamp};
use nexus_protocol::{MonitorInfo, SessionCapability};
use prost::Message;
use quinn::Connection;
use rustls::pki_types::CertificateDer;
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use thiserror::Error;

const RUNTIME_POLL_INTERVAL: Duration = Duration::from_millis(2);
const RECONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// Cloneable cancellation signal for a running client runtime. `notify_one`
/// retains a permit, so cancellation cannot be lost between the atomic check
/// and registering a `Notify` waiter.
#[derive(Clone)]
pub struct RuntimeCancellation {
    cancelled: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

pub type ShutdownHandle = RuntimeCancellation;

impl RuntimeCancellation {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for RuntimeCancellation {
    fn default() -> Self {
        Self::new()
    }
}

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

fn required_env(name: &str) -> Result<String, ClientRuntimeError> {
    std::env::var(name).map_err(|_| ClientRuntimeError::InvalidBootstrap(name.to_owned()))
}

fn parse_env<T>(name: &str) -> Result<T, ClientRuntimeError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    required_env(name)?
        .parse()
        .map_err(|error: T::Err| ClientRuntimeError::InvalidBootstrap(format!("{name}: {error}")))
}

fn read_hex_env(name: &str) -> Result<Vec<u8>, ClientRuntimeError> {
    decode_hex(name, &required_env(name)?)
}

fn decode_hex(name: &str, value: &str) -> Result<Vec<u8>, ClientRuntimeError> {
    let bytes = value.as_bytes();
    if !value.is_ascii() {
        return Err(ClientRuntimeError::InvalidBootstrap(format!(
            "{name} must contain ASCII hexadecimal digits"
        )));
    }
    if !bytes.len().is_multiple_of(2) {
        return Err(ClientRuntimeError::InvalidBootstrap(format!(
            "{name} must contain an even number of hexadecimal digits"
        )));
    }
    bytes
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII input is valid UTF-8");
            u8::from_str_radix(pair, 16).map_err(|_| {
                ClientRuntimeError::InvalidBootstrap(format!("{name} is not hexadecimal"))
            })
        })
        .collect()
}

#[cfg(test)]
mod bootstrap_tests {
    use super::{
        decode_hex, ClientConfiguration, ClientRuntime, ClientRuntimeError, RuntimeCancellation,
    };

    #[test]
    fn malformed_unicode_hex_is_rejected_without_panicking() {
        assert!(matches!(
            decode_hex("FRAME_KEY", "é1"),
            Err(ClientRuntimeError::InvalidBootstrap(_))
        ));
    }

    #[tokio::test]
    async fn configured_entrypoint_fails_closed_without_authenticated_bootstrap() {
        let bootstrap_names = [
            "NEXUS_CLIENT_SERVER_CERT_DER",
            "NEXUS_CLIENT_CAPABILITY_HEX",
            "NEXUS_CLIENT_CAPABILITY_VERIFYING_KEY_HEX",
            "NEXUS_CLIENT_RELAY_ID",
            "NEXUS_CLIENT_RELAY_SESSION_ID",
            "NEXUS_CLIENT_RELAY_CLIENT_DEVICE_ID",
            "NEXUS_CLIENT_RELAY_TARGET_DEVICE_ID",
            "NEXUS_CLIENT_RELAY_EXPIRES_AT",
            "NEXUS_CLIENT_RELAY_SIGNATURE_HEX",
            "NEXUS_CLIENT_RELAY_VERIFYING_KEY_HEX",
            "NEXUS_CLIENT_FRAME_KEY_HEX",
            "NEXUS_CLIENT_NONCE_DOMAIN",
            "NEXUS_CLIENT_MONITOR_WIDTH",
            "NEXUS_CLIENT_MONITOR_HEIGHT",
            "NEXUS_CLIENT_STREAM_WIDTH",
            "NEXUS_CLIENT_STREAM_HEIGHT",
        ];
        if bootstrap_names
            .iter()
            .any(|name| std::env::var_os(name).is_some())
        {
            return;
        }
        let result = ClientRuntime::run_configured(ClientConfiguration {
            server: "127.0.0.1:4433".parse().unwrap(),
            server_name: "localhost".to_owned(),
        })
        .await;
        assert!(matches!(
            result,
            Err(ClientRuntimeError::SessionBootstrapRequired)
        ));
    }

    #[tokio::test]
    async fn configured_entrypoint_honors_caller_cancellation() {
        let cancellation = RuntimeCancellation::new();
        cancellation.cancel();
        let result = ClientRuntime::run_configured_with_cancellation(
            ClientConfiguration {
                server: "127.0.0.1:4433".parse().unwrap(),
                server_name: "localhost".to_owned(),
            },
            cancellation.clone(),
        )
        .await;
        assert!(matches!(result, Err(ClientRuntimeError::Shutdown)));
    }
}

fn read_fixed_hex_env<const N: usize>(name: &str) -> Result<[u8; N], ClientRuntimeError> {
    let bytes = read_hex_env(name)?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        ClientRuntimeError::InvalidBootstrap(format!(
            "{name} must decode to exactly {N} bytes, got {}",
            bytes.len()
        ))
    })
}

fn verifying_key_env(name: &str) -> Result<VerifyingKey, ClientRuntimeError> {
    VerifyingKey::from_bytes(&read_fixed_hex_env::<32>(name)?)
        .map_err(|error| ClientRuntimeError::InvalidBootstrap(format!("{name}: {error}")))
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
    #[error("authenticated client bootstrap value is invalid: {0}")]
    InvalidBootstrap(String),
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
use std::sync::{mpsc, Condvar, Mutex};
#[cfg(windows)]
use std::thread::{self, JoinHandle};

#[cfg(windows)]
struct NativePipeline {
    pending: Arc<(Mutex<Option<PendingNativeJob>>, Condvar)>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    errors: mpsc::Receiver<crate::decoder::DecoderError>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(windows)]
struct PendingNativeJob {
    generation: u64,
    job: crate::DecodedFrameJob,
}

#[cfg(windows)]
fn stale_native_job(job_generation: u64, current_generation: u64, worker_generation: u64) -> bool {
    job_generation < current_generation || job_generation < worker_generation
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
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_pending = Arc::clone(&pending);
        let shared_generation = Arc::clone(&generation);
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
                let mut worker_generation = 0_u64;
                loop {
                    let pending_job = {
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
                    let Some(pending_job) = pending_job else {
                        break;
                    };
                    let job_generation = pending_job.generation;
                    let current_generation =
                        shared_generation.load(std::sync::atomic::Ordering::Acquire);
                    if stale_native_job(job_generation, current_generation, worker_generation) {
                        continue;
                    }
                    if job_generation > worker_generation {
                        if let Err(error) = decoder.reset() {
                            let _ = error_tx.try_send(error);
                            break;
                        }
                        worker_generation = job_generation;
                    }
                    match decoder.decode(pending_job.job) {
                        Ok(Some(surface)) => {
                            if job_generation
                                == shared_generation.load(std::sync::atomic::Ordering::Acquire)
                            {
                                if let Err(error) = renderer.present(surface) {
                                    let _ = error_tx.try_send(error);
                                    break;
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(crate::decoder::DecoderError::MissingSequenceHeader) => {
                            // A reconnect starts a new continuity epoch. Drop
                            // deltas until the host supplies a keyframe; this
                            // is expected recovery, not a dead decoder.
                        }
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
                generation,
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
        let generation = self.generation.load(std::sync::atomic::Ordering::Acquire);
        *lock
            .lock()
            .map_err(|_| crate::decoder::DecoderError::BackendLost)? =
            Some(PendingNativeJob { generation, job });
        wake.notify_one();
        Ok(())
    }

    /// Drops all pre-disconnect work and makes the next submission start a
    /// fresh decoder continuity epoch, requiring a keyframe.
    fn reset_for_reconnect(&mut self) -> Result<(), crate::decoder::DecoderError> {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let (lock, wake) = &*self.pending;
        lock.lock()
            .map_err(|_| crate::decoder::DecoderError::BackendLost)?
            .take();
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
            let slot = Arc::new(Mutex::new(Some(worker)));
            let reaper_slot = Arc::clone(&slot);
            if thread::Builder::new()
                .name("nexus-client-pipeline-reaper".to_owned())
                .spawn(move || {
                    if let Some(worker) = reaper_slot.lock().ok().and_then(|mut slot| slot.take()) {
                        let _ = worker.join();
                    }
                })
                .is_err()
            {
                // Keep join ownership if the bounded reaper cannot start.
                // This exceptional fallback may exceed the deadline, but it
                // never silently detaches a native worker.
                if let Some(worker) = slot.lock().ok().and_then(|mut slot| slot.take()) {
                    let _ = worker.join();
                }
            }
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

#[cfg(all(test, windows))]
mod native_pipeline_tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn reconnect_reset_clears_pending_job_and_advances_generation() {
        let pending = Arc::new((Mutex::new(None), Condvar::new()));
        let generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (_, errors) = mpsc::sync_channel(1);
        let mut pipeline = NativePipeline {
            pending: Arc::clone(&pending),
            generation: Arc::clone(&generation),
            stop,
            errors,
            worker: None,
        };
        pipeline
            .submit(crate::DecodedFrameJob {
                frame_id: 1,
                timestamp_us: 1,
                keyframe: true,
                access_unit: vec![1],
            })
            .unwrap();
        assert!(pending.0.lock().unwrap().is_some());
        pipeline.reset_for_reconnect().unwrap();
        assert!(pending.0.lock().unwrap().is_none());
        assert_eq!(generation.load(Ordering::Acquire), 1);
        assert!(stale_native_job(0, 1, 0));
        assert!(!stale_native_job(1, 1, 0));
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
    cancellation: RuntimeCancellation,
    shutdown: bool,
}

impl ClientRuntime {
    /// Enters the production runtime from the validated endpoint configuration.
    ///
    /// The control plane owns the authenticated capability, relay metadata,
    /// peer certificate, and frame key. These are accepted only through the
    /// explicit bootstrap environment contract below; private identity keys
    /// and browser credentials are never loaded by this entrypoint. When the
    /// contract is absent, the boundary fails closed instead of manufacturing
    /// an unauthenticated session.
    pub async fn run_configured(
        configuration: ClientConfiguration,
    ) -> Result<RuntimeSummary, ClientRuntimeError> {
        Self::run_configured_with_cancellation(configuration, RuntimeCancellation::new()).await
    }

    /// Runs the configured client with caller-owned cancellation. The handle
    /// is checked before bootstrap parsing and is shared with the constructed
    /// runtime, so callers can interrupt reconnect/connect waits externally.
    pub async fn run_configured_with_cancellation(
        configuration: ClientConfiguration,
        cancellation: RuntimeCancellation,
    ) -> Result<RuntimeSummary, ClientRuntimeError> {
        if cancellation.is_cancelled() {
            return Err(ClientRuntimeError::Shutdown);
        }
        let bootstrap_names = [
            "NEXUS_CLIENT_SERVER_CERT_DER",
            "NEXUS_CLIENT_CAPABILITY_HEX",
            "NEXUS_CLIENT_CAPABILITY_VERIFYING_KEY_HEX",
            "NEXUS_CLIENT_RELAY_ID",
            "NEXUS_CLIENT_RELAY_SESSION_ID",
            "NEXUS_CLIENT_RELAY_CLIENT_DEVICE_ID",
            "NEXUS_CLIENT_RELAY_TARGET_DEVICE_ID",
            "NEXUS_CLIENT_RELAY_EXPIRES_AT",
            "NEXUS_CLIENT_RELAY_SIGNATURE_HEX",
            "NEXUS_CLIENT_RELAY_VERIFYING_KEY_HEX",
            "NEXUS_CLIENT_FRAME_KEY_HEX",
            "NEXUS_CLIENT_NONCE_DOMAIN",
            "NEXUS_CLIENT_MONITOR_WIDTH",
            "NEXUS_CLIENT_MONITOR_HEIGHT",
            "NEXUS_CLIENT_STREAM_WIDTH",
            "NEXUS_CLIENT_STREAM_HEIGHT",
        ];
        if !bootstrap_names
            .iter()
            .any(|name| std::env::var_os(name).is_some())
        {
            return Err(ClientRuntimeError::SessionBootstrapRequired);
        }

        let server_certificate =
            CertificateDer::from(read_hex_env("NEXUS_CLIENT_SERVER_CERT_DER")?);
        let endpoint = nexus_transport::quic::make_client_endpoint(
            "0.0.0.0:0"
                .parse()
                .expect("literal client bind address is valid"),
            &server_certificate,
        )
        .map_err(|error| ClientRuntimeError::InvalidBootstrap(error.to_string()))?;
        let capability_bytes = read_hex_env("NEXUS_CLIENT_CAPABILITY_HEX")?;
        let capability = SessionCapability::decode(capability_bytes.as_slice())
            .map_err(|error| ClientRuntimeError::InvalidBootstrap(error.to_string()))?;
        let relay_token = RelayTokenMetadata {
            relay_id: required_env("NEXUS_CLIENT_RELAY_ID")?,
            session_id: required_env("NEXUS_CLIENT_RELAY_SESSION_ID")?,
            client_device_id: required_env("NEXUS_CLIENT_RELAY_CLIENT_DEVICE_ID")?,
            target_device_id: required_env("NEXUS_CLIENT_RELAY_TARGET_DEVICE_ID")?,
            expires_at: UnixTimestamp::from_secs(parse_env("NEXUS_CLIENT_RELAY_EXPIRES_AT")?),
            signature: read_hex_env("NEXUS_CLIENT_RELAY_SIGNATURE_HEX")?,
        };
        let verification = ClientVerification {
            capability_key: verifying_key_env("NEXUS_CLIENT_CAPABILITY_VERIFYING_KEY_HEX")?,
            relay_key: verifying_key_env("NEXUS_CLIENT_RELAY_VERIFYING_KEY_HEX")?,
            relay_id: relay_token.relay_id.clone(),
        };
        let session = ClientSession::new(
            capability,
            relay_token,
            SystemClock::new(),
            SessionPolicy::new(DEFAULT_MAX_SESSION_DURATION, Duration::from_secs(60))
                .map_err(|error| ClientRuntimeError::InvalidBootstrap(error.to_string()))?,
            verification,
        );
        let monitor = MonitorInfo {
            id: 0,
            origin_x: 0,
            origin_y: 0,
            width: parse_env("NEXUS_CLIENT_MONITOR_WIDTH")?,
            height: parse_env("NEXUS_CLIENT_MONITOR_HEIGHT")?,
            scale: 1.0,
        };
        let server = configuration.server;
        let server_name = configuration.server_name.clone();
        let connect_config = ClientConnectConfig {
            server,
            server_name: server_name.clone(),
            monitor,
            stream: VideoStreamConfig::new(
                parse_env("NEXUS_CLIENT_STREAM_WIDTH")?,
                parse_env("NEXUS_CLIENT_STREAM_HEIGHT")?,
            )?,
        };
        let frame_key = read_fixed_hex_env::<32>("NEXUS_CLIENT_FRAME_KEY_HEX")?;
        let nonce_domain = parse_env("NEXUS_CLIENT_NONCE_DOMAIN")?;
        let mut runtime = Self::connect_with_cancellation(
            &endpoint,
            connect_config,
            session,
            frame_key,
            nonce_domain,
            cancellation.clone(),
        )
        .await?;
        let summary = loop {
            let summary = runtime.run().await;
            if cancellation.is_cancelled() && matches!(&summary, Err(ClientRuntimeError::Shutdown))
            {
                break summary;
            }
            if runtime.session_state() != ClientState::Reconnecting {
                break summary;
            }
            runtime
                .reconnect_with_retry(&endpoint, server, &server_name, RECONNECT_RETRY_INTERVAL)
                .await?;
        };
        let shutdown_result = runtime.shutdown(Instant::now() + Duration::from_secs(1));
        match (summary, shutdown_result) {
            (Err(error), _) => Err(error),
            (Ok(summary), Ok(())) => Ok(summary),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Validates signed claims before opening a QUIC transport.
    pub async fn connect(
        endpoint: &quinn::Endpoint,
        config: ClientConnectConfig,
        session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
    ) -> Result<Self, ClientRuntimeError> {
        Self::connect_with_cancellation(
            endpoint,
            config,
            session,
            frame_key,
            nonce_domain,
            RuntimeCancellation::new(),
        )
        .await
    }

    pub async fn connect_with_cancellation(
        endpoint: &quinn::Endpoint,
        config: ClientConnectConfig,
        mut session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
        cancellation: RuntimeCancellation,
    ) -> Result<Self, ClientRuntimeError> {
        if cancellation.is_cancelled() {
            let _ = session.expire();
            return Err(ClientRuntimeError::Shutdown);
        }
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
        let connection = match tokio::select! {
            connection = connecting => connection,
            _ = cancellation.notify.notified() => {
                let _ = session.expire();
                return Err(ClientRuntimeError::Shutdown);
            }
        } {
            Ok(connection) => connection,
            Err(error) => {
                let _ = session.expire();
                return Err(ClientRuntimeError::Transport(error));
            }
        };
        if cancellation.is_cancelled() {
            connection.close(0u32.into(), b"client shutdown requested");
            let _ = session.expire();
            return Err(ClientRuntimeError::Shutdown);
        }
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
            cancellation,
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
        self.reconnect_with_retry(endpoint, server, server_name, RECONNECT_RETRY_INTERVAL)
            .await
    }

    /// Retries transport establishment without revoking the authenticated
    /// session. The retry loop ends only when the session reconnect deadline
    /// is reached or the runtime is explicitly shut down.
    pub async fn reconnect_with_retry(
        &mut self,
        endpoint: &quinn::Endpoint,
        server: SocketAddr,
        server_name: &str,
        retry_interval: Duration,
    ) -> Result<(), ClientRuntimeError> {
        if self.shutdown {
            return Err(ClientRuntimeError::Shutdown);
        }
        let retry_interval = retry_interval.max(Duration::from_millis(1));
        loop {
            if self.cancellation.is_cancelled() {
                return Err(ClientRuntimeError::Shutdown);
            }
            match self.reconnect_attempt(endpoint, server, server_name).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if self.session.state() != ClientState::Reconnecting {
                        self.fail_closed();
                        return Err(error);
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(retry_interval) => {}
                        _ = self.cancellation.notify.notified() => return Err(ClientRuntimeError::Shutdown),
                    }
                }
            }
        }
    }

    async fn reconnect_attempt(
        &mut self,
        endpoint: &quinn::Endpoint,
        server: SocketAddr,
        server_name: &str,
    ) -> Result<(), ClientRuntimeError> {
        if self.cancellation.is_cancelled() {
            return Err(ClientRuntimeError::Shutdown);
        }
        let now = self.session.clock().now();
        if let Err(error) = self.session.begin_connect(now) {
            return Err(ClientRuntimeError::Session(error));
        }
        let connecting = match endpoint.connect(server, server_name) {
            Ok(connecting) => connecting,
            Err(error) => {
                self.session
                    .reconnect_attempt_failed(self.session.clock().now())
                    .map_err(ClientRuntimeError::Session)?;
                return Err(ClientRuntimeError::Connect(error));
            }
        };
        let connection = match tokio::select! {
            connection = connecting => connection,
            _ = self.cancellation.notify.notified() => return Err(ClientRuntimeError::Shutdown),
        } {
            Ok(connection) => connection,
            Err(error) => {
                self.session
                    .reconnect_attempt_failed(self.session.clock().now())
                    .map_err(ClientRuntimeError::Session)?;
                return Err(ClientRuntimeError::Transport(error));
            }
        };
        if let Err(error) = self.session.connected(self.session.clock().now()) {
            connection.close(0u32.into(), b"reconnect rejected");
            return Err(ClientRuntimeError::Session(error));
        }
        self.connection.close(0u32.into(), b"transport replaced");
        self.connection = connection;
        self.input.clear_pending();
        self.render_queue.clear();
        self.latest_frame = None;
        #[cfg(windows)]
        if let Err(error) = self.pipeline.reset_for_reconnect() {
            self.fail_closed();
            return Err(ClientRuntimeError::Decoder(error));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        connection: Connection,
        session: ClientSession,
        frame_key: [u8; 32],
        nonce_domain: u32,
        monitor: MonitorInfo,
        stream: VideoStreamConfig,
        window_config: WindowConfig,
        cancellation: RuntimeCancellation,
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
            cancellation,
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

    /// Requests cancellation from another task or thread. The owner should
    /// subsequently call [`Self::shutdown`] to join workers and release all
    /// resources; the notification makes reconnect and transport waits stop
    /// promptly instead of waiting for their retry/QUIC deadlines. For a
    /// configured runtime, prefer retaining the cloneable [`ShutdownHandle`]
    /// returned by [`Self::shutdown_handle`].
    pub fn request_shutdown(&self) {
        self.cancellation.cancel();
        self.connection
            .close(0u32.into(), b"client shutdown requested");
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.cancellation.clone()
    }

    pub async fn run(&mut self) -> Result<RuntimeSummary, ClientRuntimeError> {
        if self.shutdown || self.cancellation.is_cancelled() {
            return Err(ClientRuntimeError::Shutdown);
        }
        loop {
            tokio::select! {
                _ = self.cancellation.notify.notified() => {
                    self.fail_closed();
                    return Err(ClientRuntimeError::Shutdown);
                }
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
                                if self.cancellation.is_cancelled() {
                                    self.fail_closed();
                                    return Err(ClientRuntimeError::Shutdown);
                                }
                                let now = self.session.clock().now();
                                if let Err(error) = self.session.transport_lost(now) {
                                    self.fail_closed();
                                    return Err(ClientRuntimeError::Session(error));
                                }
                                #[cfg(windows)]
                                if let Err(error) = self.pipeline.reset_for_reconnect() {
                                    self.fail_closed();
                                    return Err(ClientRuntimeError::Decoder(error));
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
        self.cancellation.cancel();
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
