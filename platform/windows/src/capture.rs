use nexus_capture::{CaptureSource, CapturedFrame};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::{self, JoinHandle};

use crate::{BackendError, BackendErrorKind, BackendResult};

/// Windows capture APIs supported by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureApi {
    Wgc,
    Dxgi,
}

/// Startup policy for a Windows desktop capture source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    pub preferred: CaptureApi,
    pub allow_dxgi_fallback: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            preferred: CaptureApi::Wgc,
            allow_dxgi_fallback: true,
        }
    }
}

/// Observable capture lifecycle state. Native objects are never exposed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Starting,
    Running(CaptureApi),
    RecoverableLoss,
    Stopped,
}

/// Internal adapter around an initialized capture session.
///
/// This trait is public only so platform contract tests can provide a
/// deterministic adapter. Native implementations remain private.
#[doc(hidden)]
pub trait CaptureSession {
    fn next_frame(&mut self) -> BackendResult<CapturedFrame>;

    fn stop(&mut self) -> BackendResult<()> {
        Ok(())
    }
}

/// Internal adapter around WGC/DXGI session initialization.
///
/// This trait is public only so platform contract tests can verify selection
/// without requiring an interactive Windows desktop.
#[doc(hidden)]
pub trait CaptureFactory: Send + 'static {
    fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>>;
}

/// A Windows desktop capture source.
pub struct WindowsCaptureSource {
    state: CaptureState,
    command_tx: Option<SyncSender<CaptureCommand>>,
    response_rx: Receiver<CaptureResponse>,
    native_thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for WindowsCaptureSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsCaptureSource")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl WindowsCaptureSource {
    pub fn start(config: CaptureConfig) -> BackendResult<Self> {
        Self::start_with_factory(config, NativeCaptureFactory)
    }

    #[doc(hidden)]
    pub fn start_with_factory<F>(config: CaptureConfig, factory: F) -> BackendResult<Self>
    where
        F: CaptureFactory,
    {
        let (command_tx, command_rx) = sync_channel(1);
        let (response_tx, response_rx) = sync_channel(1);
        let native_thread = thread::Builder::new()
            .name("nexus-windows-capture".to_owned())
            .spawn(move || worker_main(config, factory, command_rx, response_tx))
            .map_err(|_| BackendErrorKind::InitializationFailed)?;

        match response_rx.recv() {
            Ok(CaptureResponse::Started(Ok(api))) => Ok(Self {
                state: CaptureState::Running(api),
                command_tx: Some(command_tx),
                response_rx,
                native_thread: Some(native_thread),
            }),
            Ok(CaptureResponse::Started(Err(error))) => {
                let _ = native_thread.join();
                Err(error)
            }
            Ok(_) | Err(_) => {
                let _ = native_thread.join();
                Err(BackendErrorKind::InitializationFailed.into())
            }
        }
    }

    pub const fn state(&self) -> CaptureState {
        self.state
    }

    pub fn stop(&mut self) -> BackendResult<()> {
        if self.state == CaptureState::Stopped {
            return Ok(());
        }

        let result = self.request_stop();
        self.state = CaptureState::Stopped;
        let join_result = self
            .native_thread
            .take()
            .map(JoinHandle::join)
            .unwrap_or(Ok(()));

        result?;
        join_result.map_err(|_| BackendErrorKind::NativeFailure.into())
    }

    fn request_stop(&mut self) -> BackendResult<()> {
        let sender = self
            .command_tx
            .take()
            .ok_or_else(|| BackendError::new(BackendErrorKind::Stopped))?;
        if sender.send(CaptureCommand::Stop).is_err() {
            return Err(BackendErrorKind::NativeFailure.into());
        }
        match self.response_rx.recv() {
            Ok(CaptureResponse::Stopped(result)) => result,
            Ok(_) | Err(_) => Err(BackendErrorKind::NativeFailure.into()),
        }
    }
}

impl CaptureSource for WindowsCaptureSource {
    type Error = BackendError;

    fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
        let result = match self.state {
            CaptureState::Running(_) => {
                let sender = self
                    .command_tx
                    .as_ref()
                    .ok_or_else(|| BackendError::new(BackendErrorKind::Stopped))?;
                sender
                    .send(CaptureCommand::NextFrame)
                    .map_err(|_| BackendErrorKind::NativeFailure)?;
                match self.response_rx.recv() {
                    Ok(CaptureResponse::Frame(result)) => result,
                    Ok(_) | Err(_) => Err(BackendErrorKind::NativeFailure.into()),
                }
            }
            CaptureState::RecoverableLoss => Err(BackendErrorKind::DeviceLost.into()),
            CaptureState::Starting => Err(BackendErrorKind::InitializationFailed.into()),
            CaptureState::Stopped => Err(BackendErrorKind::Stopped.into()),
        };

        if matches!(
            result.as_ref().map_err(BackendError::kind),
            Err(BackendErrorKind::DeviceLost)
        ) {
            self.state = CaptureState::RecoverableLoss;
        }
        result
    }
}

impl Drop for WindowsCaptureSource {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn select_session<F>(
    config: CaptureConfig,
    factory: &mut F,
) -> BackendResult<(CaptureApi, Box<dyn CaptureSession>)>
where
    F: CaptureFactory,
{
    match factory.start(config.preferred) {
        Ok(session) => Ok((config.preferred, session)),
        Err(error)
            if config.preferred == CaptureApi::Wgc
                && config.allow_dxgi_fallback
                && matches!(
                    error.kind(),
                    BackendErrorKind::UnsupportedApi | BackendErrorKind::DeviceLost
                ) =>
        {
            factory
                .start(CaptureApi::Dxgi)
                .map(|session| (CaptureApi::Dxgi, session))
        }
        Err(error) => Err(error),
    }
}

enum CaptureCommand {
    NextFrame,
    Stop,
}

enum CaptureResponse {
    Started(BackendResult<CaptureApi>),
    Frame(BackendResult<CapturedFrame>),
    Stopped(BackendResult<()>),
}

fn worker_main<F>(
    config: CaptureConfig,
    mut factory: F,
    command_rx: Receiver<CaptureCommand>,
    response_tx: SyncSender<CaptureResponse>,
) where
    F: CaptureFactory,
{
    #[cfg(windows)]
    let _com = match native::ComApartment::initialize() {
        Ok(apartment) => apartment,
        Err(error) => {
            let _ = response_tx.send(CaptureResponse::Started(Err(error)));
            return;
        }
    };

    let (api, mut session) = match select_session(config, &mut factory) {
        Ok(selection) => selection,
        Err(error) => {
            let _ = response_tx.send(CaptureResponse::Started(Err(error)));
            return;
        }
    };
    if response_tx.send(CaptureResponse::Started(Ok(api))).is_err() {
        let _ = session.stop();
        return;
    }

    while let Ok(command) = command_rx.recv() {
        match command {
            CaptureCommand::NextFrame => {
                let result = session.next_frame().and_then(|frame| {
                    frame
                        .validate()
                        .map_err(|_| BackendErrorKind::InvalidFrame)?;
                    Ok(frame)
                });
                if response_tx.send(CaptureResponse::Frame(result)).is_err() {
                    let _ = session.stop();
                    return;
                }
            }
            CaptureCommand::Stop => {
                let result = session.stop();
                let _ = response_tx.send(CaptureResponse::Stopped(result));
                return;
            }
        }
    }

    let _ = session.stop();
}

struct NativeCaptureFactory;

impl CaptureFactory for NativeCaptureFactory {
    fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
        #[cfg(windows)]
        {
            match api {
                CaptureApi::Wgc => native::WgcSession::start()
                    .map(|session| Box::new(session) as Box<dyn CaptureSession>),
                CaptureApi::Dxgi => native::DxgiSession::start()
                    .map(|session| Box::new(session) as Box<dyn CaptureSession>),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = api;
            Err(BackendErrorKind::UnsupportedPlatform.into())
        }
    }
}

#[cfg(any(windows, test))]
fn copy_bgra_rows(
    source: &[u8],
    width: u32,
    height: u32,
    row_pitch: usize,
) -> BackendResult<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(BackendErrorKind::InvalidFrame.into());
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    if row_pitch < row_bytes {
        return Err(BackendErrorKind::InvalidFrame.into());
    }
    let preceding_rows = (height as usize - 1)
        .checked_mul(row_pitch)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    let required_source_len = preceding_rows
        .checked_add(row_bytes)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    if source.len() < required_source_len {
        return Err(BackendErrorKind::InvalidFrame.into());
    }
    let output_len = row_bytes
        .checked_mul(height as usize)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    let mut output = Vec::with_capacity(output_len);
    for row in 0..height as usize {
        let start = row
            .checked_mul(row_pitch)
            .ok_or(BackendErrorKind::InvalidFrame)?;
        output.extend_from_slice(&source[start..start + row_bytes]);
    }
    Ok(output)
}

