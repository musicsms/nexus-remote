use nexus_capture::{CapturedFrame, PixelFormat};
use nexus_codec::{CodecError, EncodedFrame, EncoderConfig, VideoEncoder};
#[cfg(any(windows, test))]
use std::collections::VecDeque;
#[cfg(any(windows, test))]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Receiver,
    Arc, Mutex,
};
#[cfg(any(windows, test))]
use std::thread::{self, JoinHandle};
#[cfg(any(windows, test))]
use std::time::Duration;

#[cfg(any(windows, test))]
const MAX_ASYNC_INPUT_CREDITS: usize = 8;
#[cfg(any(windows, test))]
const MAX_PENDING_OUTPUT_EVENTS: usize = 8;
#[cfg(any(windows, test))]
const MFT_OUTPUT_NO_SAMPLE_STATUS: u32 = 0x300;
#[cfg(any(windows, test))]
const MFT_OUTPUT_INCOMPLETE_STATUS: u32 = 0x0100_0000;

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncPumpEvent {
    NeedInput,
    HaveOutput,
    DrainComplete,
}

/// Bounded accounting for the asynchronous MFT event protocol.
///
/// Every `METransformNeedInput` event is one independent input credit.  A
/// Boolean loses credits when a hardware MFT pipelines multiple requests,
/// which can leave an idle hardware lane waiting forever.  Output events are
/// accounted separately because an MFT can produce zero, one, or several
/// samples between input requests.
#[cfg(any(windows, test))]
#[derive(Debug, Default)]
struct AsyncPump {
    input_credits: usize,
    pending_output_events: usize,
    drain_complete: bool,
}

#[cfg(any(windows, test))]
impl AsyncPump {
    fn new() -> Self {
        Self::default()
    }

    fn observe(&mut self, event: AsyncPumpEvent) -> Result<(), CodecError> {
        match event {
            AsyncPumpEvent::NeedInput => {
                self.input_credits = self
                    .input_credits
                    .checked_add(1)
                    .filter(|credits| *credits <= MAX_ASYNC_INPUT_CREDITS)
                    .ok_or(CodecError::BackendLost)?;
            }
            AsyncPumpEvent::HaveOutput => {
                self.pending_output_events = self
                    .pending_output_events
                    .checked_add(1)
                    .filter(|events| *events <= MAX_PENDING_OUTPUT_EVENTS)
                    .ok_or(CodecError::BackendLost)?;
            }
            AsyncPumpEvent::DrainComplete => self.drain_complete = true,
        }
        Ok(())
    }

    #[cfg(test)]
    fn input_credits(&self) -> usize {
        self.input_credits
    }

    #[cfg(test)]
    fn pending_output_events(&self) -> usize {
        self.pending_output_events
    }

    fn take_input_credit(&mut self) -> bool {
        match self.input_credits.checked_sub(1) {
            Some(credits) => {
                self.input_credits = credits;
                true
            }
            None => false,
        }
    }

    fn take_output_event(&mut self) -> bool {
        match self.pending_output_events.checked_sub(1) {
            Some(events) => {
                self.pending_output_events = events;
                true
            }
            None => false,
        }
    }

    fn take_drain_complete(&mut self) -> bool {
        std::mem::take(&mut self.drain_complete)
    }
}

/// Couples event credits to queued input commands so that an async MFT can
/// receive every input it has requested without waiting for the first output.
#[cfg(any(windows, test))]
#[derive(Debug)]
struct AsyncInputScheduler<T> {
    pump: AsyncPump,
    queued_inputs: VecDeque<T>,
}

#[cfg(any(windows, test))]
impl<T> AsyncInputScheduler<T> {
    fn new() -> Self {
        Self {
            pump: AsyncPump::new(),
            queued_inputs: VecDeque::new(),
        }
    }

    fn observe(&mut self, event: AsyncPumpEvent) -> Result<(), CodecError> {
        self.pump.observe(event)
    }

    fn enqueue(&mut self, input: T) -> Result<(), CodecError> {
        if self.queued_inputs.len() == MAX_ASYNC_INPUT_CREDITS {
            return Err(CodecError::BackendLost);
        }
        self.queued_inputs.push_back(input);
        Ok(())
    }

    fn take_ready_input(&mut self) -> Option<T> {
        if self.queued_inputs.is_empty() || !self.pump.take_input_credit() {
            return None;
        }
        self.queued_inputs.pop_front()
    }

    fn take_output_event(&mut self) -> bool {
        self.pump.take_output_event()
    }

