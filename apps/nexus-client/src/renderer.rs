//! Bounded, latest-frame render handoff.
//!
//! The receive task never waits for a renderer.  It replaces the one pending
//! frame under a short mutex and lets the native rendering thread take it when
//! it is ready.  No Windows or GPU handle is part of this portable contract.

use crate::DecodedFrameJob;
use nexus_transport::video::MAX_FRAME_PAYLOAD_SIZE;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Maximum plaintext H.264 access unit that may cross into rendering.
pub const MAX_RENDER_ACCESS_UNIT_SIZE: usize = MAX_FRAME_PAYLOAD_SIZE;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderQueueError {
    #[error("decoded access unit is empty")]
    EmptyAccessUnit,
    #[error("decoded access unit is too large: {actual} bytes (limit {limit})")]
    AccessUnitTooLarge { actual: usize, limit: usize },
    #[error("render queue has shut down")]
    Shutdown,
    #[error("render queue state is unavailable")]
    StateUnavailable,
}

#[derive(Debug, Default)]
struct RenderQueueState {
    latest: Option<DecodedFrameJob>,
    dropped_frames: u64,
    shutdown: bool,
}

/// A depth-one, non-blocking producer handoff for authenticated frame jobs.
///
/// Cloning the queue is safe: all producers share one bounded slot and a
/// renderer always consumes the newest submitted frame.
#[derive(Debug, Clone, Default)]
pub struct RenderQueue {
    state: Arc<Mutex<RenderQueueState>>,
}

impl RenderQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the pending frame rather than waiting for rendering work.
    pub fn push_latest(&self, frame: DecodedFrameJob) -> Result<(), RenderQueueError> {
        validate_frame(&frame)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| RenderQueueError::StateUnavailable)?;
        if state.shutdown {
            return Err(RenderQueueError::Shutdown);
        }
        if state.latest.replace(frame).is_some() {
            state.dropped_frames = state.dropped_frames.saturating_add(1);
        }
        Ok(())
    }

    /// Takes the newest frame without blocking a network producer.
    pub fn take_latest(&self) -> Option<DecodedFrameJob> {
        self.state.lock().ok()?.latest.take()
    }

    pub fn dropped_frames(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.dropped_frames)
    }

    /// Prevents future enqueueing and discards the pending plaintext frame.
    pub fn shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.shutdown = true;
            state.latest = None;
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.shutdown)
    }
}

