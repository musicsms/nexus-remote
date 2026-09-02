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
pub(crate) fn native_renderer_smoke(
    hwnd: isize,
    surface: crate::decoder::DecodedSurface,
) -> Result<(), crate::decoder::DecoderError> {
    let hwnd = windows::Win32::Foundation::HWND(hwnd as *mut core::ffi::c_void);
    let mut renderer = native::D3D11Renderer::start_for_window(hwnd)?;
    renderer.present(surface)
}

#[cfg(windows)]
pub(crate) mod native {
    //! Native renderer ownership lives exclusively on `nexus-client-renderer`.

    use super::super::decoder::{DecodedSurface, DecoderError, SurfaceFormat};
    use crate::native_worker::WorkerLifecycle;
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext,
        ID3D11Texture2D, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC,
        DXGI_RATIONAL, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGISwapChain, DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG,
        DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
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
        lifecycle: Arc<WorkerLifecycle>,
    }

    impl D3D11Renderer {
        pub(super) fn start() -> Result<Self, DecoderError> {
            Self::start_inner(None)
        }

        /// Task 4 supplies the HWND owned by its message-loop thread. This
        /// creates a real D3D11 swap chain and keeps it on the renderer worker.
        pub(super) fn start_for_window(hwnd: HWND) -> Result<Self, DecoderError> {
            Self::start_inner(Some(hwnd))
        }

        fn start_inner(hwnd: Option<HWND>) -> Result<Self, DecoderError> {
            let (commands, receiver) = sync_channel(1);
            let (started_tx, started_rx) = sync_channel(1);
            let worker = thread::Builder::new()
                .name("nexus-client-renderer".to_owned())
                .spawn(move || renderer_main(receiver, started_tx, hwnd))
                .map_err(|_| DecoderError::BackendUnavailable)?;
            let lifecycle = WorkerLifecycle::new(worker);
            match started_rx.recv_timeout(COMMAND_TIMEOUT) {
                Ok(Ok(())) => Ok(Self {
                    commands: Some(commands),
                    lifecycle,
                }),
                Ok(Err(error)) => {
                    lifecycle.reap_in_background();
                    Err(error)
                }
                Err(_) => {
                    lifecycle.reap_in_background();
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
            self.lifecycle.join_before(Instant::now() + COMMAND_TIMEOUT);
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
        hwnd: Option<HWND>,
    ) {
        let mut feature_level = Default::default();
        let mut device = None;
        let mut context = None;
        // SAFETY: this dedicated thread owns the created D3D11 interfaces and
        // passes valid out-pointers for the documented D3D11CreateDevice call.
        let mut swap_chain = None;
        let init = if let Some(hwnd) = hwnd {
            let descriptor = DXGI_SWAP_CHAIN_DESC {
                BufferDesc: DXGI_MODE_DESC {
                    Width: 0,
                    Height: 0,
                    RefreshRate: DXGI_RATIONAL {
                        Numerator: 0,
                        Denominator: 1,
                    },
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    ..Default::default()
                },
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                OutputWindow: hwnd,
                Windowed: BOOL(1),
                SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
                Flags: 0,
            };
            unsafe {
                D3D11CreateDeviceAndSwapChain(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&descriptor),
                    Some(&mut swap_chain),
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut context),
                )
            }
        } else {
            unsafe {
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
        }
        .map_err(|_| DecoderError::BackendUnavailable)
        .and_then(|_| match (device, context) {
            (Some(device), Some(context)) => Ok((device, context, swap_chain)),
            _ => Err(DecoderError::BackendUnavailable),
        });
        let Ok((device, context, swap_chain)) = init else {
            let _ = started.send(Err(DecoderError::BackendUnavailable));
            return;
        };
        let _ = started.send(Ok(()));
        let mut presentation_size = None;

        while let Ok(command) = receiver.recv() {
            match command {
                RendererCommand::Present(surface, reply) => {
                    // Surface bytes have already been bounded and dimensions
                    // checked by the decoder.  The actual HWND/swap-chain
                    // attachment is deliberately deferred to the window task;
                    // keeping the device here prevents it crossing Tokio.
                    let result = upload_surface(
                        &device,
                        &context,
                        swap_chain.as_ref(),
                        &mut presentation_size,
                        &surface,
                    );
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
        swap_chain: Option<&IDXGISwapChain>,
        presentation_size: &mut Option<(u32, u32)>,
        surface: &DecodedSurface,
    ) -> Result<(), DecoderError> {
        surface.validate()?;
        let (bytes, format, row_pitch) = match (surface.format, swap_chain.is_some()) {
            (SurfaceFormat::Nv12, true) => (
                nv12_to_bgra(surface)?,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                surface
                    .width
                    .checked_mul(4)
                    .ok_or(DecoderError::InvalidDimensions)?,
            ),
            (SurfaceFormat::Nv12, false) => {
                (surface.bytes.clone(), DXGI_FORMAT_NV12, surface.width)
            }
            (SurfaceFormat::Rgba8, true) => (
                rgba_to_bgra(surface)?,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                surface
                    .width
                    .checked_mul(4)
                    .ok_or(DecoderError::InvalidDimensions)?,
            ),
            (SurfaceFormat::Rgba8, false) => (
                surface.bytes.clone(),
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
        // SAFETY: `bytes` remains alive for the call and validation proved its
        // tight row pitch matches the selected texture format.
        unsafe {
            context.UpdateSubresource(&texture, 0, None, bytes.as_ptr().cast(), row_pitch, 0)
        };
        if let Some(swap_chain) = swap_chain {
            let requested_size = (surface.width, surface.height);
            if *presentation_size != Some(requested_size) {
                // SAFETY: this worker exclusively owns the swap chain and has
                // released all backbuffer references before resizing it.
                unsafe {
                    swap_chain.ResizeBuffers(
                        0,
                        surface.width,
                        surface.height,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        DXGI_SWAP_CHAIN_FLAG(0),
                    )
                }
                .map_err(|_| DecoderError::BackendLost)?;
                *presentation_size = Some(requested_size);
            }
            let mut back_buffer: ID3D11Texture2D =
                unsafe { swap_chain.GetBuffer(0) }.map_err(|_| DecoderError::BackendLost)?;
            // The window may have been resized independently of the decoded
            // stream since the last frame.  `presentation_size` is only a
            // cache, so verify the actual back-buffer dimensions immediately
            // before CopyResource; that API requires identical extents.
            let mut back_buffer_desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: `back_buffer_desc` is a valid output struct for the
            // texture's synchronous descriptor query.
            unsafe { back_buffer.GetDesc(&mut back_buffer_desc) };
            if (back_buffer_desc.Width, back_buffer_desc.Height) != requested_size {
                drop(back_buffer);
                unsafe {
                    swap_chain.ResizeBuffers(
                        0,
                        surface.width,
                        surface.height,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        DXGI_SWAP_CHAIN_FLAG(0),
                    )
                }
                .map_err(|_| DecoderError::BackendLost)?;
                *presentation_size = Some(requested_size);
                back_buffer =
                    unsafe { swap_chain.GetBuffer(0) }.map_err(|_| DecoderError::BackendLost)?;
            }
            let mut back_buffer_desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: the back buffer remains live while its descriptor is
            // copied into this stack-owned output struct.
            unsafe { back_buffer.GetDesc(&mut back_buffer_desc) };
            if (back_buffer_desc.Width, back_buffer_desc.Height) != requested_size
                || back_buffer_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            {
                return Err(DecoderError::BackendLost);
            }
            // SAFETY: both textures are owned by this D3D11 device and the
            // swap-chain path normalizes decoded surfaces to a BGRA texture
            // with the same dimensions as the backbuffer.
            unsafe { context.CopyResource(&back_buffer, &texture) };
            let present_status = unsafe { swap_chain.Present(1, DXGI_PRESENT(0)) };
            if present_status < 0 {
                return Err(DecoderError::BackendLost);
            }
        }
        Ok(())
    }

    fn nv12_to_bgra(surface: &DecodedSurface) -> Result<Vec<u8>, DecoderError> {
        let width = usize::try_from(surface.width).map_err(|_| DecoderError::InvalidDimensions)?;
        let height =
            usize::try_from(surface.height).map_err(|_| DecoderError::InvalidDimensions)?;
        let y_len = width
            .checked_mul(height)
            .ok_or(DecoderError::InvalidDimensions)?;
        let mut bgra = vec![
            0_u8;
            y_len
                .checked_mul(4)
                .ok_or(DecoderError::InvalidDimensions)?
        ];
        for y in 0..height {
            for x in 0..width {
                let luma = i32::from(surface.bytes[y * width + x])
                    .saturating_sub(16)
                    .max(0);
                let chroma = y_len + (y / 2) * width + (x & !1);
                let u = i32::from(surface.bytes[chroma]) - 128;
                let v = i32::from(surface.bytes[chroma + 1]) - 128;
                let red = (298 * luma + 409 * v + 128) >> 8;
                let green = (298 * luma - 100 * u - 208 * v + 128) >> 8;
                let blue = (298 * luma + 516 * u + 128) >> 8;
                let dst = (y * width + x) * 4;
                bgra[dst] = blue.clamp(0, 255) as u8;
                bgra[dst + 1] = green.clamp(0, 255) as u8;
                bgra[dst + 2] = red.clamp(0, 255) as u8;
                bgra[dst + 3] = 255;
            }
        }
        Ok(bgra)
    }

    fn rgba_to_bgra(surface: &DecodedSurface) -> Result<Vec<u8>, DecoderError> {
        let mut bgra = Vec::with_capacity(surface.bytes.len());
        for rgba in surface.bytes.chunks_exact(4) {
            bgra.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        if bgra.len() != surface.bytes.len() {
            return Err(DecoderError::InvalidSurface);
        }
        Ok(bgra)
    }
}