    fn take_drain_complete(&mut self) -> bool {
        self.pump.take_drain_complete()
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutputBufferSpec {
    capacity: u32,
    alignment: Option<u32>,
}

#[cfg(any(windows, test))]
fn output_buffer_spec(cb_size: u32, cb_alignment: u32) -> Result<OutputBufferSpec, CodecError> {
    if cb_size == 0 {
        return Err(CodecError::BackendUnavailable);
    }
    let alignment = if cb_alignment == 0 {
        None
    } else if cb_alignment.is_power_of_two() {
        Some(
            cb_alignment
                .checked_sub(1)
                .ok_or(CodecError::BackendUnavailable)?,
        )
    } else {
        return Err(CodecError::BackendUnavailable);
    };
    Ok(OutputBufferSpec {
        capacity: cb_size,
        alignment,
    })
}

#[cfg(any(windows, test))]
fn output_sample_is_usable(output_status: u32, sample_len: Option<usize>) -> Option<usize> {
    if output_status & MFT_OUTPUT_NO_SAMPLE_STATUS == MFT_OUTPUT_NO_SAMPLE_STATUS {
        return None;
    }
    sample_len.filter(|length| *length != 0)
}

#[cfg(any(windows, test))]
fn output_requires_retry(output_status: u32) -> bool {
    output_status & MFT_OUTPUT_INCOMPLETE_STATUS != 0
}

#[cfg(any(windows, test))]
struct OutputPoll<T> {
    output: Option<T>,
    more_output: bool,
}

#[cfg(any(windows, test))]
fn collect_output_polls<T>(
    mut take_output: impl FnMut() -> Result<OutputPoll<T>, CodecError>,
) -> Result<Vec<T>, CodecError> {
    let mut outputs = Vec::new();
    for _ in 0..MAX_PENDING_OUTPUT_EVENTS {
        let poll = take_output()?;
        if let Some(output) = poll.output {
            outputs.push(output);
        }
        if !poll.more_output {
            return Ok(outputs);
        }
    }
    Err(CodecError::BackendLost)
}

#[cfg(any(windows, test))]
fn clear_converter_after_drain<T>(
    converter: &mut Option<T>,
    result: Result<(), CodecError>,
) -> Result<(), CodecError> {
    converter.take();
    result
}

/// Owns the native worker's `JoinHandle` after a bounded caller timeout.
///
/// A reaper holds and joins the handle on a small Rust thread. The caller can
/// return without blocking on a stalled driver, but the native worker is never
/// detached: when it exits, its COM, Media Foundation, and D3D cleanup runs
/// before the join completes.
#[cfg(any(windows, test))]
#[derive(Debug)]
struct WorkerLifecycle {
    worker: Mutex<Option<JoinHandle<()>>>,
    reaping: AtomicBool,
    finished: AtomicBool,
}

#[cfg(any(windows, test))]
impl WorkerLifecycle {
    fn new(worker: JoinHandle<()>) -> Arc<Self> {
        Arc::new(Self {
            worker: Mutex::new(Some(worker)),
            reaping: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        })
    }

    fn reap_in_background(self: &Arc<Self>) {
        if self.reaping.swap(true, Ordering::AcqRel) {
            return;
        }
        let lifecycle = Arc::clone(self);
        if thread::Builder::new()
            .name("nexus-windows-encoder-reaper".to_owned())
            .spawn(move || lifecycle.join_owned_worker())
            .is_err()
        {
            self.reaping.store(false, Ordering::Release);
        }
    }

    fn join_owned_worker(&self) {
        let worker = self
            .worker
            .lock()
            .expect("worker lifecycle lock poisoned")
            .take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
        self.finished.store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn reaping(&self) -> bool {
        self.reaping.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy)]
struct Nv12Layout {
    width: usize,
    height: usize,
    bgra_stride: usize,
    bgra_len: usize,
    y_len: usize,
    output_len: usize,
}

#[cfg(any(windows, test))]
impl Nv12Layout {
    /// Validates every multiplication that bounds the CPU and mapped-D3D
    /// copies.  Once constructed, `row < height` and `column < width` make
    /// each `row * stride + column` offset strictly less than its checked
    /// allocation length.
    fn from_bgra_dimensions(width: u32, height: u32) -> Result<Self, CodecError> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(CodecError::InvalidDimensions);
        }
        let width = usize::try_from(width).map_err(|_| CodecError::InvalidFrame)?;
        let height = usize::try_from(height).map_err(|_| CodecError::InvalidFrame)?;
        let bgra_stride = width.checked_mul(4).ok_or(CodecError::InvalidFrame)?;
        let bgra_len = bgra_stride
            .checked_mul(height)
            .ok_or(CodecError::InvalidFrame)?;
        let y_len = width.checked_mul(height).ok_or(CodecError::InvalidFrame)?;
        let output_len = y_len
            .checked_add(y_len / 2)
            .ok_or(CodecError::InvalidFrame)?;
        Ok(Self {
            width,
            height,
            bgra_stride,
            bgra_len,
            y_len,
            output_len,
        })
    }

    fn bgra_offset(self, row: usize, column: usize) -> usize {
        debug_assert!(row < self.height);
        debug_assert!(column < self.width);
        row * self.bgra_stride + column * 4
    }

    fn nv12_offset(self, row: usize, column: usize) -> usize {
        debug_assert!(row < self.height);
        debug_assert!(column < self.width);
        row * self.width + column
    }
}

#[cfg(any(windows, test))]
fn receive_response<T>(
    receiver: &Receiver<Result<T, CodecError>>,
    timeout: Duration,
) -> Result<T, CodecError> {
    receiver
        .recv_timeout(timeout)
        .unwrap_or(Err(CodecError::BackendLost))
}

#[cfg(any(windows, test))]
fn h264_access_unit(
    sequence_header: &[u8],
    access_unit: &[u8],
    keyframe: bool,
) -> Result<Vec<u8>, CodecError> {
    if access_unit.is_empty() {
        return Err(CodecError::BackendUnavailable);
    }
    if !keyframe {
        return Ok(access_unit.to_vec());
    }
    if sequence_header.is_empty() {
        return Err(CodecError::BackendUnavailable);
    }
    let capacity = sequence_header
        .len()
        .checked_add(access_unit.len())
        .ok_or(CodecError::BackendUnavailable)?;
    let mut complete_access_unit = Vec::with_capacity(capacity);
    complete_access_unit.extend_from_slice(sequence_header);
    complete_access_unit.extend_from_slice(access_unit);
    Ok(complete_access_unit)
}

#[cfg(any(windows, test))]
fn copy_pitched_nv12(
    mapped: &[u8],
    pitch: usize,
    layout: Nv12Layout,
) -> Result<Vec<u8>, CodecError> {
    if pitch < layout.width {
        return Err(CodecError::BackendLost);
    }
    let mapped_rows = layout
        .height
        .checked_add(layout.height / 2)
        .ok_or(CodecError::BackendLost)?;
    let required_len = pitch
        .checked_mul(mapped_rows)
        .ok_or(CodecError::BackendLost)?;
    if mapped.len() < required_len {
        return Err(CodecError::BackendLost);
    }

    let mut nv12 = vec![0_u8; layout.output_len];
    for row in 0..layout.height {
        let source_start = row * pitch;
        let destination_start = row * layout.width;
        nv12[destination_start..destination_start + layout.width]
            .copy_from_slice(&mapped[source_start..source_start + layout.width]);
    }
    let uv_source_start = layout.height * pitch;
    for row in 0..layout.height / 2 {
        let source_start = uv_source_start + row * pitch;
        let destination_start = layout.y_len + row * layout.width;
        nv12[destination_start..destination_start + layout.width]
            .copy_from_slice(&mapped[source_start..source_start + layout.width]);
    }
    Ok(nv12)
}

/// Private adapter around the platform-owned encoder transform.
///
/// Deterministic transforms are available only to this module's unit tests;
/// dependency builds cannot replace the Media Foundation backend.
trait EncoderTransform: Send {
    fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError>;

