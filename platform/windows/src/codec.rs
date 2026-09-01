use nexus_capture::{CapturedFrame, PixelFormat};
use nexus_codec::{CodecError, EncodedFrame, EncoderConfig, VideoEncoder};

/// Portable adapter around the platform-owned encoder transform.
///
/// This trait is public only to support deterministic contract tests. Native
/// Windows objects remain private to this crate's Media Foundation worker.
#[doc(hidden)]
pub trait EncoderTransform: Send {
    fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError>;

    fn encode(
        &mut self,
        frame: CapturedFrame,
        force_keyframe: bool,
    ) -> Result<EncodedFrame, CodecError>;

    fn drain(&mut self) -> Result<(), CodecError>;

    fn shutdown(&mut self);
}

#[cfg(any(windows, test))]
fn bgra_to_nv12(frame: &CapturedFrame) -> Result<Vec<u8>, CodecError> {
    if frame.width == 0
        || frame.height == 0
        || !frame.width.is_multiple_of(2)
        || !frame.height.is_multiple_of(2)
    {
        return Err(CodecError::InvalidDimensions);
    }
    if frame.format != PixelFormat::Bgra8 || frame.validate().is_err() {
        return Err(CodecError::InvalidFrame);
    }

    let width = frame.width as usize;
    let height = frame.height as usize;
    let y_len = width.checked_mul(height).ok_or(CodecError::InvalidFrame)?;
    let output_len = y_len
        .checked_add(y_len / 2)
        .ok_or(CodecError::InvalidFrame)?;
    let mut nv12 = vec![0_u8; output_len];

    for row in 0..height {
        for column in 0..width {
            let source = (row * width + column) * 4;
            let b = i32::from(frame.data[source]);
            let g = i32::from(frame.data[source + 1]);
            let r = i32::from(frame.data[source + 2]);
            nv12[row * width + column] = clamp_byte(((66 * r + 129 * g + 25 * b + 128) >> 8) + 16);
        }
    }

    for row in (0..height).step_by(2) {
        for column in (0..width).step_by(2) {
            let mut u_sum = 0_i32;
            let mut v_sum = 0_i32;
            for row_offset in 0..2 {
                for column_offset in 0..2 {
                    let source = ((row + row_offset) * width + column + column_offset) * 4;
                    let b = i32::from(frame.data[source]);
                    let g = i32::from(frame.data[source + 1]);
                    let r = i32::from(frame.data[source + 2]);
                    u_sum += ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                    v_sum += ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                }
            }
            let destination = y_len + (row / 2) * width + column;
            nv12[destination] = clamp_byte(u_sum / 4);
            nv12[destination + 1] = clamp_byte(v_sum / 4);
        }
    }

    Ok(nv12)
}

#[cfg(any(windows, test))]
fn clamp_byte(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// Windows H.264 encoder state machine around a platform transform.
pub struct WindowsH264Encoder {
    config: Option<EncoderConfig>,
    transform: Box<dyn EncoderTransform>,
    force_next_keyframe: bool,
}

impl std::fmt::Debug for WindowsH264Encoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsH264Encoder")
            .field("config", &self.config)
            .field("force_next_keyframe", &self.force_next_keyframe)
            .finish_non_exhaustive()
    }
}

impl WindowsH264Encoder {
    /// Starts a Media Foundation H.264 encoder on a dedicated native thread.
    pub fn new() -> Result<Self, CodecError> {
        #[cfg(windows)]
        {
            Ok(Self::with_transform(
                native::MediaFoundationTransform::start()?,
            ))
        }
        #[cfg(not(windows))]
        {
            Err(CodecError::BackendUnavailable)
        }
    }

    /// Creates an encoder around a portable transform adapter.
    ///
    /// This constructor is public only to support deterministic contract tests.
    #[doc(hidden)]
    pub fn with_transform<T>(transform: T) -> Self
    where
        T: EncoderTransform + 'static,
    {
        Self {
            config: None,
            transform: Box::new(transform),
            force_next_keyframe: false,
        }
    }
}

