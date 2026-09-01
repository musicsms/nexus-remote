use nexus_capture::{CaptureSource, CapturedFrame};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{BackendError, BackendErrorKind, BackendResult};

const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

/// Windows capture APIs supported by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureApi {
    Wgc,
    Dxgi,
}

/// Startup policy for a Windows desktop capture source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    pub allow_dxgi_fallback: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            allow_dxgi_fallback: true,
        }
    }
}

/// Observable capture lifecycle state. Native objects are never exposed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Running(CaptureApi),
    RecoverableLoss,
    Stopped,
}

/// Internal adapter around an initialized capture session.
trait CaptureSession {
    fn next_frame(&mut self) -> BackendResult<CapturedFrame>;

    fn stop(&mut self) -> BackendResult<()> {
        Ok(())
    }
}

/// Internal adapter around WGC/DXGI session initialization.
trait CaptureFactory: Send + 'static {
    fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>>;
}

/// A Windows desktop capture source.
pub struct WindowsCaptureSource {
    state: CaptureState,
    command_tx: Option<SyncSender<CaptureCommand>>,
    response_rx: Receiver<CaptureResponse>,
    native_thread: Option<JoinHandle<()>>,
    control_timeout: Duration,
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
        Self::start_with_factory_and_timeout(config, NativeCaptureFactory, DEFAULT_CONTROL_TIMEOUT)
    }

    #[cfg(test)]
    fn start_with_factory<F>(config: CaptureConfig, factory: F) -> BackendResult<Self>
    where
        F: CaptureFactory,
    {
        Self::start_with_factory_and_timeout(config, factory, DEFAULT_CONTROL_TIMEOUT)
    }

    fn start_with_factory_and_timeout<F>(
        config: CaptureConfig,
        factory: F,
        control_timeout: Duration,
    ) -> BackendResult<Self>
    where
        F: CaptureFactory,
    {
        let (command_tx, command_rx) = sync_channel(1);
        let (response_tx, response_rx) = sync_channel(1);
        let native_thread = thread::Builder::new()
            .name("nexus-windows-capture".to_owned())
            .spawn(move || worker_main(config, factory, command_rx, response_tx))
            .map_err(|_| BackendErrorKind::InitializationFailed)?;

        match response_rx.recv_timeout(control_timeout) {
            Ok(CaptureResponse::Started(Ok(api))) => Ok(Self {
                state: CaptureState::Running(api),
                command_tx: Some(command_tx),
                response_rx,
                native_thread: Some(native_thread),
                control_timeout,
            }),
            Ok(CaptureResponse::Started(Err(error))) => Err(error),
            Err(RecvTimeoutError::Timeout) => Err(BackendErrorKind::Timeout.into()),
            Ok(_) | Err(RecvTimeoutError::Disconnected) => {
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

        let deadline = Instant::now() + self.control_timeout;
        let result = self.request_stop(deadline);
        self.state = CaptureState::Stopped;
        let join_result = self.join_before(deadline);

        result?;
        join_result
    }

    fn request_stop(&mut self, deadline: Instant) -> BackendResult<()> {
        let sender = self
            .command_tx
            .take()
            .ok_or_else(|| BackendError::new(BackendErrorKind::Stopped))?;
        match sender.try_send(CaptureCommand::Stop) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(BackendErrorKind::Timeout.into()),
            Err(TrySendError::Disconnected(_)) => {
                return Err(BackendErrorKind::NativeFailure.into())
            }
        }
        match self.response_rx.recv_timeout(remaining(deadline)?) {
            Ok(CaptureResponse::Stopped(result)) => result,
            Err(RecvTimeoutError::Timeout) => Err(BackendErrorKind::Timeout.into()),
            Ok(_) | Err(RecvTimeoutError::Disconnected) => {
                Err(BackendErrorKind::NativeFailure.into())
            }
        }
    }

    fn join_before(&mut self, deadline: Instant) -> BackendResult<()> {
        let Some(handle) = self.native_thread.take() else {
            return Ok(());
        };
        while !handle.is_finished() {
            let wait = remaining(deadline)?;
            thread::sleep(wait.min(Duration::from_millis(1)));
        }
        handle
            .join()
            .map_err(|_| BackendErrorKind::NativeFailure.into())
    }

    fn transition_to_stopped(&mut self) {
        self.state = CaptureState::Stopped;
        self.command_tx.take();
    }
}