    fn encode(
        &mut self,
        frame: CapturedFrame,
        force_keyframe: bool,
    ) -> Result<Vec<EncodedFrame>, CodecError>;

    fn drain(&mut self) -> Result<(), CodecError>;

    fn shutdown(&mut self);
}

#[cfg(any(windows, test))]
fn bgra_to_nv12(frame: &CapturedFrame) -> Result<Vec<u8>, CodecError> {
    if frame.format != PixelFormat::Bgra8 || frame.validate().is_err() {
        return Err(CodecError::InvalidFrame);
    }

    let layout = Nv12Layout::from_bgra_dimensions(frame.width, frame.height)?;
    if frame.data.len() != layout.bgra_len {
        return Err(CodecError::InvalidFrame);
    }
    let mut nv12 = vec![0_u8; layout.output_len];

    for row in 0..layout.height {
        for column in 0..layout.width {
            let source = layout.bgra_offset(row, column);
            let b = i32::from(frame.data[source]);
            let g = i32::from(frame.data[source + 1]);
            let r = i32::from(frame.data[source + 2]);
            nv12[layout.nv12_offset(row, column)] =
                clamp_byte(((66 * r + 129 * g + 25 * b + 128) >> 8) + 16);
        }
    }

    for row in (0..layout.height).step_by(2) {
        for column in (0..layout.width).step_by(2) {
            let mut u_sum = 0_i32;
            let mut v_sum = 0_i32;
            for row_offset in 0..2 {
                for column_offset in 0..2 {
                    let source = layout.bgra_offset(row + row_offset, column + column_offset);
                    let b = i32::from(frame.data[source]);
                    let g = i32::from(frame.data[source + 1]);
                    let r = i32::from(frame.data[source + 2]);
                    u_sum += ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                    v_sum += ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                }
            }
            let destination = layout.y_len + (row / 2) * layout.width + column;
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
            Ok(Self {
                config: None,
                transform: Box::new(native::MediaFoundationTransform::start()?),
                force_next_keyframe: false,
            })
        }
        #[cfg(not(windows))]
        {
            Err(CodecError::BackendUnavailable)
        }
    }

    #[cfg(test)]
    fn with_transform<T>(transform: T) -> Self
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
    use super::{
        bgra_to_nv12, clear_converter_after_drain, collect_output_polls, copy_pitched_nv12,
        h264_access_unit, output_buffer_spec, output_requires_retry, output_sample_is_usable,
        receive_response, AsyncInputScheduler, AsyncPumpEvent, EncoderTransform, Nv12Layout,
        OutputPoll, WorkerLifecycle,
    };
    use bytes::Bytes;
    use nexus_capture::{CapturedFrame, PixelFormat};
    use nexus_codec::{CodecError, EncodedFrame, EncoderConfig};
    use std::collections::VecDeque;
    use std::mem::ManuallyDrop;
    use std::ptr;
    use std::sync::{
        mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender},
        Arc,
    };
    use std::thread;
    use std::time::{Duration, Instant};
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
        eAVEncH264VProfile_ConstrainedBase, CODECAPI_AVEncCommonMeanBitRate,
        CODECAPI_AVEncVideoForceKeyFrame, ICodecAPI, IMFActivate, IMFMediaBuffer,
        IMFMediaEventGenerator, IMFSample, IMFShutdown, IMFTransform, METransformDrainComplete,
        METransformHaveOutput, METransformNeedInput, MFCreateAlignedMemoryBuffer,
        MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
        MFSampleExtension_CleanPoint, MFShutdown, MFStartup, MFTEnumEx, MFVideoFormat_H264,
        MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
        MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_HARDWARE,
        MFT_ENUM_FLAG_SORTANDFILTER, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_COMMAND_FLUSH,
        MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM,
        MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
        MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
        MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_EVENT_FLAG_NO_WAIT,
        MF_E_HW_MFT_FAILED_START_STREAMING, MF_E_NO_EVENTS_AVAILABLE,
        MF_E_TRANSFORM_NEED_MORE_INPUT, MF_LOW_LATENCY, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE,
        MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE,
        MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC,
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
            SyncSender<Result<Vec<EncodedFrame>, CodecError>>,
        ),
        Drain(SyncSender<Result<(), CodecError>>),
        Shutdown(SyncSender<()>),
    }

    const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);
    const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(2);
    const MAX_PENDING_FRAMES: usize = 8;

    pub(super) struct MediaFoundationTransform {
        command_tx: Option<SyncSender<WorkerCommand>>,
        lifecycle: Arc<WorkerLifecycle>,
    }

    impl MediaFoundationTransform {
        pub(super) fn start() -> Result<Self, CodecError> {
            let (command_tx, command_rx) = sync_channel(1);
            let (startup_tx, startup_rx) = sync_channel(1);
            let worker = thread::Builder::new()
                .name("nexus-windows-encoder".to_owned())
                .spawn(move || worker_main(command_rx, startup_tx))
                .map_err(|_| CodecError::BackendUnavailable)?;
            let lifecycle = WorkerLifecycle::new(worker);

            match startup_rx.recv_timeout(STARTUP_TIMEOUT) {
                Ok(Ok(())) => Ok(Self {
                    command_tx: Some(command_tx),
                    lifecycle,
                }),
                Ok(Err(error)) => {
                    lifecycle.reap_in_background();
                    Err(error)
                }
                Err(_) => {
                    // A driver can stall during MFStartup or transform activation.
                    // Retain the worker's handle in a reaper so the bounded
                    // constructor does not detach native COM/MF/D3D ownership.
                    lifecycle.reap_in_background();
                    Err(CodecError::BackendUnavailable)
                }
            }
        }

        fn request<T>(
            &mut self,
            build_command: impl FnOnce(SyncSender<Result<T, CodecError>>) -> WorkerCommand,
        ) -> Result<T, CodecError> {
            let (response_tx, response_rx) = sync_channel(1);
            let command = build_command(response_tx);
            let send_result = self
                .command_tx
                .as_ref()
                .ok_or(CodecError::BackendLost)?
                .try_send(command);
            if send_result.is_err() {
                self.command_tx.take();
                return Err(CodecError::BackendLost);
            }
            let response = receive_response(&response_rx, REQUEST_TIMEOUT);
            if response.is_err() {
                self.command_tx.take();
            }
            response
        }

        fn stop_worker(&mut self) {
            if let Some(sender) = self.command_tx.take() {
                let (response_tx, response_rx) = sync_channel(1);
                if sender
                    .try_send(WorkerCommand::Shutdown(response_tx))
                    .is_ok()
                {
                    let _ = response_rx.recv_timeout(SHUTDOWN_TIMEOUT);
                }
            }
            self.lifecycle.reap_in_background();
        }
    }

    impl EncoderTransform for MediaFoundationTransform {
        fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
            self.request(|response| WorkerCommand::Configure(config, response))
        }

        fn encode(
            &mut self,
            frame: CapturedFrame,
            force_keyframe: bool,
        ) -> Result<Vec<EncodedFrame>, CodecError> {
            self.request(|response| WorkerCommand::Encode(frame, force_keyframe, response))
        }

        fn drain(&mut self) -> Result<(), CodecError> {
            self.request(WorkerCommand::Drain)
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

        loop {
            match command_rx.recv_timeout(EVENT_POLL_INTERVAL) {
                Ok(WorkerCommand::Configure(config, response)) => {
                    let _ = response.send(encoder.configure(config));
                }
                Ok(WorkerCommand::Encode(frame, force_keyframe, response)) => {
                    let _ = response.send(encoder.encode(frame, force_keyframe));
                }
                Ok(WorkerCommand::Drain(response)) => {
                    let _ = response.send(encoder.drain());
                }
                Ok(WorkerCommand::Shutdown(response)) => {
                    encoder.shutdown();
                    let _ = response.send(());
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Keep pumping asynchronous MFT events while no caller is
                    // blocked in a command response. Delayed outputs are held
                    // for the next encode pump; a terminal native error ends
                    // the worker and causes later requests to fail closed.
                    if encoder.pump().is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
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
        async_inputs: AsyncInputScheduler<PendingSubmission>,
        config: Option<EncoderConfig>,
        converter: Option<Nv12Converter>,
        sequence_header: Option<Vec<u8>>,
        pending_inputs: VecDeque<PendingInput>,
        pending_outputs: VecDeque<EncodedFrame>,
        media_foundation_started: bool,
    }

    struct PendingInput {
        frame_id: u64,
        timestamp_us: u64,
        timestamp_hns: i64,
        forced_keyframe: bool,
    }

    struct PendingSubmission {
        sample: IMFSample,
        input: PendingInput,
    }

    struct NativeOutput {
        timestamp_hns: Option<i64>,
        keyframe: bool,
        data: Vec<u8>,
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
                        async_inputs: AsyncInputScheduler::new(),
                        config: None,
                        converter: None,
                        sequence_header: None,
                        pending_inputs: VecDeque::new(),
                        pending_outputs: VecDeque::new(),
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

            let negotiated_output_type =
                unsafe { transform.GetOutputCurrentType(0) }.map_err(map_windows_error)?;
            let sequence_header = sequence_header_from_type(&negotiated_output_type)?;
            self.converter = Some(Nv12Converter::new(&config));
            self.config = Some(config);
            self.is_async = is_async;
            self.async_inputs = AsyncInputScheduler::new();
            self.sequence_header = Some(sequence_header);
            self.pending_inputs.clear();
            self.pending_outputs.clear();
            Ok(())
        }

        fn encode(
            &mut self,
            frame: CapturedFrame,
            force_keyframe: bool,
        ) -> Result<Vec<EncodedFrame>, CodecError> {
            let config = self.config.ok_or(CodecError::NotConfigured)?;
            let nv12 = self
                .converter
                .as_mut()
                .ok_or(CodecError::NotConfigured)?
                .convert(&frame)?;
            let input = create_input_sample(&nv12, &frame, &config)?;
            let pending_input = PendingInput {
                frame_id: frame.frame_id,
                timestamp_us: frame.timestamp_us,
                timestamp_hns: frame
                    .timestamp_us
                    .checked_mul(10)
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or(CodecError::InvalidFrame)?,
                forced_keyframe: force_keyframe,
            };

            if self.is_async {
                self.async_inputs.enqueue(PendingSubmission {
                    sample: input,
                    input: pending_input,
                })?;
                self.pump_async(&config)?;
                Ok(self.take_pending_outputs())
            } else {
                let transform = self.transform()?.clone();
                if force_keyframe {
                    force_keyframe_on_transform(&transform)?;
                }
                // SAFETY: The sample owns a complete NV12 buffer and timestamps
                // are expressed in Media Foundation's 100-nanosecond units.
                unsafe { transform.ProcessInput(0, &input, 0) }.map_err(map_windows_error)?;
                self.queue_input(pending_input)?;
                self.collect_synchronous_output(&config)?;
                Ok(self.take_pending_outputs())
            }
        }

        fn submit_credited_inputs(&mut self) -> Result<(), CodecError> {
            let transform = self.transform()?.clone();
            while let Some(submission) = self.async_inputs.take_ready_input() {
                if submission.input.forced_keyframe {
                    force_keyframe_on_transform(&transform)?;
                }
                // SAFETY: This worker owns the MFT and each queued sample owns
                // a complete NV12 input buffer. Credits are consumed exactly
                // once by AsyncInputScheduler before ProcessInput.
                unsafe { transform.ProcessInput(0, &submission.sample, 0) }
                    .map_err(map_windows_error)?;
                self.queue_input(submission.input)?;
            }
            Ok(())
        }

        fn pump(&mut self) -> Result<(), CodecError> {
            if self.is_async {
                let config = self.config.ok_or(CodecError::NotConfigured)?;
                self.pump_async(&config)?;
            }
            Ok(())
        }

        fn pump_async(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
            self.poll_async_events(config)?;
            self.submit_credited_inputs()?;
            self.poll_async_events(config)
        }

        fn take_pending_outputs(&mut self) -> Vec<EncodedFrame> {
            self.pending_outputs.drain(..).collect()
        }

        fn wait_for_drain_complete(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
            let deadline = Instant::now() + REQUEST_TIMEOUT;
            loop {
                self.poll_async_events(config)?;
                if self.async_inputs.take_drain_complete() {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(CodecError::BackendLost);
                }
                thread::sleep(EVENT_POLL_INTERVAL);
            }
        }

        fn poll_async_events(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
            let generator = self
                .event_generator
                .as_ref()
                .ok_or(CodecError::BackendUnavailable)?
                .clone();
            loop {
                // SAFETY: MF_EVENT_FLAG_NO_WAIT guarantees this polling call
                // never blocks the worker. The deadline in each caller bounds
                // a driver that never queues another event.
                let event = match unsafe { generator.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                    Ok(event) => event,
                    Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                    Err(error) => return Err(map_windows_error(error)),
                };
                let status = unsafe { event.GetStatus() }.map_err(map_windows_error)?;
                status.ok().map_err(|_| classify_hresult(status.0))?;
                let event_type = unsafe { event.GetType() }.map_err(map_windows_error)?;
                if event_type == METransformNeedInput.0 as u32 {
                    self.async_inputs.observe(AsyncPumpEvent::NeedInput)?;
                } else if event_type == METransformHaveOutput.0 as u32 {
                    self.async_inputs.observe(AsyncPumpEvent::HaveOutput)?;
                    self.collect_async_output(config)?;
                } else if event_type == METransformDrainComplete.0 as u32 {
                    self.async_inputs.observe(AsyncPumpEvent::DrainComplete)?;
                }
            }
        }

        fn collect_async_output(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
            if !self.async_inputs.take_output_event() {
                return Err(CodecError::BackendLost);
            }
            self.collect_synchronous_output(config)
        }

        fn collect_synchronous_output(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
            for output in collect_output_polls(|| self.take_output(config))? {
                self.queue_output(output)?;
            }
            Ok(())
        }

        fn queue_input(&mut self, input: PendingInput) -> Result<(), CodecError> {
            if self.pending_inputs.len() == MAX_PENDING_FRAMES {
                return Err(CodecError::BackendLost);
            }
            self.pending_inputs.push_back(input);
            Ok(())
        }

        fn queue_output(&mut self, output: NativeOutput) -> Result<(), CodecError> {
            let input_index = output
                .timestamp_hns
                .and_then(|timestamp| {
                    self.pending_inputs
                        .iter()
                        .position(|input| input.timestamp_hns == timestamp)
                })
                .unwrap_or(0);
            let input = self
                .pending_inputs
                .remove(input_index)
                .ok_or(CodecError::BackendLost)?;
            if input.forced_keyframe && !output.keyframe {
                return Err(CodecError::BackendUnavailable);
            }
            if self.pending_outputs.len() == MAX_PENDING_FRAMES {
                return Err(CodecError::BackendLost);
            }
            self.pending_outputs.push_back(EncodedFrame {
                frame_id: input.frame_id,
                timestamp_us: input.timestamp_us,
                keyframe: output.keyframe,
                data: Bytes::from(output.data),
            });
            Ok(())
        }

        fn take_output(
            &self,
            _config: &EncoderConfig,
        ) -> Result<OutputPoll<NativeOutput>, CodecError> {
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
                let buffer_spec = output_buffer_spec(stream_info.cbSize, stream_info.cbAlignment)?;
                let sample = unsafe { MFCreateSample() }.map_err(map_windows_error)?;
                let buffer = match buffer_spec.alignment {
                    Some(alignment) => {
                        unsafe { MFCreateAlignedMemoryBuffer(buffer_spec.capacity, alignment) }
                            .map_err(map_windows_error)?
                    }
                    None => unsafe { MFCreateMemoryBuffer(buffer_spec.capacity) }
                        .map_err(map_windows_error)?,
                };
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
                    // `status` is `_MFT_PROCESS_OUTPUT_STATUS`, whose current
                    // flag is NEW_STREAMS; it is not an output-ready flag. The
                    // per-buffer INCOMPLETE flag is what requires another
                    // ProcessOutput call.
                    let _process_output_status = status;
                    let more_output = output_requires_retry(output.dwStatus);
                    if output.dwStatus & MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE.0 as u32
                        == MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE.0 as u32
                    {
                        return Ok(OutputPoll {
                            output: None,
                            more_output,
                        });
                    }
                    let Some(sample) = sample else {
                        return Ok(OutputPoll {
                            output: None,
                            more_output,
                        });
                    };
                    let keyframe = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }
                        .unwrap_or(0)
                        != 0;
                    let buffer =
                        unsafe { sample.ConvertToContiguousBuffer() }.map_err(map_windows_error)?;
                    let data = copy_buffer(&buffer)?;
                    if output_sample_is_usable(output.dwStatus, Some(data.len())).is_none() {
                        return Ok(OutputPoll {
                            output: None,
                            more_output,
                        });
                    }
                    let sequence_header = self
                        .sequence_header
                        .as_deref()
                        .ok_or(CodecError::BackendUnavailable)?;
                    let data = h264_access_unit(sequence_header, &data, keyframe)?;
                    Ok(OutputPoll {
                        output: Some(NativeOutput {
                            timestamp_hns: unsafe { sample.GetSampleTime() }.ok(),
                            keyframe,
                            data,
                        }),
                        more_output,
                    })
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(OutputPoll {
                    output: None,
                    more_output: false,
                }),
                Err(error) => Err(map_windows_error(error)),
            }
        }

        fn drain(&mut self) -> Result<(), CodecError> {
            if self.config.is_none() {
                return Ok(());
            }
            let transform = self.transform()?.clone();
            let config = self.config.ok_or(CodecError::NotConfigured)?;
            let result = (|| {
                // SAFETY: These messages close the configured input stream
                // before draining delayed output and flushing transform state.
                unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0) }
                    .map_err(map_windows_error)?;
                unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) }
                    .map_err(map_windows_error)?;
                if self.is_async {
                    self.wait_for_drain_complete(&config)?;
                } else {
                    self.collect_synchronous_output(&config)?;
                }
                unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) }
                    .map_err(map_windows_error)
            })();
            let result = clear_converter_after_drain(&mut self.converter, result);
            self.config = None;
            self.is_async = false;
            self.async_inputs = AsyncInputScheduler::new();
            self.sequence_header = None;
            self.pending_inputs.clear();
            self.pending_outputs.clear();
            result
        }

        fn shutdown(&mut self) {
            let _ = self.drain();
            // Drain can fail when a driver is already lost. Release the CPU/GPU
            // converter before COM apartment teardown regardless, because it
            // owns D3D interfaces created on this worker.
            self.converter.take();
            self.config = None;
            self.sequence_header = None;
            self.pending_inputs.clear();
            self.pending_outputs.clear();
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
            // Constrained baseline avoids decoder-side CABAC and B-frame
            // requirements for the first remote-desktop operating point. A
            // hardware MFT that cannot negotiate it fails closed at SetOutputType.
            unsafe {
                media_type.SetUINT32(
                    &MF_MT_MPEG2_PROFILE,
                    eAVEncH264VProfile_ConstrainedBase.0 as u32,
                )
            }
            .map_err(map_windows_error)?;
        }
        Ok(media_type)
    }

    fn sequence_header_from_type(
        media_type: &windows::Win32::Media::MediaFoundation::IMFMediaType,
    ) -> Result<Vec<u8>, CodecError> {
        let size = unsafe { media_type.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) }
            .map_err(|_| CodecError::BackendUnavailable)?;
        if size == 0 {
            return Err(CodecError::BackendUnavailable);
        }
        let mut sequence_header = vec![0_u8; size as usize];
        let mut actual_size = 0_u32;
        unsafe {
            media_type.GetBlob(
                &MF_MT_MPEG_SEQUENCE_HEADER,
                &mut sequence_header,
                Some(&mut actual_size),
            )
        }
        .map_err(|_| CodecError::BackendUnavailable)?;
        sequence_header.truncate(actual_size as usize);
        if sequence_header.is_empty() || !sequence_header.windows(3).any(|bytes| bytes == [0, 0, 1])
        {
            return Err(CodecError::BackendUnavailable);
        }
        Ok(sequence_header)
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
        let layout = Nv12Layout::from_bgra_dimensions(config.width, config.height)?;
        if nv12.len() != layout.output_len {
            return Err(CodecError::InvalidFrame);
        }
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
            if frame.width != self.width || frame.height != self.height {
                return Err(CodecError::FrameDimensionsMismatch);
            }
            if frame.format != PixelFormat::Bgra8 || frame.validate().is_err() {
                return Err(CodecError::InvalidFrame);
            }
            let layout = Nv12Layout::from_bgra_dimensions(self.width, self.height)?;
            if frame.data.len() != layout.bgra_len {
                return Err(CodecError::InvalidFrame);
            }
            let row_pitch =
                u32::try_from(layout.bgra_stride).map_err(|_| CodecError::InvalidFrame)?;
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
        let layout = Nv12Layout::from_bgra_dimensions(width, height)?;
        let pitch = usize::try_from(mapped.RowPitch).map_err(|_| CodecError::BackendLost)?;
        if mapped.pData.is_null() || pitch < layout.width {
            return Err(CodecError::BackendLost);
        }
        let mapped_rows = layout
            .height
            .checked_add(layout.height / 2)
            .ok_or(CodecError::BackendLost)?;
        let mapped_len = pitch
            .checked_mul(mapped_rows)
            .ok_or(CodecError::BackendLost)?;
        // SAFETY: `staging_texture` is created as an NV12 texture with this
        // checked width/height. A successful D3D11 Map exposes exactly one Y
        // plane plus one half-height UV plane, each with `RowPitch` bytes per
        // row. `mapped_len` is their checked total, and `copy_pitched_nv12`
        // verifies every row range before indexing the resulting slice.
        let source = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped_len) };
        copy_pitched_nv12(source, pitch, layout)
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

    fn encode(&mut self, frame: CapturedFrame) -> Result<Vec<EncodedFrame>, CodecError> {
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
    use std::sync::{mpsc::sync_channel, Arc, Mutex};
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TransformCall {
        Configure(EncoderConfig),
        Encode { frame_id: u64, force_keyframe: bool },
        Drain,
        Shutdown,
    }

    struct RecordingTransform {
        calls: Arc<Mutex<Vec<TransformCall>>>,
        output_batches: VecDeque<Vec<EncodedFrame>>,
        fail_configure_call: Option<usize>,
        configure_count: usize,
    }

    impl EncoderTransform for RecordingTransform {
        fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
            self.configure_count += 1;
            self.calls
                .lock()
                .unwrap()
                .push(TransformCall::Configure(config));
            if self.fail_configure_call == Some(self.configure_count) {
                return Err(CodecError::BackendLost);
            }
            Ok(())
        }

        fn encode(
            &mut self,
            frame: CapturedFrame,
            force_keyframe: bool,
        ) -> Result<Vec<EncodedFrame>, CodecError> {
            self.calls.lock().unwrap().push(TransformCall::Encode {
                frame_id: frame.frame_id,
                force_keyframe,
            });
            Ok(self.output_batches.pop_front().unwrap_or_else(|| {
                vec![EncodedFrame {
                    frame_id: frame.frame_id,
                    timestamp_us: frame.timestamp_us,
                    keyframe: force_keyframe,
                    data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
                }]
            }))
        }

        fn drain(&mut self) -> Result<(), CodecError> {
            self.calls.lock().unwrap().push(TransformCall::Drain);
            Ok(())
        }

        fn shutdown(&mut self) {
            self.calls.lock().unwrap().push(TransformCall::Shutdown);
        }
    }

    fn config(width: u32, height: u32, bitrate_bps: u32) -> EncoderConfig {
        EncoderConfig {
            codec: nexus_codec::CodecKind::H264,
            width,
            height,
            max_fps: 30,
            bitrate_bps,
        }
    }

    fn frame(frame_id: u64, width: u32, height: u32) -> CapturedFrame {
        CapturedFrame {
            frame_id,
            timestamp_us: frame_id * 33_333,
            width,
            height,
            format: PixelFormat::Bgra8,
            data: Bytes::from(vec![0x80; (width * height * 4) as usize]),
        }
    }

    fn encoder(
        output_batches: VecDeque<Vec<EncodedFrame>>,
        fail_configure_call: Option<usize>,
    ) -> (WindowsH264Encoder, Arc<Mutex<Vec<TransformCall>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transform = RecordingTransform {
            calls: Arc::clone(&calls),
            output_batches,
            fail_configure_call,
            configure_count: 0,
        };
        (WindowsH264Encoder::with_transform(transform), calls)
    }

    #[test]
    fn private_state_machine_rejects_invalid_configuration_before_transform_calls() {
        let (mut encoder, calls) = encoder(VecDeque::new(), None);

        assert_eq!(
            encoder.configure(config(0, 64, 1_000_000)),
            Err(CodecError::InvalidDimensions)
        );
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn private_state_machine_returns_no_output_until_the_transform_has_one() {
        let delayed = EncodedFrame {
            frame_id: 1,
            timestamp_us: 33_333,
            keyframe: true,
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
        };
        let (mut encoder, calls) =
            encoder(VecDeque::from([Vec::new(), vec![delayed.clone()]]), None);
        encoder.configure(config(64, 64, 1_000_000)).unwrap();

        assert!(encoder.encode(frame(1, 64, 64)).unwrap().is_empty());
        assert_eq!(encoder.encode(frame(2, 64, 64)).unwrap(), vec![delayed]);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[
                TransformCall::Configure(config(64, 64, 1_000_000)),
                TransformCall::Encode {
                    frame_id: 1,
                    force_keyframe: true,
                },
                TransformCall::Encode {
                    frame_id: 2,
                    force_keyframe: false,
                },
            ]
        );
    }

    #[test]
    fn private_state_machine_dimension_reconfiguration_drains_and_forces_keyframe() {
        let (mut encoder, calls) = encoder(VecDeque::new(), None);
        encoder.configure(config(64, 64, 1_000_000)).unwrap();
        encoder.encode(frame(1, 64, 64)).unwrap();

        encoder.reconfigure(config(128, 64, 2_000_000)).unwrap();
        let outputs = encoder.encode(frame(2, 128, 64)).unwrap();

        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].keyframe);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[
                TransformCall::Configure(config(64, 64, 1_000_000)),
                TransformCall::Encode {
                    frame_id: 1,
                    force_keyframe: true,
                },
                TransformCall::Drain,
                TransformCall::Configure(config(128, 64, 2_000_000)),
                TransformCall::Encode {
                    frame_id: 2,
                    force_keyframe: true,
                },
            ]
        );
    }

    #[test]
    fn private_state_machine_failed_reconfiguration_leaves_encoder_unconfigured() {
        let (mut encoder, calls) = encoder(VecDeque::new(), Some(2));
        encoder.configure(config(64, 64, 1_000_000)).unwrap();

        assert_eq!(
            encoder.reconfigure(config(128, 64, 2_000_000)),
            Err(CodecError::BackendLost)
        );
        assert_eq!(
            encoder.encode(frame(1, 64, 64)),
            Err(CodecError::NotConfigured)
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[
                TransformCall::Configure(config(64, 64, 1_000_000)),
                TransformCall::Drain,
                TransformCall::Configure(config(128, 64, 2_000_000)),
            ]
        );
    }

    #[test]
    fn private_state_machine_drop_drains_before_transform_shutdown() {
        let (mut encoder, calls) = encoder(VecDeque::new(), None);
        encoder.configure(config(64, 64, 1_000_000)).unwrap();

        drop(encoder);

        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[
                TransformCall::Configure(config(64, 64, 1_000_000)),
                TransformCall::Drain,
                TransformCall::Shutdown,
            ]
        );
    }

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

    #[test]
    fn async_pump_preserves_each_need_input_credit_before_output_arrives() {
        let mut pump = AsyncPump::new();

        pump.observe(AsyncPumpEvent::NeedInput).unwrap();
        pump.observe(AsyncPumpEvent::NeedInput).unwrap();
        pump.observe(AsyncPumpEvent::HaveOutput).unwrap();
        pump.observe(AsyncPumpEvent::DrainComplete).unwrap();

        assert_eq!(pump.input_credits(), 2);
        assert_eq!(pump.pending_output_events(), 1);
        assert!(pump.take_input_credit());
        assert!(pump.take_input_credit());
        assert!(!pump.take_input_credit());
        assert!(pump.take_output_event());
        assert!(pump.take_drain_complete());
    }

    #[test]
    fn async_input_scheduler_submits_multiple_credited_inputs_before_any_output() {
        let mut scheduler = AsyncInputScheduler::new();
        scheduler.enqueue(1_u64).unwrap();
        scheduler.enqueue(2_u64).unwrap();

        scheduler.observe(AsyncPumpEvent::NeedInput).unwrap();
        scheduler.observe(AsyncPumpEvent::NeedInput).unwrap();

        assert_eq!(scheduler.take_ready_input(), Some(1));
        assert_eq!(scheduler.take_ready_input(), Some(2));
        assert_eq!(scheduler.take_ready_input(), None);

        scheduler.observe(AsyncPumpEvent::HaveOutput).unwrap();
        assert!(scheduler.take_output_event());
        scheduler.observe(AsyncPumpEvent::DrainComplete).unwrap();
        assert!(scheduler.take_drain_complete());
    }

    #[test]
    fn output_buffer_spec_converts_mft_alignment_to_a_mask() {
        assert_eq!(
            output_buffer_spec(4_096, 64),
            Ok(OutputBufferSpec {
                capacity: 4_096,
                alignment: Some(63),
            })
        );
    }

    #[test]
    fn output_buffer_spec_rejects_a_non_power_of_two_alignment() {
        assert_eq!(
            output_buffer_spec(4_096, 48),
            Err(CodecError::BackendUnavailable)
        );
    }

    #[test]
    fn output_without_a_sample_is_not_emitted() {
        assert_eq!(output_sample_is_usable(0x300, Some(5)), None);
        assert_eq!(output_sample_is_usable(0, Some(0)), None);
        assert_eq!(output_sample_is_usable(0, Some(5)), Some(5));
    }

    #[test]
    fn incomplete_output_status_requests_another_process_output_call() {
        assert!(output_requires_retry(0x0100_0000));
        assert!(!output_requires_retry(0));
    }

    #[test]
    fn incomplete_output_retries_until_the_buffer_is_complete() {
        let mut statuses = VecDeque::from([0x0100_0000, 0]);

        let outputs = collect_output_polls(|| {
            let status = statuses.pop_front().unwrap();
            Ok(OutputPoll {
                output: Some(status),
                more_output: output_requires_retry(status),
            })
        })
        .unwrap();

        assert_eq!(outputs, vec![0x0100_0000, 0]);
    }

    #[test]
    fn keyframes_are_prefixed_with_the_negotiated_annex_b_sequence_header() {
        let sequence_header = [0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x68];
        let access_unit = [0, 0, 0, 1, 0x65, 0x88];

        assert_eq!(
            h264_access_unit(&sequence_header, &access_unit, true).unwrap(),
            [sequence_header.as_slice(), access_unit.as_slice()].concat()
        );
        assert_eq!(
            h264_access_unit(&sequence_header, &access_unit, false).unwrap(),
            access_unit
        );
        assert_eq!(
            h264_access_unit(&[], &access_unit, true),
            Err(CodecError::BackendUnavailable)
        );
    }

    #[test]
    fn padded_nv12_rows_copy_only_the_valid_y_and_uv_bytes() {
        let layout = Nv12Layout::from_bgra_dimensions(2, 2).unwrap();
        let mapped = [1, 2, 99, 99, 3, 4, 99, 99, 5, 6, 99, 99];

        assert_eq!(
            copy_pitched_nv12(&mapped, 4, layout).unwrap(),
            vec![1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn nv12_layout_rejects_source_size_overflow_before_indexing() {
        assert!(matches!(
            Nv12Layout::from_bgra_dimensions(u32::MAX - 1, u32::MAX - 1),
            Err(CodecError::InvalidFrame)
        ));
    }

    #[test]
    fn response_wait_returns_backend_lost_when_the_worker_does_not_reply() {
        let (_sender, receiver) = sync_channel::<Result<(), CodecError>>(1);

        assert_eq!(
            receive_response(&receiver, Duration::ZERO),
            Err(CodecError::BackendLost)
        );
    }

    #[test]
    fn failed_drain_releases_converter_before_the_apartment_can_drop() {
        let mut converter = Some("d3d converter");

        assert_eq!(
            clear_converter_after_drain(&mut converter, Err(CodecError::BackendLost)),
            Err(CodecError::BackendLost)
        );
        assert_eq!(converter, None);
    }

    #[test]
    fn owned_worker_reaper_keeps_the_native_join_handle_until_cleanup() {
        let (release_tx, release_rx) = sync_channel::<()>(1);
        let (finished_tx, finished_rx) = sync_channel::<()>(1);
        let lifecycle = WorkerLifecycle::new(std::thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = finished_tx.send(());
        }));

        lifecycle.reap_in_background();
        assert!(lifecycle.reaping());
        assert!(!lifecycle.finished());

        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !lifecycle.finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            lifecycle.finished(),
            "the worker join handle was not reaped after the worker exited"
        );
    }
}