#[cfg(windows)]
mod native {
    use super::{bgra_to_nv12, EncoderTransform};
    use bytes::Bytes;
    use nexus_capture::CapturedFrame;
    use nexus_codec::{CodecError, EncodedFrame, EncoderConfig};
    use std::mem::ManuallyDrop;
    use std::ptr;
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::thread::{self, JoinHandle};
    use windows::core::{Error as WindowsError, Interface, VARIANT};
    use windows::Win32::Foundation::{BOOL, E_FAIL, HMODULE};
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoContext,
        ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorInputView,
        ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
        D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
        D3D11_SDK_VERSION, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT,
        D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
        D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
        D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
        D3D11_VPOV_DIMENSION_TEXTURE2D,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter, DXGI_ERROR_DEVICE_HUNG, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    };
    use windows::Win32::Media::MediaFoundation::{
        CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncVideoForceKeyFrame, ICodecAPI, IMFActivate,
        IMFMediaBuffer, IMFMediaEventGenerator, IMFSample, IMFShutdown, IMFTransform,
        METransformDrainComplete, METransformHaveOutput, METransformNeedInput, MFCreateMediaType,
        MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSampleExtension_CleanPoint,
        MFShutdown, MFStartup, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
        MFVideoInterlace_Progressive, MFSTARTUP_FULL, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG,
        MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_MESSAGE_COMMAND_DRAIN,
        MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
        MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
        MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
        MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_EVENT_FLAG_NONE,
        MF_E_HW_MFT_FAILED_START_STREAMING, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_LOW_LATENCY,
        MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
        MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC,
        MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
    };

    enum WorkerCommand {
        Configure(EncoderConfig, SyncSender<Result<(), CodecError>>),
        Encode(
            CapturedFrame,
            bool,
            SyncSender<Result<EncodedFrame, CodecError>>,
        ),
        Drain(SyncSender<Result<(), CodecError>>),
        Shutdown(SyncSender<()>),
    }

    pub(super) struct MediaFoundationTransform {
        command_tx: Option<SyncSender<WorkerCommand>>,
        worker: Option<JoinHandle<()>>,
    }

    impl MediaFoundationTransform {
        pub(super) fn start() -> Result<Self, CodecError> {
            let (command_tx, command_rx) = sync_channel(1);
            let (startup_tx, startup_rx) = sync_channel(1);
            let worker = thread::Builder::new()
                .name("nexus-windows-encoder".to_owned())
                .spawn(move || worker_main(command_rx, startup_tx))
                .map_err(|_| CodecError::BackendUnavailable)?;

            match startup_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    command_tx: Some(command_tx),
                    worker: Some(worker),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    Err(error)
                }
                Err(_) => {
                    let _ = worker.join();
                    Err(CodecError::BackendUnavailable)
                }
            }
        }

        fn send(&self, command: WorkerCommand) -> Result<(), CodecError> {
            self.command_tx
                .as_ref()
                .ok_or(CodecError::BackendLost)?
                .send(command)
                .map_err(|_| CodecError::BackendLost)
        }

        fn stop_worker(&mut self) {
            let Some(sender) = self.command_tx.take() else {
                return;
            };
            let (response_tx, response_rx) = sync_channel(1);
            let _ = sender.send(WorkerCommand::Shutdown(response_tx));
            let _ = response_rx.recv();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    impl EncoderTransform for MediaFoundationTransform {
        fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
            let (response_tx, response_rx) = sync_channel(1);
            self.send(WorkerCommand::Configure(config, response_tx))?;
            response_rx.recv().unwrap_or(Err(CodecError::BackendLost))
        }

        fn encode(
            &mut self,
            frame: CapturedFrame,
            force_keyframe: bool,
        ) -> Result<EncodedFrame, CodecError> {
            let (response_tx, response_rx) = sync_channel(1);
            self.send(WorkerCommand::Encode(frame, force_keyframe, response_tx))?;
            response_rx.recv().unwrap_or(Err(CodecError::BackendLost))
        }

        fn drain(&mut self) -> Result<(), CodecError> {
            let (response_tx, response_rx) = sync_channel(1);
            self.send(WorkerCommand::Drain(response_tx))?;
            response_rx.recv().unwrap_or(Err(CodecError::BackendLost))
        }

        fn shutdown(&mut self) {
            self.stop_worker();
        }
    }

    impl Drop for MediaFoundationTransform {
        fn drop(&mut self) {
            self.stop_worker();
        }
    }

    fn worker_main(
        command_rx: Receiver<WorkerCommand>,
        startup_tx: SyncSender<Result<(), CodecError>>,
    ) {
        let apartment = match ComApartment::initialize() {
            Ok(apartment) => apartment,
            Err(error) => {
                let _ = startup_tx.send(Err(error));
                return;
            }
        };
        let mut encoder = match NativeEncoder::start() {
            Ok(encoder) => encoder,
            Err(error) => {
                let _ = startup_tx.send(Err(error));
                drop(apartment);
                return;
            }
        };
        if startup_tx.send(Ok(())).is_err() {
            encoder.shutdown();
            drop(apartment);
            return;
        }

        while let Ok(command) = command_rx.recv() {
            match command {
                WorkerCommand::Configure(config, response) => {
                    let _ = response.send(encoder.configure(config));
                }
                WorkerCommand::Encode(frame, force_keyframe, response) => {
                    let _ = response.send(encoder.encode(frame, force_keyframe));
                }
                WorkerCommand::Drain(response) => {
                    let _ = response.send(encoder.drain());
                }
                WorkerCommand::Shutdown(response) => {
                    encoder.shutdown();
                    let _ = response.send(());
                    break;
                }
            }
        }
        encoder.shutdown();
        drop(apartment);
    }

    struct ComApartment;

    impl ComApartment {
        fn initialize() -> Result<Self, CodecError> {
            // SAFETY: The dedicated worker has not created COM objects yet, and
            // this apartment guard never leaves that thread.
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(|_| CodecError::BackendUnavailable)?;
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            // SAFETY: Balances this worker's successful CoInitializeEx call.
            unsafe { CoUninitialize() };
        }
    }

    struct NativeEncoder {
        transform: Option<IMFTransform>,
        event_generator: Option<IMFMediaEventGenerator>,
        is_async: bool,
        async_input_ready: bool,
        config: Option<EncoderConfig>,
        converter: Option<Nv12Converter>,
        media_foundation_started: bool,
    }

    impl NativeEncoder {
        fn start() -> Result<Self, CodecError> {
            // SAFETY: Startup and shutdown are paired on this dedicated worker.
            unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
                .map_err(|_| CodecError::BackendUnavailable)?;

            match select_hardware_h264_transform() {
                Ok(transform) => {
                    let event_generator = transform.cast::<IMFMediaEventGenerator>().ok();
                    Ok(Self {
                        transform: Some(transform),
                        event_generator,
                        is_async: false,
                        async_input_ready: false,
                        config: None,
                        converter: None,
                        media_foundation_started: true,
                    })
                }
                Err(error) => {
                    // SAFETY: Balances MFStartup after transform selection failed.
                    let _ = unsafe { MFShutdown() };
                    Err(error)
                }
            }
        }

        fn transform(&self) -> Result<&IMFTransform, CodecError> {
            self.transform.as_ref().ok_or(CodecError::BackendLost)
        }

        fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
            if !config.width.is_multiple_of(2) || !config.height.is_multiple_of(2) {
                return Err(CodecError::InvalidDimensions);
            }
            let transform = self.transform()?.clone();
            let attributes = unsafe { transform.GetAttributes() }.map_err(map_windows_error)?;
            let is_async = unsafe { attributes.GetUINT32(&MF_TRANSFORM_ASYNC) }.unwrap_or(0) != 0;
            if is_async {
                if self.event_generator.is_none() {
                    return Err(CodecError::BackendUnavailable);
                }
                // SAFETY: Async hardware MFTs require this documented opt-in
                // before processing messages or samples.
                unsafe { attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1) }
                    .map_err(map_windows_error)?;
            }
            // Best-effort low-latency hint. Unsupported attributes do not make a
            // standards-compliant hardware encoder unusable.
            let _ = unsafe { attributes.SetUINT32(&MF_LOW_LATENCY, 1) };

            let output_type = create_video_type(&config, MFVideoFormat_H264, true)?;
            let input_type = create_video_type(&config, MFVideoFormat_NV12, false)?;
            // SAFETY: Media types contain complete dimensions, cadence, and
            // subtypes and remain alive for both calls.
            unsafe { transform.SetOutputType(0, &output_type, 0) }.map_err(map_windows_error)?;
            unsafe { transform.SetInputType(0, &input_type, 0) }.map_err(map_windows_error)?;

            if let Ok(codec_api) = transform.cast::<ICodecAPI>() {
                let bitrate = VARIANT::from(config.bitrate_bps);
                // SAFETY: The variant type and CODECAPI key are documented for
                // mean bitrate; rejection is non-fatal because the output media
                // type already carries the required bitrate.
                let _ = unsafe { codec_api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &bitrate) };
            }
            // SAFETY: Type negotiation completed before streaming begins.
            unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }
                .map_err(map_windows_error)?;
            unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }
                .map_err(map_windows_error)?;
            self.converter = Some(Nv12Converter::new(&config));
            self.config = Some(config);
            self.is_async = is_async;
            self.async_input_ready = !is_async;
            Ok(())
        }

        fn encode(
            &mut self,
            frame: CapturedFrame,
            force_keyframe: bool,
        ) -> Result<EncodedFrame, CodecError> {
            let config = self.config.ok_or(CodecError::NotConfigured)?;
            let transform = self.transform()?.clone();
            if self.is_async && !self.async_input_ready {
                self.wait_for_event(METransformNeedInput.0 as u32)?;
            }
            self.async_input_ready = false;

            if force_keyframe {
                force_keyframe_on_transform(&transform)?;
            }
            let nv12 = self
                .converter
                .as_mut()
                .ok_or(CodecError::NotConfigured)?
                .convert(&frame)?;
            let input = create_input_sample(&nv12, &frame, &config)?;
            // SAFETY: The sample owns a complete NV12 buffer and timestamps are
            // expressed in Media Foundation's 100-nanosecond units.
            unsafe { transform.ProcessInput(0, &input, 0) }.map_err(map_windows_error)?;

            if self.is_async {
                self.wait_for_event(METransformHaveOutput.0 as u32)?;
            }
            let (data, keyframe) = self
                .take_output(&config)?
                .ok_or(CodecError::BackendUnavailable)?;
            Ok(EncodedFrame {
                frame_id: frame.frame_id,
                timestamp_us: frame.timestamp_us,
                keyframe,
                data: Bytes::from(data),
            })
        }

        fn wait_for_event(&mut self, expected: u32) -> Result<(), CodecError> {
            let generator = self
                .event_generator
                .as_ref()
                .ok_or(CodecError::BackendUnavailable)?
                .clone();
            loop {
                // SAFETY: Blocking event retrieval occurs only on the dedicated
                // encoder worker, never on an async runtime thread.
                let event =
                    unsafe { generator.GetEvent(MF_EVENT_FLAG_NONE) }.map_err(map_windows_error)?;
                let status = unsafe { event.GetStatus() }.map_err(map_windows_error)?;
                status.ok().map_err(|_| classify_hresult(status.0))?;
                let event_type = unsafe { event.GetType() }.map_err(map_windows_error)?;
                if event_type == METransformNeedInput.0 as u32 {
                    self.async_input_ready = true;
                }
                if event_type == expected {
                    return Ok(());
                }
                if event_type == METransformHaveOutput.0 as u32 {
                    let config = self.config.ok_or(CodecError::NotConfigured)?;
                    let _ = self.take_output(&config)?;
                }
            }
        }

        fn take_output(
            &self,
            config: &EncoderConfig,
        ) -> Result<Option<(Vec<u8>, bool)>, CodecError> {
            let transform = self.transform()?;
            let stream_info =
                unsafe { transform.GetOutputStreamInfo(0) }.map_err(map_windows_error)?;
            let transform_provides_sample = stream_info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                    | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
                != 0;
            let sample = if transform_provides_sample {
                None
            } else {
                let raw_size = config
                    .width
                    .checked_mul(config.height)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or(CodecError::InvalidDimensions)?;
                let capacity = stream_info.cbSize.max(raw_size);
                let sample = unsafe { MFCreateSample() }.map_err(map_windows_error)?;
                let buffer =
                    unsafe { MFCreateMemoryBuffer(capacity) }.map_err(map_windows_error)?;
                unsafe { sample.AddBuffer(&buffer) }.map_err(map_windows_error)?;
                Some(sample)
            };
            let mut output = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0_u32;
            let result = unsafe {
                transform.ProcessOutput(0, std::slice::from_mut(&mut output), &mut status)
            };
            // SAFETY: Both fields were initialized above and are taken exactly
            // once after ProcessOutput returns.
            let sample = unsafe { ManuallyDrop::take(&mut output.pSample) };
            let events = unsafe { ManuallyDrop::take(&mut output.pEvents) };
            drop(events);
            match result {
                Ok(()) => {
                    let sample = sample.ok_or(CodecError::BackendUnavailable)?;
                    let keyframe = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }
                        .unwrap_or(0)
                        != 0;
                    let buffer =
                        unsafe { sample.ConvertToContiguousBuffer() }.map_err(map_windows_error)?;
                    Ok(Some((copy_buffer(&buffer)?, keyframe)))
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
                Err(error) => Err(map_windows_error(error)),
            }
        }

        fn drain(&mut self) -> Result<(), CodecError> {
            if self.config.is_none() {
                return Ok(());
            }
            let transform = self.transform()?.clone();
            // SAFETY: These messages close the configured input stream before
            // draining delayed output and flushing transform state.
            unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0) }
                .map_err(map_windows_error)?;
            unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) }
                .map_err(map_windows_error)?;
            if self.is_async {
                self.wait_for_event(METransformDrainComplete.0 as u32)?;
            } else {
                let config = self.config.ok_or(CodecError::NotConfigured)?;
                while self.take_output(&config)?.is_some() {}
            }
            unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) }
                .map_err(map_windows_error)?;
            self.config = None;
            self.converter = None;
            self.is_async = false;
            self.async_input_ready = false;
            Ok(())
        }

        fn shutdown(&mut self) {
            let _ = self.drain();
            if let Some(transform) = self.transform.as_ref() {
                if let Ok(shutdown) = transform.cast::<IMFShutdown>() {
                    let _ = unsafe { shutdown.Shutdown() };
                }
            }
            self.transform.take();
            self.event_generator.take();
            if self.media_foundation_started {
                // SAFETY: All Media Foundation objects have been released on the
                // startup thread before balancing MFStartup.
                let _ = unsafe { MFShutdown() };
                self.media_foundation_started = false;
            }
        }
    }

    impl Drop for NativeEncoder {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn select_hardware_h264_transform() -> Result<IMFTransform, CodecError> {
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
        let mut activations = ptr::null_mut();
        let mut activation_count = 0_u32;
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags,
                Some(&input),
                Some(&output),
                &mut activations,
                &mut activation_count,
            )
        }
        .map_err(|_| CodecError::BackendUnavailable)?;
        if activations.is_null() || activation_count == 0 {
            if !activations.is_null() {
                unsafe { CoTaskMemFree(Some(activations.cast())) };
            }
            return Err(CodecError::BackendUnavailable);
        }

        // SAFETY: MFTEnumEx returned `activation_count` initialized entries in a
        // CoTaskMem allocation. Taking each Option releases unselected COM refs
        // before the allocation itself is freed.
        let selected: Option<IMFActivate> = unsafe {
            let slice = std::slice::from_raw_parts_mut(activations, activation_count as usize);
            let mut selected = None;
            for activation in slice {
                let current = activation.take();
                if selected.is_none() {
                    selected = current;
                }
            }
            CoTaskMemFree(Some(activations.cast()));
            selected
        };
        let activation = selected.ok_or(CodecError::BackendUnavailable)?;
        unsafe { activation.ActivateObject::<IMFTransform>() }
            .map_err(|_| CodecError::BackendUnavailable)
    }

    fn create_video_type(
        config: &EncoderConfig,
        subtype: windows::core::GUID,
        compressed: bool,
    ) -> Result<windows::Win32::Media::MediaFoundation::IMFMediaType, CodecError> {
        let media_type = unsafe { MFCreateMediaType() }.map_err(map_windows_error)?;
        let frame_size = (u64::from(config.width) << 32) | u64::from(config.height);
        let frame_rate = (u64::from(config.max_fps) << 32) | 1;
        let square_pixels = (1_u64 << 32) | 1;
        unsafe { media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
            .map_err(map_windows_error)?;
        unsafe { media_type.SetGUID(&MF_MT_SUBTYPE, &subtype) }.map_err(map_windows_error)?;
        unsafe { media_type.SetUINT64(&MF_MT_FRAME_SIZE, frame_size) }
            .map_err(map_windows_error)?;
        unsafe { media_type.SetUINT64(&MF_MT_FRAME_RATE, frame_rate) }
            .map_err(map_windows_error)?;
        unsafe { media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, square_pixels) }
            .map_err(map_windows_error)?;
        unsafe {
            media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
        }
        .map_err(map_windows_error)?;
        if compressed {
            unsafe { media_type.SetUINT32(&MF_MT_AVG_BITRATE, config.bitrate_bps) }
                .map_err(map_windows_error)?;
        }
        Ok(media_type)
    }

    fn force_keyframe_on_transform(transform: &IMFTransform) -> Result<(), CodecError> {
        let codec_api = transform
            .cast::<ICodecAPI>()
            .map_err(|_| CodecError::BackendUnavailable)?;
        let force = VARIANT::from(1_u32);
        unsafe { codec_api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &force) }
            .map_err(map_windows_error)
    }

    fn create_input_sample(
        nv12: &[u8],
        frame: &CapturedFrame,
        config: &EncoderConfig,
    ) -> Result<IMFSample, CodecError> {
        let buffer_len = u32::try_from(nv12.len()).map_err(|_| CodecError::InvalidFrame)?;
        let buffer = unsafe { MFCreateMemoryBuffer(buffer_len) }.map_err(map_windows_error)?;
        let mut destination = ptr::null_mut();
        unsafe { buffer.Lock(&mut destination, None, None) }.map_err(map_windows_error)?;
        if destination.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err(CodecError::BackendLost);
        }
        unsafe { ptr::copy_nonoverlapping(nv12.as_ptr(), destination, nv12.len()) };
        unsafe { buffer.Unlock() }.map_err(map_windows_error)?;
        unsafe { buffer.SetCurrentLength(buffer_len) }.map_err(map_windows_error)?;

        let sample = unsafe { MFCreateSample() }.map_err(map_windows_error)?;
        unsafe { sample.AddBuffer(&buffer) }.map_err(map_windows_error)?;
        let sample_time = frame
            .timestamp_us
            .checked_mul(10)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(CodecError::InvalidFrame)?;
        let duration = 10_000_000_i64 / i64::from(config.max_fps);
        unsafe { sample.SetSampleTime(sample_time) }.map_err(map_windows_error)?;
        unsafe { sample.SetSampleDuration(duration) }.map_err(map_windows_error)?;
        Ok(sample)
    }

    enum Nv12Converter {
        D3d11(D3d11Nv12Converter),
        Cpu,
    }

    impl Nv12Converter {
        fn new(config: &EncoderConfig) -> Self {
            D3d11Nv12Converter::new(config.width, config.height, config.max_fps)
                .map(Self::D3d11)
                .unwrap_or(Self::Cpu)
        }

        fn convert(&mut self, frame: &CapturedFrame) -> Result<Vec<u8>, CodecError> {
            match self {
                Self::D3d11(converter) => converter.convert(frame),
                Self::Cpu => bgra_to_nv12(frame),
            }
        }
    }

    struct D3d11Nv12Converter {
        immediate: ID3D11DeviceContext,
        video_context: ID3D11VideoContext,
        processor: ID3D11VideoProcessor,
        input_texture: ID3D11Texture2D,
        output_texture: ID3D11Texture2D,
        staging_texture: ID3D11Texture2D,
        input_view: ID3D11VideoProcessorInputView,
        output_view: ID3D11VideoProcessorOutputView,
        width: u32,
        height: u32,
    }

    impl D3d11Nv12Converter {
        fn new(width: u32, height: u32, max_fps: u32) -> Result<Self, ()> {
            let mut device = None;
            let mut immediate = None;
            let mut feature_level = D3D_FEATURE_LEVEL::default();
            let flags = D3D11_CREATE_DEVICE_FLAG(
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0,
            );
            unsafe {
                D3D11CreateDevice(
                    None::<&IDXGIAdapter>,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut immediate),
                )
            }
            .map_err(|_| ())?;
            let device = device.ok_or(())?;
            let immediate = immediate.ok_or(())?;
            let video_device = device.cast::<ID3D11VideoDevice>().map_err(|_| ())?;
            let video_context = immediate.cast::<ID3D11VideoContext>().map_err(|_| ())?;

            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: max_fps,
                    Denominator: 1,
                },
                InputWidth: width,
                InputHeight: height,
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: max_fps,
                    Denominator: 1,
                },
                OutputWidth: width,
                OutputHeight: height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator =
                unsafe { video_device.CreateVideoProcessorEnumerator(&content) }.map_err(|_| ())?;
            let input_support =
                unsafe { enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM) }
                    .map_err(|_| ())?;
            let output_support = unsafe { enumerator.CheckVideoProcessorFormat(DXGI_FORMAT_NV12) }
                .map_err(|_| ())?;
            if input_support & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0 as u32 == 0
                || output_support & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32 == 0
            {
                return Err(());
            }
            let processor =
                unsafe { video_device.CreateVideoProcessor(&enumerator, 0) }.map_err(|_| ())?;

            let input_desc = texture_desc(
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                D3D11_USAGE_DEFAULT,
                0,
                0,
            );
            let output_desc = texture_desc(
                width,
                height,
                DXGI_FORMAT_NV12,
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_RENDER_TARGET.0 as u32,
                0,
            );
            let staging_desc = texture_desc(
                width,
                height,
                DXGI_FORMAT_NV12,
                D3D11_USAGE_STAGING,
                0,
                D3D11_CPU_ACCESS_READ.0 as u32,
            );
            let input_texture = create_texture(&device, &input_desc).map_err(|_| ())?;
            let output_texture = create_texture(&device, &output_desc).map_err(|_| ())?;
            let staging_texture = create_texture(&device, &staging_desc).map_err(|_| ())?;

            let input_view_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let mut input_view = None;
            unsafe {
                video_device.CreateVideoProcessorInputView(
                    &input_texture,
                    &enumerator,
                    &input_view_desc,
                    Some(&mut input_view),
                )
            }
            .map_err(|_| ())?;
            let output_view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output_view = None;
            unsafe {
                video_device.CreateVideoProcessorOutputView(
                    &output_texture,
                    &enumerator,
                    &output_view_desc,
                    Some(&mut output_view),
                )
            }
            .map_err(|_| ())?;

            Ok(Self {
                immediate,
                video_context,
                processor,
                input_texture,
                output_texture,
                staging_texture,
                input_view: input_view.ok_or(())?,
                output_view: output_view.ok_or(())?,
                width,
                height,
            })
        }

        fn convert(&mut self, frame: &CapturedFrame) -> Result<Vec<u8>, CodecError> {
            let row_pitch = self
                .width
                .checked_mul(4)
                .ok_or(CodecError::InvalidDimensions)?;
            unsafe {
                self.immediate.UpdateSubresource(
                    &self.input_texture,
                    0,
                    None,
                    frame.data.as_ptr().cast(),
                    row_pitch,
                    0,
                )
            };
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: BOOL(1),
                pInputSurface: ManuallyDrop::new(Some(self.input_view.clone())),
                ..Default::default()
            };
            let result = unsafe {
                self.video_context.VideoProcessorBlt(
                    &self.processor,
                    &self.output_view,
                    0,
                    std::slice::from_ref(&stream),
                )
            };
            // SAFETY: Releases the one cloned view stored in the ManuallyDrop
            // field after VideoProcessorBlt has returned.
            drop(unsafe { ManuallyDrop::take(&mut stream.pInputSurface) });
            result.map_err(map_windows_error)?;

            unsafe {
                self.immediate
                    .CopyResource(&self.staging_texture, &self.output_texture)
            };
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                self.immediate.Map(
                    &self.staging_texture,
                    0,
                    D3D11_MAP_READ,
                    0,
                    Some(&mut mapped),
                )
            }
            .map_err(map_windows_error)?;
            let copy_result = copy_mapped_nv12(&mapped, self.width, self.height);
            unsafe { self.immediate.Unmap(&self.staging_texture, 0) };
            copy_result
        }
    }

    fn texture_desc(
        width: u32,
        height: u32,
        format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
        usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE,
        bind_flags: u32,
        cpu_access_flags: u32,
    ) -> D3D11_TEXTURE2D_DESC {
        D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: usage,
            BindFlags: bind_flags,
            CPUAccessFlags: cpu_access_flags,
            MiscFlags: 0,
        }
    }

    fn create_texture(
        device: &ID3D11Device,
        descriptor: &D3D11_TEXTURE2D_DESC,
    ) -> windows::core::Result<ID3D11Texture2D> {
        let mut texture = None;
        unsafe { device.CreateTexture2D(descriptor, None, Some(&mut texture)) }?;
        texture.ok_or_else(|| WindowsError::from_hresult(E_FAIL))
    }

    fn copy_mapped_nv12(
        mapped: &D3D11_MAPPED_SUBRESOURCE,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, CodecError> {
        if mapped.pData.is_null() || mapped.RowPitch < width {
            return Err(CodecError::BackendLost);
        }
        let width = width as usize;
        let height = height as usize;
        let y_len = width
            .checked_mul(height)
            .ok_or(CodecError::InvalidDimensions)?;
        let mut nv12 = vec![0_u8; y_len + y_len / 2];
        let pitch = mapped.RowPitch as usize;
        let source = mapped.pData.cast::<u8>();
        for row in 0..height {
            let source_row = unsafe { std::slice::from_raw_parts(source.add(row * pitch), width) };
            nv12[row * width..(row + 1) * width].copy_from_slice(source_row);
        }
        let source_uv = unsafe { source.add(height * pitch) };
        for row in 0..height / 2 {
            let source_row =
                unsafe { std::slice::from_raw_parts(source_uv.add(row * pitch), width) };
            let destination = y_len + row * width;
            nv12[destination..destination + width].copy_from_slice(source_row);
        }
        Ok(nv12)
    }

    fn copy_buffer(buffer: &IMFMediaBuffer) -> Result<Vec<u8>, CodecError> {
        let length = unsafe { buffer.GetCurrentLength() }.map_err(map_windows_error)?;
        let mut source = ptr::null_mut();
        unsafe { buffer.Lock(&mut source, None, None) }.map_err(map_windows_error)?;
        if source.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err(CodecError::BackendLost);
        }
        let data = unsafe { std::slice::from_raw_parts(source, length as usize) }.to_vec();
        unsafe { buffer.Unlock() }.map_err(map_windows_error)?;
        Ok(data)
    }

    fn map_windows_error(error: WindowsError) -> CodecError {
        classify_hresult(error.code().0)
    }

    fn classify_hresult(code: i32) -> CodecError {
        if code == DXGI_ERROR_DEVICE_REMOVED.0
            || code == DXGI_ERROR_DEVICE_RESET.0
            || code == DXGI_ERROR_DEVICE_HUNG.0
            || code == MF_E_HW_MFT_FAILED_START_STREAMING.0
        {
            CodecError::BackendLost
        } else {
            CodecError::BackendUnavailable
        }
    }
}