impl CaptureSource for WindowsCaptureSource {
    type Error = BackendError;

    fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
        let result = match self.state {
            CaptureState::Running(_) => {
                let send_result = match self.command_tx.as_ref() {
                    Some(sender) => sender.try_send(CaptureCommand::NextFrame),
                    None => {
                        self.transition_to_stopped();
                        return Err(BackendErrorKind::Stopped.into());
                    }
                };
                if let Err(error) = send_result {
                    self.transition_to_stopped();
                    return match error {
                        TrySendError::Full(_) => Err(BackendErrorKind::Timeout.into()),
                        TrySendError::Disconnected(_) => {
                            Err(BackendErrorKind::NativeFailure.into())
                        }
                    };
                }
                match self.response_rx.recv_timeout(self.control_timeout) {
                    Ok(CaptureResponse::Frame(result)) => result,
                    Err(RecvTimeoutError::Timeout) => {
                        self.transition_to_stopped();
                        Err(BackendErrorKind::Timeout.into())
                    }
                    Ok(_) | Err(RecvTimeoutError::Disconnected) => {
                        self.transition_to_stopped();
                        Err(BackendErrorKind::NativeFailure.into())
                    }
                }
            }
            CaptureState::RecoverableLoss => Err(BackendErrorKind::DeviceLost.into()),
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
        if self.state == CaptureState::Stopped {
            return;
        }
        self.state = CaptureState::Stopped;
        if let Some(sender) = self.command_tx.take() {
            let _ = sender.try_send(CaptureCommand::Stop);
        }
        let _ = self.native_thread.take();
    }
}

fn remaining(deadline: Instant) -> BackendResult<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| BackendErrorKind::Timeout.into())
}