#[cfg(windows)]
mod native {
    use super::{copy_bgra_rows, BackendError, BackendErrorKind, BackendResult, CaptureSession};
    use bytes::Bytes;
    use nexus_capture::{CapturedFrame, PixelFormat};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use windows::core::{factory, Error as WindowsError, IInspectable, Interface};
    use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
    use windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice as WinRtD3dDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Graphics::SizeInt32;
    use windows::Win32::Foundation::{E_ACCESSDENIED, HMODULE};
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
        D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter, IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
        DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
        DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED, DXGI_OUTDUPL_FRAME_INFO,
    };
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    const FRAME_TIMEOUT: Duration = Duration::from_secs(1);

    pub(super) struct ComApartment;

    impl ComApartment {
        pub(super) fn initialize() -> BackendResult<Self> {
            // SAFETY: This is called once at the start of the dedicated capture
            // thread, before any COM objects are created on that thread.
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(|_| BackendErrorKind::InitializationFailed)?;
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            // SAFETY: `ComApartment` is created and dropped on the same named
            // capture thread, balancing the successful initialization call.
            unsafe { CoUninitialize() };
        }
    }

    struct D3dContext {
        device: ID3D11Device,
        immediate: ID3D11DeviceContext,
    }

    impl D3dContext {
        fn create() -> BackendResult<Self> {
            let mut device = None;
            let mut immediate = None;
            let mut feature_level = D3D_FEATURE_LEVEL::default();
            // SAFETY: All output pointers refer to initialized `Option` storage;
            // the hardware driver owns the returned COM objects.
            unsafe {
                D3D11CreateDevice(
                    None::<&IDXGIAdapter>,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut immediate),
                )
            }
            .map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            Ok(Self {
                device: device.ok_or(BackendErrorKind::InitializationFailed)?,
                immediate: immediate.ok_or(BackendErrorKind::InitializationFailed)?,
            })
        }

        fn copy_texture(
            &self,
            texture: &ID3D11Texture2D,
            frame_id: u64,
            timestamp_us: u64,
        ) -> BackendResult<CapturedFrame> {
            let mut source_desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: `source_desc` is valid writable storage for the descriptor.
            unsafe { texture.GetDesc(&mut source_desc) };
            if source_desc.Width == 0
                || source_desc.Height == 0
                || source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            {
                return Err(BackendErrorKind::InvalidFrame.into());
            }

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
                ..source_desc
            };
            let mut staging = None;
            // SAFETY: `staging_desc` is fully initialized and `staging` is valid
            // output storage. No initial data is supplied for a staging texture.
            unsafe {
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut staging))
            }
            .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;
            let staging = staging.ok_or(BackendErrorKind::NativeFailure)?;

            // SAFETY: Both resources belong to this device and remain alive for
            // the copy. The capture thread is their only caller.
            unsafe { self.immediate.CopyResource(&staging, texture) };
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            // SAFETY: `mapped` is valid output storage and the staging texture was
            // created with CPU read access.
            unsafe {
                self.immediate
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            }
            .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;

            let copy_result = mapped_texture_bytes(&mapped, source_desc.Width, source_desc.Height);
            // SAFETY: This balances the successful `Map` above before either COM
            // resource is released.
            unsafe { self.immediate.Unmap(&staging, 0) };
            let data = copy_result?;

            let frame = CapturedFrame {
                frame_id,
                timestamp_us,
                width: source_desc.Width,
                height: source_desc.Height,
                format: PixelFormat::Bgra8,
                data: Bytes::from(data),
            };
            frame
                .validate()
                .map_err(|_| BackendErrorKind::InvalidFrame)?;
            Ok(frame)
        }
    }

    fn mapped_texture_bytes(
        mapped: &D3D11_MAPPED_SUBRESOURCE,
        width: u32,
        height: u32,
    ) -> BackendResult<Vec<u8>> {
        if mapped.pData.is_null() || height == 0 {
            return Err(BackendErrorKind::InvalidFrame.into());
        }
        let row_bytes = (width as usize)
            .checked_mul(4)
            .ok_or(BackendErrorKind::InvalidFrame)?;
        let row_pitch = mapped.RowPitch as usize;
        if row_pitch < row_bytes {
            return Err(BackendErrorKind::InvalidFrame.into());
        }
        let mapped_len = (height as usize - 1)
            .checked_mul(row_pitch)
            .and_then(|preceding| preceding.checked_add(row_bytes))
            .ok_or(BackendErrorKind::InvalidFrame)?;
        // SAFETY: D3D11 guarantees a successfully mapped texture exposes at least
        // RowPitch bytes for each row. The checked span excludes trailing padding
        // after the final row and is copied before `Unmap`.
        let source = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped_len) };
        copy_bgra_rows(source, width, height, row_pitch)
    }

    pub(super) struct WgcSession {
        d3d: D3dContext,
        winrt_device: WinRtD3dDevice,
        item: GraphicsCaptureItem,
        frame_pool: Direct3D11CaptureFramePool,
        capture_session: GraphicsCaptureSession,
        frame_ready: Receiver<()>,
        closed: Arc<AtomicBool>,
        frame_token: EventRegistrationToken,
        closed_token: EventRegistrationToken,
        _frame_handler: TypedEventHandler<Direct3D11CaptureFramePool, IInspectable>,
        _closed_handler: TypedEventHandler<GraphicsCaptureItem, IInspectable>,
        pool_size: SizeInt32,
        frame_id: u64,
        stopped: bool,
    }

    impl WgcSession {
        pub(super) fn start() -> BackendResult<Self> {
            match GraphicsCaptureSession::IsSupported() {
                Ok(true) => {}
                Ok(false) => return Err(BackendErrorKind::UnsupportedApi.into()),
                Err(error) => {
                    return Err(classify_windows_error(
                        &error,
                        BackendErrorKind::UnsupportedApi,
                    ))
                }
            }

            let d3d = D3dContext::create()?;
            let dxgi_device: IDXGIDevice = d3d.device.cast().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            // SAFETY: The DXGI device is a live D3D11 device created above. The
            // returned WinRT wrapper shares its COM lifetime.
            let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
                .map_err(|error| {
                    classify_windows_error(&error, BackendErrorKind::InitializationFailed)
                })?;
            let winrt_device: WinRtD3dDevice = inspectable.cast().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;

            let interop: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(|error| {
                    classify_windows_error(&error, BackendErrorKind::UnsupportedApi)
                })?;
            // SAFETY: Both calls only identify the primary desktop monitor. The
            // returned handle is checked before passing it to the WinRT factory.
            let monitor =
                unsafe { MonitorFromWindow(GetDesktopWindow(), MONITOR_DEFAULTTOPRIMARY) };
            if monitor.is_invalid() {
                return Err(BackendErrorKind::InitializationFailed.into());
            }
            // SAFETY: `monitor` is a valid system handle and the requested output
            // interface is the documented GraphicsCaptureItem runtime class.
            let item: GraphicsCaptureItem =
                unsafe { interop.CreateForMonitor(monitor) }.map_err(|error| {
                    classify_windows_error(&error, BackendErrorKind::PermissionDenied)
                })?;
            let pool_size = item.Size().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            validate_pool_size(pool_size)?;
            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                pool_size,
            )
            .map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;

            let (frame_tx, frame_ready) = sync_channel(1);
            let arrived_tx = frame_tx.clone();
            let frame_handler = TypedEventHandler::new(move |_, _| {
                let _ = arrived_tx.try_send(());
                Ok(())
            });
            let frame_token = frame_pool.FrameArrived(&frame_handler).map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;

            let closed = Arc::new(AtomicBool::new(false));
            let closed_flag = Arc::clone(&closed);
            let closed_handler = TypedEventHandler::new(move |_, _| {
                closed_flag.store(true, Ordering::Release);
                let _ = frame_tx.try_send(());
                Ok(())
            });
            let closed_token = item.Closed(&closed_handler).map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            let capture_session = frame_pool.CreateCaptureSession(&item).map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::PermissionDenied)
            })?;
            capture_session.StartCapture().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::PermissionDenied)
            })?;

            Ok(Self {
                d3d,
                winrt_device,
                item,
                frame_pool,
                capture_session,
                frame_ready,
                closed,
                frame_token,
                closed_token,
                _frame_handler: frame_handler,
                _closed_handler: closed_handler,
                pool_size,
                frame_id: 0,
                stopped: false,
            })
        }

        fn acquire_frame(&mut self) -> BackendResult<CapturedFrame> {
            match self.frame_ready.recv_timeout(FRAME_TIMEOUT) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) if self.closed.load(Ordering::Acquire) => {
                    return Err(BackendErrorKind::DeviceLost.into())
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(BackendErrorKind::NativeFailure.into())
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(BackendErrorKind::DeviceLost.into())
                }
            }
            if self.closed.load(Ordering::Acquire) {
                return Err(BackendErrorKind::DeviceLost.into());
            }

            let frame = self.frame_pool.TryGetNextFrame().map_err(|error| {
                if self.closed.load(Ordering::Acquire) {
                    BackendError::new(BackendErrorKind::DeviceLost)
                } else {
                    classify_windows_error(&error, BackendErrorKind::NativeFailure)
                }
            })?;
            let content_size = frame
                .ContentSize()
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;
            validate_pool_size(content_size)?;
            let timestamp_us = frame
                .SystemRelativeTime()
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?
                .Duration;
            let timestamp_us =
                u64::try_from(timestamp_us / 10).map_err(|_| BackendErrorKind::InvalidFrame)?;
            let surface = frame
                .Surface()
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;
            let access: IDirect3DDxgiInterfaceAccess = surface
                .cast()
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;
            // SAFETY: `access` is the documented bridge for retrieving the D3D11
            // texture backing a WinRT Direct3D surface.
            let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;
            self.frame_id = self.frame_id.saturating_add(1);
            let result = self.d3d.copy_texture(&texture, self.frame_id, timestamp_us);
            let close_result = frame
                .Close()
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure));
            let captured = result?;
            close_result?;

            if content_size != self.pool_size {
                self.frame_pool
                    .Recreate(
                        &self.winrt_device,
                        DirectXPixelFormat::B8G8R8A8UIntNormalized,
                        2,
                        content_size,
                    )
                    .map_err(|error| {
                        classify_windows_error(&error, BackendErrorKind::DeviceLost)
                    })?;
                self.pool_size = content_size;
            }
            Ok(captured)
        }
    }

    impl CaptureSession for WgcSession {
        fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
            if self.stopped {
                return Err(BackendErrorKind::Stopped.into());
            }
            self.acquire_frame()
        }

        fn stop(&mut self) -> BackendResult<()> {
            if self.stopped {
                return Ok(());
            }
            self.stopped = true;
            let mut result = Ok(());
            record_close_result(&mut result, self.item.RemoveClosed(self.closed_token));
            record_close_result(
                &mut result,
                self.frame_pool.RemoveFrameArrived(self.frame_token),
            );
            record_close_result(&mut result, self.capture_session.Close());
            record_close_result(&mut result, self.frame_pool.Close());
            result
        }
    }

    pub(super) struct DxgiSession {
        d3d: D3dContext,
        duplication: IDXGIOutputDuplication,
        started: Instant,
        frame_id: u64,
        stopped: bool,
    }

    impl DxgiSession {
        pub(super) fn start() -> BackendResult<Self> {
            let d3d = D3dContext::create()?;
            let dxgi_device: IDXGIDevice = d3d.device.cast().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            // SAFETY: Enumeration and duplication are performed on the adapter
            // that owns `d3d.device`; all returned COM objects stay on this thread.
            let adapter = unsafe { dxgi_device.GetAdapter() }.map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            let output = unsafe { adapter.EnumOutputs(0) }.map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::UnsupportedApi)
            })?;
            let output: IDXGIOutput1 = output.cast().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::UnsupportedApi)
            })?;
            let duplication = unsafe { output.DuplicateOutput(&d3d.device) }.map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            Ok(Self {
                d3d,
                duplication,
                started: Instant::now(),
                frame_id: 0,
                stopped: false,
            })
        }

        fn acquire_frame(&mut self) -> BackendResult<CapturedFrame> {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            // SAFETY: Both output pointers are valid, and the duplication object
            // remains alive until `ReleaseFrame` is called below.
            unsafe {
                self.duplication.AcquireNextFrame(
                    FRAME_TIMEOUT.as_millis() as u32,
                    &mut frame_info,
                    &mut resource,
                )
            }
            .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure))?;

            let result = (|| {
                let resource = resource.ok_or(BackendErrorKind::InvalidFrame)?;
                let texture: ID3D11Texture2D = resource.cast().map_err(|error| {
                    classify_windows_error(&error, BackendErrorKind::InvalidFrame)
                })?;
                self.frame_id = self.frame_id.saturating_add(1);
                let timestamp_us = self
                    .started
                    .elapsed()
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX);
                self.d3d.copy_texture(&texture, self.frame_id, timestamp_us)
            })();
            // SAFETY: This exactly balances the successful `AcquireNextFrame`,
            // including when validation or the CPU copy failed.
            let release_result = unsafe { self.duplication.ReleaseFrame() }
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::DeviceLost));
            let captured = result?;
            release_result?;
            Ok(captured)
        }
    }

    impl CaptureSession for DxgiSession {
        fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
            if self.stopped {
                return Err(BackendErrorKind::Stopped.into());
            }
            self.acquire_frame()
        }

        fn stop(&mut self) -> BackendResult<()> {
            self.stopped = true;
            Ok(())
        }
    }

    fn validate_pool_size(size: SizeInt32) -> BackendResult<()> {
        if size.Width <= 0 || size.Height <= 0 {
            return Err(BackendErrorKind::InvalidFrame.into());
        }
        Ok(())
    }

    fn record_close_result(
        overall: &mut BackendResult<()>,
        native_result: windows::core::Result<()>,
    ) {
        if overall.is_ok() {
            if let Err(error) = native_result {
                *overall = Err(classify_windows_error(
                    &error,
                    BackendErrorKind::NativeFailure,
                ));
            }
        }
    }

    fn classify_windows_error(error: &WindowsError, fallback: BackendErrorKind) -> BackendError {
        let code = error.code();
        let kind = if code == E_ACCESSDENIED {
            BackendErrorKind::PermissionDenied
        } else if matches!(
            code,
            DXGI_ERROR_ACCESS_LOST
                | DXGI_ERROR_DEVICE_REMOVED
                | DXGI_ERROR_DEVICE_RESET
                | DXGI_ERROR_SESSION_DISCONNECTED
        ) {
            BackendErrorKind::DeviceLost
        } else if code == DXGI_ERROR_UNSUPPORTED {
            BackendErrorKind::UnsupportedApi
        } else {
            fallback
        };
        kind.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_copy_removes_native_pitch_padding() {
        let source = [
            1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 88, 88, 88, 88,
        ];

        assert_eq!(
            copy_bgra_rows(&source, 2, 2, 12).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn row_copy_rejects_pitch_smaller_than_a_bgra_row() {
        let error = copy_bgra_rows(&[0; 8], 2, 1, 7).unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::InvalidFrame);
    }

    #[test]
    fn row_copy_rejects_a_truncated_mapped_surface() {
        let error = copy_bgra_rows(&[0; 19], 2, 2, 12).unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::InvalidFrame);
    }
}