impl VideoEncoder for WindowsH264Encoder {
    fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
        let validated = config.validate()?;
        self.transform.configure(validated)?;
        self.config = Some(validated);
        self.force_next_keyframe = true;
        Ok(())
    }

    fn encode(&mut self, frame: CapturedFrame) -> Result<EncodedFrame, CodecError> {
        let config = self.config.ok_or(CodecError::NotConfigured)?;
        if frame.width != config.width || frame.height != config.height {
            return Err(CodecError::FrameDimensionsMismatch);
        }
        if frame.format != PixelFormat::Bgra8 || frame.validate().is_err() {
            return Err(CodecError::InvalidFrame);
        }

        let output = self.transform.encode(frame, self.force_next_keyframe)?;
        self.force_next_keyframe = false;
        Ok(output)
    }

    fn request_keyframe(&mut self) -> Result<(), CodecError> {
        self.force_next_keyframe = true;
        Ok(())
    }

    fn reconfigure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
        let validated = config.validate()?;

        let Some(previous) = self.config.take() else {
            return self.configure(validated);
        };
        let force_next_keyframe = self.force_next_keyframe
            || previous.width != validated.width
            || previous.height != validated.height;

        self.transform.drain()?;
        self.transform.configure(validated)?;
        self.config = Some(validated);
        self.force_next_keyframe = force_next_keyframe;
        Ok(())
    }
}

impl Drop for WindowsH264Encoder {
    fn drop(&mut self) {
        if self.config.take().is_some() {
            let _ = self.transform.drain();
        }
        self.transform.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn cpu_nv12_conversion_uses_checked_planes_and_two_by_two_chroma() {
        let black = CapturedFrame::new_bgra(1, 2, 2, 2, Bytes::from_static(&[0; 16])).unwrap();

        assert_eq!(
            bgra_to_nv12(&black).unwrap(),
            vec![16, 16, 16, 16, 128, 128]
        );
    }

    #[test]
    fn cpu_nv12_conversion_rejects_odd_dimensions() {
        let frame = CapturedFrame::new_bgra(1, 2, 1, 2, Bytes::from_static(&[0; 8])).unwrap();

        assert_eq!(bgra_to_nv12(&frame), Err(CodecError::InvalidDimensions));
    }
}