pub(crate) fn validate_frame(frame: &DecodedFrameJob) -> Result<(), RenderQueueError> {
    if frame.access_unit.is_empty() {
        return Err(RenderQueueError::EmptyAccessUnit);
    }
    if frame.access_unit.len() > MAX_RENDER_ACCESS_UNIT_SIZE {
        return Err(RenderQueueError::AccessUnitTooLarge {
            actual: frame.access_unit.len(),
            limit: MAX_RENDER_ACCESS_UNIT_SIZE,
        });
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn native_renderer_smoke() -> Result<(), crate::decoder::DecoderError> {
    let mut renderer = native::D3D11Renderer::start()?;
    renderer.present(crate::decoder::DecodedSurface {
        frame_id: 0,
        timestamp_us: 0,
        keyframe: true,
        width: 2,
        height: 2,
        format: crate::decoder::SurfaceFormat::Rgba8,
        bytes: vec![0; 16],
    })
}

#[cfg(windows)]
pub(crate) mod native {
    //! Native renderer ownership lives exclusively on `nexus-client-renderer`.

    use super::super::decoder::{DecodedSurface, DecoderError, SurfaceFormat};
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
    };

    const COMMAND_TIMEOUT: Duration = Duration::from_millis(250);

    enum RendererCommand {
        Present(DecodedSurface, SyncSender<Result<(), DecoderError>>),
        Shutdown(SyncSender<()>),
    }

    /// Private D3D11 adapter.  The window task supplies the swap-chain target
    /// later; this task establishes the dedicated device-owning thread.
    pub(super) struct D3D11Renderer {
        commands: Option<SyncSender<RendererCommand>>,
        worker: Option<JoinHandle<()>>,
    }

    impl D3D11Renderer {
        pub(super) fn start() -> Result<Self, DecoderError> {
            let (commands, receiver) = sync_channel(1);
            let (started_tx, started_rx) = sync_channel(1);
            let worker = thread::Builder::new()
                .name("nexus-client-renderer".to_owned())
                .spawn(move || renderer_main(receiver, started_tx))
                .map_err(|_| DecoderError::BackendUnavailable)?;
            match started_rx.recv_timeout(COMMAND_TIMEOUT) {
                Ok(Ok(())) => Ok(Self {
                    commands: Some(commands),
                    worker: Some(worker),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    Err(error)
                }
                Err(_) => {
                    // Joining here would make native startup unbounded.  The
                    // worker owns no shared GPU handles before it signals ready.
                    drop(worker);
                    Err(DecoderError::BackendUnavailable)
                }
            }
        }

        pub(super) fn present(&mut self, surface: DecodedSurface) -> Result<(), DecoderError> {
            let (reply_tx, reply_rx) = sync_channel(1);
            self.commands
                .as_ref()
                .ok_or(DecoderError::BackendLost)?
                .try_send(RendererCommand::Present(surface, reply_tx))
                .map_err(|_| DecoderError::BackendLost)?;
            reply_rx
                .recv_timeout(COMMAND_TIMEOUT)
                .map_err(|_| DecoderError::BackendLost)?
        }

        fn stop(&mut self) {
            if let Some(commands) = self.commands.take() {
                let (reply_tx, reply_rx) = sync_channel(1);
                let _ = commands.try_send(RendererCommand::Shutdown(reply_tx));
                let _ = reply_rx.recv_timeout(COMMAND_TIMEOUT);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    impl Drop for D3D11Renderer {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn renderer_main(
        receiver: Receiver<RendererCommand>,
        started: SyncSender<Result<(), DecoderError>>,
    ) {
        let mut feature_level = Default::default();
        let mut device = None;
        let mut context = None;
        // SAFETY: this dedicated thread owns the created D3D11 interfaces and
        // passes valid out-pointers for the documented D3D11CreateDevice call.
        let init = unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut feature_level),
                Some(&mut context),
            )
        }
        .map_err(|_| DecoderError::BackendUnavailable)
        .and_then(|_| match (device, context) {
            (Some(device), Some(context)) => Ok((device, context)),
            _ => Err(DecoderError::BackendUnavailable),
        });
        let Ok((device, context)) = init else {
            let _ = started.send(Err(DecoderError::BackendUnavailable));
            return;
        };
        let _ = started.send(Ok(()));

        while let Ok(command) = receiver.recv() {
            match command {
                RendererCommand::Present(surface, reply) => {
                    // Surface bytes have already been bounded and dimensions
                    // checked by the decoder.  The actual HWND/swap-chain
                    // attachment is deliberately deferred to the window task;
                    // keeping the device here prevents it crossing Tokio.
                    let result = upload_surface(&device, &context, &surface);
                    let _ = reply.send(result);
                }
                RendererCommand::Shutdown(reply) => {
                    // SAFETY: this worker exclusively owns the immediate
                    // context and no resource pointer outlives ClearState.
                    unsafe { context.ClearState() };
                    let _ = reply.send(());
                    break;
                }
            }
        }
    }

    fn upload_surface(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        surface: &DecodedSurface,
    ) -> Result<(), DecoderError> {
        surface.validate()?;
        let (format, row_pitch) = match surface.format {
            SurfaceFormat::Nv12 => (DXGI_FORMAT_NV12, surface.width),
            SurfaceFormat::Rgba8 => (
                DXGI_FORMAT_R8G8B8A8_UNORM,
                surface
                    .width
                    .checked_mul(4)
                    .ok_or(DecoderError::InvalidDimensions)?,
            ),
        };
        let descriptor = D3D11_TEXTURE2D_DESC {
            Width: surface.width,
            Height: surface.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: the descriptor has checked dimensions and `texture` is a
        // valid output slot owned by this renderer worker.
        unsafe { device.CreateTexture2D(&descriptor, None, Some(&mut texture)) }
            .map_err(|_| DecoderError::BackendLost)?;
        let texture = texture.ok_or(DecoderError::BackendLost)?;
        // SAFETY: `surface.bytes` remains alive for the call and validation
        // proved its tight row pitch matches the selected texture format.
        unsafe {
            context.UpdateSubresource(
                &texture,
                0,
                None,
                surface.bytes.as_ptr().cast(),
                row_pitch,
                0,
            )
        };
        Ok(())
    }
}