fn select_session<F>(
    config: CaptureConfig,
    factory: &mut F,
) -> BackendResult<(CaptureApi, Box<dyn CaptureSession>)>
where
    F: CaptureFactory,
{
    match factory.start(CaptureApi::Wgc) {
        Ok(session) => Ok((CaptureApi::Wgc, session)),
        Err(error)
            if config.allow_dxgi_fallback
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
    allocation_width: u32,
    allocation_height: u32,
    content_width: u32,
    content_height: u32,
    row_pitch: usize,
) -> BackendResult<Vec<u8>> {
    if allocation_width == 0
        || allocation_height == 0
        || content_width == 0
        || content_height == 0
        || content_width > allocation_width
        || content_height > allocation_height
    {
        return Err(BackendErrorKind::InvalidFrame.into());
    }
    let allocation_row_bytes = (allocation_width as usize)
        .checked_mul(4)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    if row_pitch < allocation_row_bytes {
        return Err(BackendErrorKind::InvalidFrame.into());
    }
    let content_row_bytes = (content_width as usize)
        .checked_mul(4)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    let preceding_rows = (content_height as usize - 1)
        .checked_mul(row_pitch)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    let required_source_len = preceding_rows
        .checked_add(content_row_bytes)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    if source.len() < required_source_len {
        return Err(BackendErrorKind::InvalidFrame.into());
    }
    let output_len = content_row_bytes
        .checked_mul(content_height as usize)
        .ok_or(BackendErrorKind::InvalidFrame)?;
    let mut output = Vec::with_capacity(output_len);
    for row in 0..content_height as usize {
        let start = row
            .checked_mul(row_pitch)
            .ok_or(BackendErrorKind::InvalidFrame)?;
        output.extend_from_slice(&source[start..start + content_row_bytes]);
    }
    Ok(output)
}

#[cfg(any(windows, test))]
fn drain_newest_available_frame<T, Next, Discard>(
    notifications: usize,
    mut next: Next,
    mut discard: Discard,
) -> BackendResult<T>
where
    Next: FnMut() -> BackendResult<Option<T>>,
    Discard: FnMut(T) -> BackendResult<()>,
{
    if notifications == 0 {
        return Err(BackendErrorKind::FrameUnavailable.into());
    }
    let mut newest = None;
    loop {
        let frame = match next() {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => {
                if let Some(frame) = newest.take() {
                    let _ = discard(frame);
                }
                return Err(error);
            }
        };
        if let Some(stale) = newest.replace(frame) {
            if let Err(error) = discard(stale) {
                if let Some(frame) = newest.take() {
                    let _ = discard(frame);
                }
                return Err(error);
            }
        }
    }
    newest.ok_or_else(|| BackendErrorKind::FrameUnavailable.into())
}

#[cfg(any(windows, test))]
fn finish_acquired_frame<T>(
    frame_result: BackendResult<T>,
    release_result: BackendResult<()>,
) -> BackendResult<T> {
    match release_result {
        Err(error) => Err(error),
        Ok(()) => frame_result,
    }
}

#[cfg(any(windows, test))]
fn classify_windows_error_code(code: i32, fallback: BackendErrorKind) -> BackendErrorKind {
    match code as u32 {
        0x8007_0005 => BackendErrorKind::PermissionDenied,
        0x887A_0026 | 0x887A_0005 | 0x887A_0007 | 0x887A_0028 | 0x887A_0006 | 0x887A_0020 => {
            BackendErrorKind::DeviceLost
        }
        0x887A_0027 => BackendErrorKind::FrameUnavailable,
        0x887A_0004 => BackendErrorKind::UnsupportedApi,
        _ => fallback,
    }
}

#[cfg(windows)]
mod native {
    use super::{
        classify_windows_error_code, copy_bgra_rows, drain_newest_available_frame,
        finish_acquired_frame, BackendError, BackendErrorKind, BackendResult, CaptureSession,
    };
    use bytes::Bytes;
    use nexus_capture::{CapturedFrame, PixelFormat};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
        D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter, IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
        DXGI_OUTDUPL_FRAME_INFO,
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
            content_size: Option<(u32, u32)>,
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
            let (content_width, content_height) =
                content_size.unwrap_or((source_desc.Width, source_desc.Height));

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

            let copy_result = mapped_texture_bytes(
                &mapped,
                source_desc.Width,
                source_desc.Height,
                content_width,
                content_height,
            );
            // SAFETY: This balances the successful `Map` above before either COM
            // resource is released.
            unsafe { self.immediate.Unmap(&staging, 0) };
            let data = copy_result?;

            let frame = CapturedFrame {
                frame_id,
                timestamp_us,
                width: content_width,
                height: content_height,
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
        allocation_width: u32,
        allocation_height: u32,
        content_width: u32,
        content_height: u32,
    ) -> BackendResult<Vec<u8>> {
        if mapped.pData.is_null()
            || allocation_width == 0
            || allocation_height == 0
            || content_width == 0
            || content_height == 0
            || content_width > allocation_width
            || content_height > allocation_height
        {
            return Err(BackendErrorKind::InvalidFrame.into());
        }
        let content_row_bytes = (content_width as usize)
            .checked_mul(4)
            .ok_or(BackendErrorKind::InvalidFrame)?;
        let row_pitch = mapped.RowPitch as usize;
        let allocation_row_bytes = (allocation_width as usize)
            .checked_mul(4)
            .ok_or(BackendErrorKind::InvalidFrame)?;
        if row_pitch < allocation_row_bytes {
            return Err(BackendErrorKind::InvalidFrame.into());
        }
        let mapped_len = (content_height as usize - 1)
            .checked_mul(row_pitch)
            .and_then(|preceding| preceding.checked_add(content_row_bytes))
            .ok_or(BackendErrorKind::InvalidFrame)?;
        // SAFETY: D3D11 guarantees a successfully mapped texture exposes at least
        // RowPitch bytes for each row. The checked span excludes trailing padding
        // after the final row and is copied before `Unmap`.
        let source = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped_len) };
        copy_bgra_rows(
            source,
            allocation_width,
            allocation_height,
            content_width,
            content_height,
            row_pitch,
        )
    }

    pub(super) struct WgcSession {
        d3d: D3dContext,
        winrt_device: WinRtD3dDevice,
        item: GraphicsCaptureItem,
        frame_pool: Direct3D11CaptureFramePool,
        capture_session: GraphicsCaptureSession,
        frame_ready: Receiver<()>,
        pending_frames: Arc<AtomicUsize>,
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
                        BackendErrorKind::InitializationFailed,
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
                    classify_windows_error(&error, BackendErrorKind::InitializationFailed)
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
            let pending_frames = Arc::new(AtomicUsize::new(0));
            let arrived_count = Arc::clone(&pending_frames);
            let arrived_tx = frame_tx.clone();
            let frame_handler = TypedEventHandler::new(move |_, _| {
                let _ = arrived_count.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                    Some(count.saturating_add(1))
                });
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
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;
            capture_session.StartCapture().map_err(|error| {
                classify_windows_error(&error, BackendErrorKind::InitializationFailed)
            })?;

            Ok(Self {
                d3d,
                winrt_device,
                item,
                frame_pool,
                capture_session,
                frame_ready,
                pending_frames,
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
            loop {
                match self.frame_ready.recv_timeout(FRAME_TIMEOUT) {
                    Ok(()) => {}
                    Err(RecvTimeoutError::Timeout) if self.closed.load(Ordering::Acquire) => {
                        return Err(BackendErrorKind::DeviceLost.into());
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(BackendErrorKind::FrameUnavailable.into());
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(BackendErrorKind::DeviceLost.into());
                    }
                }
                if self.closed.load(Ordering::Acquire) {
                    return Err(BackendErrorKind::DeviceLost.into());
                }

                let notifications = self.pending_frames.swap(0, Ordering::AcqRel);
                let frame = match drain_newest_available_frame(
                    notifications,
                    || match self.frame_pool.TryGetNextFrame() {
                        Ok(frame) => Ok(Some(frame)),
                        Err(error) if error.code().0 == 0 => Ok(None),
                        Err(_error) if self.closed.load(Ordering::Acquire) => {
                            Err(BackendErrorKind::DeviceLost.into())
                        }
                        Err(error) => Err(classify_windows_error(
                            &error,
                            BackendErrorKind::NativeFailure,
                        )),
                    },
                    |frame| {
                        frame.Close().map_err(|error| {
                            classify_windows_error(&error, BackendErrorKind::NativeFailure)
                        })
                    },
                ) {
                    Ok(frame) => frame,
                    Err(error) if error.kind() == BackendErrorKind::FrameUnavailable => continue,
                    Err(error) => return Err(error),
                };

                let content_size = frame.ContentSize().map_err(|error| {
                    classify_windows_error(&error, BackendErrorKind::NativeFailure)
                });
                let content_size = match content_size.and_then(|size| {
                    validate_pool_size(size)?;
                    Ok(size)
                }) {
                    Ok(size) => size,
                    Err(error) => return close_wgc_frame(&frame, Err(error)),
                };

                if content_size != self.pool_size {
                    close_wgc_frame(&frame, Ok(()))?;
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
                    continue;
                }

                let result = (|| {
                    let timestamp_us = frame
                        .SystemRelativeTime()
                        .map_err(|error| {
                            classify_windows_error(&error, BackendErrorKind::NativeFailure)
                        })?
                        .Duration;
                    let timestamp_us = u64::try_from(timestamp_us / 10)
                        .map_err(|_| BackendErrorKind::InvalidFrame)?;
                    let surface = frame.Surface().map_err(|error| {
                        classify_windows_error(&error, BackendErrorKind::NativeFailure)
                    })?;
                    let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(|error| {
                        classify_windows_error(&error, BackendErrorKind::NativeFailure)
                    })?;
                    // SAFETY: `access` is the documented bridge for retrieving the D3D11
                    // texture backing a WinRT Direct3D surface.
                    let texture: ID3D11Texture2D =
                        unsafe { access.GetInterface() }.map_err(|error| {
                            classify_windows_error(&error, BackendErrorKind::NativeFailure)
                        })?;
                    self.frame_id = self.frame_id.saturating_add(1);
                    let content_width = u32::try_from(content_size.Width)
                        .map_err(|_| BackendErrorKind::InvalidFrame)?;
                    let content_height = u32::try_from(content_size.Height)
                        .map_err(|_| BackendErrorKind::InvalidFrame)?;
                    self.d3d.copy_texture(
                        &texture,
                        self.frame_id,
                        timestamp_us,
                        Some((content_width, content_height)),
                    )
                })();
                return close_wgc_frame(&frame, result);
            }
        }
    }

    fn close_wgc_frame<T>(
        frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
        result: BackendResult<T>,
    ) -> BackendResult<T> {
        let close_result = frame
            .Close()
            .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure));
        finish_acquired_frame(result, close_result)
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
                self.d3d
                    .copy_texture(&texture, self.frame_id, timestamp_us, None)
            })();
            // SAFETY: This exactly balances the successful `AcquireNextFrame`,
            // including when validation or the CPU copy failed.
            let release_result = unsafe { self.duplication.ReleaseFrame() }
                .map_err(|error| classify_windows_error(&error, BackendErrorKind::NativeFailure));
            finish_acquired_frame(result, release_result)
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
        classify_windows_error_code(error.code().0, fallback).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    enum WgcOutcome {
        Error(BackendErrorKind),
    }

    struct ApiRecordingFactory {
        calls: Arc<Mutex<Vec<CaptureApi>>>,
        wgc: WgcOutcome,
    }

    impl CaptureFactory for ApiRecordingFactory {
        fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
            self.calls.lock().unwrap().push(api);
            match (api, self.wgc) {
                (CaptureApi::Wgc, WgcOutcome::Error(kind)) => Err(kind.into()),
                (CaptureApi::Dxgi, _) => Ok(Box::new(NoFrames)),
            }
        }
    }

    struct NoFrames;

    impl CaptureSession for NoFrames {
        fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
            Err(BackendErrorKind::Stopped.into())
        }
    }

    #[test]
    fn selection_attempts_wgc_first_before_dxgi_fallback() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut factory = ApiRecordingFactory {
            calls: Arc::clone(&calls),
            wgc: WgcOutcome::Error(BackendErrorKind::UnsupportedApi),
        };
        let config = CaptureConfig {
            allow_dxgi_fallback: true,
        };

        let (api, _) = select_session(config, &mut factory).unwrap();

        assert_eq!(api, CaptureApi::Dxgi);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[CaptureApi::Wgc, CaptureApi::Dxgi]
        );
    }

    fn recording_factory(
        kind: BackendErrorKind,
    ) -> (ApiRecordingFactory, Arc<Mutex<Vec<CaptureApi>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            ApiRecordingFactory {
                calls: Arc::clone(&calls),
                wgc: WgcOutcome::Error(kind),
            },
            calls,
        )
    }

    fn config(allow_dxgi_fallback: bool) -> CaptureConfig {
        CaptureConfig {
            allow_dxgi_fallback,
        }
    }

    #[test]
    fn selection_falls_back_once_for_wgc_initialization_device_loss() {
        let (mut factory, calls) = recording_factory(BackendErrorKind::DeviceLost);

        let (api, _) = select_session(config(true), &mut factory).unwrap();

        assert_eq!(api, CaptureApi::Dxgi);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[CaptureApi::Wgc, CaptureApi::Dxgi]
        );
    }

    #[test]
    fn selection_does_not_fallback_after_permission_denial() {
        let (mut factory, calls) = recording_factory(BackendErrorKind::PermissionDenied);

        let error = match select_session(config(true), &mut factory) {
            Ok(_) => panic!("permission denial must not select a fallback session"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), BackendErrorKind::PermissionDenied);
        assert_eq!(calls.lock().unwrap().as_slice(), &[CaptureApi::Wgc]);
    }

    #[test]
    fn selection_returns_original_wgc_error_when_fallback_is_disabled() {
        let (mut factory, calls) = recording_factory(BackendErrorKind::UnsupportedApi);

        let error = match select_session(config(false), &mut factory) {
            Ok(_) => panic!("disabled fallback must return the WGC error"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), BackendErrorKind::UnsupportedApi);
        assert_eq!(calls.lock().unwrap().as_slice(), &[CaptureApi::Wgc]);
    }

    struct SessionFactory {
        session: Option<ScriptedSession>,
        thread_names: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl CaptureFactory for SessionFactory {
        fn start(&mut self, api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
            assert_eq!(api, CaptureApi::Wgc);
            self.thread_names
                .lock()
                .unwrap()
                .push(std::thread::current().name().map(str::to_owned));
            Ok(Box::new(self.session.take().unwrap()))
        }
    }

    struct ScriptedSession {
        frames: VecDeque<BackendResult<CapturedFrame>>,
        stop_count: Arc<AtomicUsize>,
        thread_names: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl CaptureSession for ScriptedSession {
        fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
            self.thread_names
                .lock()
                .unwrap()
                .push(std::thread::current().name().map(str::to_owned));
            self.frames.pop_front().unwrap()
        }

        fn stop(&mut self) -> BackendResult<()> {
            self.stop_count.fetch_add(1, Ordering::SeqCst);
            self.thread_names
                .lock()
                .unwrap()
                .push(std::thread::current().name().map(str::to_owned));
            Ok(())
        }
    }

    fn frame(width: u32, height: u32, data: Vec<u8>) -> CapturedFrame {
        CapturedFrame {
            frame_id: 7,
            timestamp_us: 11,
            width,
            height,
            format: nexus_capture::PixelFormat::Bgra8,
            data: data.into(),
        }
    }

    struct SourceFixture {
        source: WindowsCaptureSource,
        stop_count: Arc<AtomicUsize>,
        thread_names: Arc<Mutex<Vec<Option<String>>>>,
    }

    fn source_with_frames(frames: Vec<BackendResult<CapturedFrame>>) -> SourceFixture {
        let stop_count = Arc::new(AtomicUsize::new(0));
        let thread_names = Arc::new(Mutex::new(Vec::new()));
        let factory = SessionFactory {
            session: Some(ScriptedSession {
                frames: frames.into(),
                stop_count: Arc::clone(&stop_count),
                thread_names: Arc::clone(&thread_names),
            }),
            thread_names: Arc::clone(&thread_names),
        };
        let source = WindowsCaptureSource::start_with_factory(config(false), factory).unwrap();
        SourceFixture {
            source,
            stop_count,
            thread_names,
        }
    }

    #[test]
    fn lifecycle_returns_a_validated_frame_from_the_session() {
        let expected = frame(2, 1, vec![0x2a; 8]);
        let mut fixture = source_with_frames(vec![Ok(expected.clone())]);

        assert_eq!(fixture.source.next_frame().unwrap(), expected);
    }

    #[test]
    fn lifecycle_maps_a_malformed_frame_to_invalid_frame() {
        let mut fixture = source_with_frames(vec![Ok(frame(2, 1, vec![0; 7]))]);

        assert_eq!(
            fixture.source.next_frame().unwrap_err().kind(),
            BackendErrorKind::InvalidFrame
        );
    }

    #[test]
    fn lifecycle_classifies_device_loss_as_recoverable() {
        let mut fixture = source_with_frames(vec![Err(BackendErrorKind::DeviceLost.into())]);

        assert_eq!(
            fixture.source.next_frame().unwrap_err().kind(),
            BackendErrorKind::DeviceLost
        );
        assert_eq!(fixture.source.state(), CaptureState::RecoverableLoss);
    }

    #[test]
    fn lifecycle_keeps_running_after_frame_unavailable() {
        let mut fixture = source_with_frames(vec![Err(BackendErrorKind::FrameUnavailable.into())]);

        assert_eq!(
            fixture.source.next_frame().unwrap_err().kind(),
            BackendErrorKind::FrameUnavailable
        );
        assert_eq!(
            fixture.source.state(),
            CaptureState::Running(CaptureApi::Wgc)
        );
    }

    #[test]
    fn lifecycle_returns_stopped_after_shutdown_and_stops_the_session_once() {
        let mut fixture = source_with_frames(Vec::new());

        fixture.source.stop().unwrap();
        fixture.source.stop().unwrap();

        assert_eq!(fixture.source.state(), CaptureState::Stopped);
        assert_eq!(
            fixture.source.next_frame().unwrap_err().kind(),
            BackendErrorKind::Stopped
        );
        assert_eq!(fixture.stop_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn lifecycle_owns_all_session_calls_on_one_named_thread() {
        let mut fixture = source_with_frames(vec![Ok(frame(1, 1, vec![0; 4]))]);

        fixture.source.next_frame().unwrap();
        fixture.source.stop().unwrap();

        assert_eq!(
            fixture.thread_names.lock().unwrap().as_slice(),
            &[
                Some("nexus-windows-capture".to_owned()),
                Some("nexus-windows-capture".to_owned()),
                Some("nexus-windows-capture".to_owned()),
            ]
        );
    }

    struct SlowStartFactory {
        delay: Duration,
    }

    impl CaptureFactory for SlowStartFactory {
        fn start(&mut self, _api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
            std::thread::sleep(self.delay);
            Ok(Box::new(NoFrames))
        }
    }

    #[test]
    fn startup_timeout_returns_without_waiting_for_the_native_thread() {
        let started = Instant::now();

        let error = WindowsCaptureSource::start_with_factory_and_timeout(
            CaptureConfig::default(),
            SlowStartFactory {
                delay: Duration::from_millis(250),
            },
            Duration::from_millis(20),
        )
        .unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_millis(150));
    }

    struct SlowStopFactory {
        delay: Duration,
    }

    impl CaptureFactory for SlowStopFactory {
        fn start(&mut self, _api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
            Ok(Box::new(SlowStopSession { delay: self.delay }))
        }
    }

    struct SlowStopSession {
        delay: Duration,
    }

    impl CaptureSession for SlowStopSession {
        fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
            Err(BackendErrorKind::Stopped.into())
        }

        fn stop(&mut self) -> BackendResult<()> {
            std::thread::sleep(self.delay);
            Ok(())
        }
    }

    #[test]
    fn stop_timeout_detaches_a_stalled_native_thread() {
        let mut source = WindowsCaptureSource::start_with_factory_and_timeout(
            CaptureConfig::default(),
            SlowStopFactory {
                delay: Duration::from_millis(250),
            },
            Duration::from_millis(20),
        )
        .unwrap();
        let started = Instant::now();

        let error = source.stop().unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_millis(150));
        assert_eq!(source.state(), CaptureState::Stopped);
    }

    #[test]
    fn drop_never_waits_for_native_session_cleanup() {
        let source = WindowsCaptureSource::start_with_factory_and_timeout(
            CaptureConfig::default(),
            SlowStopFactory {
                delay: Duration::from_millis(250),
            },
            Duration::from_millis(20),
        )
        .unwrap();
        let started = Instant::now();

        drop(source);

        assert!(started.elapsed() < Duration::from_millis(150));
    }

    struct SlowFrameFactory {
        delay: Duration,
        panic: bool,
    }

    impl CaptureFactory for SlowFrameFactory {
        fn start(&mut self, _api: CaptureApi) -> BackendResult<Box<dyn CaptureSession>> {
            Ok(Box::new(SlowFrameSession {
                delay: self.delay,
                panic: self.panic,
            }))
        }
    }

    struct SlowFrameSession {
        delay: Duration,
        panic: bool,
    }

    impl CaptureSession for SlowFrameSession {
        fn next_frame(&mut self) -> BackendResult<CapturedFrame> {
            if self.panic {
                panic!("simulated native worker failure");
            }
            std::thread::sleep(self.delay);
            Err(BackendErrorKind::NativeFailure.into())
        }
    }

    #[test]
    fn frame_timeout_returns_without_waiting_and_stops_the_source() {
        let mut source = WindowsCaptureSource::start_with_factory_and_timeout(
            CaptureConfig::default(),
            SlowFrameFactory {
                delay: Duration::from_millis(250),
                panic: false,
            },
            Duration::from_millis(20),
        )
        .unwrap();
        let started = Instant::now();

        let error = source.next_frame().unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_millis(150));
        assert_eq!(source.state(), CaptureState::Stopped);
    }

    #[test]
    fn worker_disconnect_moves_the_source_out_of_running() {
        let mut source = WindowsCaptureSource::start_with_factory_and_timeout(
            CaptureConfig::default(),
            SlowFrameFactory {
                delay: Duration::ZERO,
                panic: true,
            },
            Duration::from_millis(100),
        )
        .unwrap();

        let error = source.next_frame().unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::NativeFailure);
        assert_eq!(source.state(), CaptureState::Stopped);
    }

    #[test]
    fn row_copy_removes_native_pitch_padding() {
        let source = [
            1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 88, 88, 88, 88,
        ];

        assert_eq!(
            copy_bgra_rows(&source, 2, 2, 2, 2, 12).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn row_copy_crops_pool_allocation_to_checked_content_size() {
        let source = [1, 2, 3, 4, 90, 90, 90, 90, 5, 6, 7, 8, 80, 80, 80, 80];

        assert_eq!(
            copy_bgra_rows(&source, 2, 2, 1, 2, 8).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn row_copy_rejects_content_larger_than_pool_allocation() {
        let error = copy_bgra_rows(&[0; 16], 2, 2, 3, 2, 8).unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::InvalidFrame);
    }

    #[test]
    fn row_copy_rejects_pitch_smaller_than_a_bgra_row() {
        let error = copy_bgra_rows(&[0; 8], 2, 1, 2, 1, 7).unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::InvalidFrame);
    }

    #[test]
    fn row_copy_rejects_a_truncated_mapped_surface() {
        let error = copy_bgra_rows(&[0; 19], 2, 2, 2, 2, 12).unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::InvalidFrame);
    }

    #[test]
    fn frame_drain_discards_stale_frames_and_returns_only_the_newest() {
        let mut available = VecDeque::from([Some(1_u64), Some(2_u64), None]);
        let mut discarded = Vec::new();

        let newest = drain_newest_available_frame(
            3,
            || Ok(available.pop_front().unwrap()),
            |frame| {
                discarded.push(frame);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(newest, 2);
        assert_eq!(discarded, vec![1]);
    }

    #[test]
    fn frame_drain_checks_the_pool_until_empty_when_notifications_coalesce() {
        let mut available = VecDeque::from([Some(1_u64), Some(2_u64), None]);
        let mut discarded = Vec::new();

        let newest = drain_newest_available_frame(
            1,
            || Ok(available.pop_front().unwrap()),
            |frame| {
                discarded.push(frame);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(newest, 2);
        assert_eq!(discarded, vec![1]);
    }

    #[test]
    fn dxgi_device_hung_and_driver_internal_error_are_recoverable_losses() {
        assert_eq!(
            classify_windows_error_code(0x887A0006_u32 as i32, BackendErrorKind::NativeFailure),
            BackendErrorKind::DeviceLost
        );
        assert_eq!(
            classify_windows_error_code(0x887A0020_u32 as i32, BackendErrorKind::NativeFailure),
            BackendErrorKind::DeviceLost
        );
    }

    #[test]
    fn dxgi_wait_timeout_is_non_fatal_frame_unavailability() {
        assert_eq!(
            classify_windows_error_code(0x887A0027_u32 as i32, BackendErrorKind::NativeFailure),
            BackendErrorKind::FrameUnavailable
        );
    }

    #[test]
    fn unrecognized_windows_error_preserves_the_contextual_fallback() {
        assert_eq!(
            classify_windows_error_code(
                0x8123_4567_u32 as i32,
                BackendErrorKind::InitializationFailed
            ),
            BackendErrorKind::InitializationFailed
        );
    }

    #[test]
    fn release_frame_device_loss_overrides_an_earlier_copy_error() {
        let result: BackendResult<u64> = Err(BackendErrorKind::InvalidFrame.into());
        let release = Err(BackendErrorKind::DeviceLost.into());

        let error = finish_acquired_frame(result, release).unwrap_err();

        assert_eq!(error.kind(), BackendErrorKind::DeviceLost);
    }
}
