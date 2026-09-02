//! Private decode-surface contracts and Windows Media Foundation boundary.
#![expect(
    dead_code,
    reason = "Task 5 owns runtime wiring; these private contracts intentionally exist before it"
)]

use crate::{renderer::validate_frame, DecodedFrameJob, RenderQueueError};
use thiserror::Error;

pub(crate) const MAX_SURFACE_WIDTH: u32 = 7_680;
pub(crate) const MAX_SURFACE_HEIGHT: u32 = 4_320;
const MAX_SURFACE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceFormat {
    Nv12,
    Rgba8,
}

/// Portable decoded metadata. Pixel bytes never leave the native media/render
/// boundary except as this owned, validated handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedSurface {
    pub(crate) frame_id: u32,
    pub(crate) timestamp_us: u64,
    pub(crate) keyframe: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: SurfaceFormat,
    pub(crate) bytes: Vec<u8>,
}

impl DecodedSurface {
    pub(crate) fn validate(&self) -> Result<(), DecoderError> {
        let pixels = validated_pixel_count(self.width, self.height)?;
        let expected = match self.format {
            SurfaceFormat::Nv12 => pixels.checked_add(pixels / 2),
            SurfaceFormat::Rgba8 => pixels.checked_mul(4),
        }
        .ok_or(DecoderError::InvalidDimensions)?;
        if expected > MAX_SURFACE_BYTES || self.bytes.len() != expected {
            return Err(DecoderError::InvalidSurface);
        }
        Ok(())
    }
}

fn validated_pixel_count(width: u32, height: u32) -> Result<usize, DecoderError> {
    if width == 0
        || height == 0
        || width > MAX_SURFACE_WIDTH
        || height > MAX_SURFACE_HEIGHT
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
    {
        return Err(DecoderError::InvalidDimensions);
    }
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(DecoderError::InvalidDimensions)?;
    Ok(pixels)
}

fn validated_nv12_len(width: u32, height: u32) -> Result<usize, DecoderError> {
    let pixels = validated_pixel_count(width, height)?;
    let bytes = pixels
        .checked_add(pixels / 2)
        .ok_or(DecoderError::InvalidDimensions)?;
    if bytes > MAX_SURFACE_BYTES {
        return Err(DecoderError::InvalidDimensions);
    }
    Ok(bytes)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum DecoderError {
    #[error("decoded frame is invalid: {0}")]
    Frame(#[from] RenderQueueError),
    #[error("decoder requires a keyframe carrying an H.264 sequence header")]
    MissingSequenceHeader,
    #[error("decoded surface dimensions are invalid")]
    InvalidDimensions,
    #[error("decoded surface bytes do not match its format and dimensions")]
    InvalidSurface,
    #[error("native media backend is unavailable")]
    BackendUnavailable,
    #[error("native media backend was lost or did not respond before its deadline")]
    BackendLost,
}

/// Private implementation contract. It has no Tokio or UI dependency and
/// never emits a surface unless the source job was authenticated upstream.
pub(crate) trait FrameDecoder {
    fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError>;
}

/// Tracks the only stream transition that needs an H.264 sequence header:
/// decoder initialization/recovery. Once initialized, ordinary inter frames
/// remain valid decoder input.
#[derive(Debug, Default)]
struct DecoderGate {
    initialized: bool,
}

impl DecoderGate {
    fn accept(&mut self, job: &DecodedFrameJob) -> Result<(), DecoderError> {
        validate_frame(job)?;
        if !self.initialized {
            if !job.keyframe || !contains_h264_sequence_header(&job.access_unit) {
                return Err(DecoderError::MissingSequenceHeader);
            }
            self.initialized = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(keyframe: bool, access_unit: &[u8]) -> DecodedFrameJob {
        DecodedFrameJob {
            frame_id: 1,
            timestamp_us: 1,
            keyframe,
            access_unit: access_unit.to_vec(),
        }
    }

    #[test]
    fn gate_requires_a_sequence_header_only_until_decoder_initialization() {
        let mut gate = DecoderGate::default();

        assert_eq!(
            gate.accept(&job(false, b"inter")),
            Err(DecoderError::MissingSequenceHeader)
        );
        assert!(gate.accept(&job(true, b"\0\0\0\x01\x67\x64")).is_ok());
        assert!(gate.accept(&job(false, b"ordinary-inter-frame")).is_ok());
    }

    #[test]
    fn surface_validation_rejects_dimensions_beyond_the_native_cap() {
        let surface = DecodedSurface {
            frame_id: 1,
            timestamp_us: 1,
            keyframe: true,
            width: MAX_SURFACE_WIDTH + 2,
            height: 2,
            format: SurfaceFormat::Nv12,
            bytes: vec![0; 3],
        };

        assert_eq!(surface.validate(), Err(DecoderError::InvalidDimensions));
    }

    #[test]
    fn repacks_padded_nv12_rows_without_copying_padding() {
        let padded = [
            1, 2, 3, 4, 90, 90, 5, 6, 7, 8, 91, 91, 9, 10, 11, 12, 92, 92,
        ];

        assert_eq!(
            repack_nv12(&padded, 6, 4, 2).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
    }
}

fn contains_h264_sequence_header(access_unit: &[u8]) -> bool {
    access_unit.windows(5).any(|nal| nal == [0, 0, 0, 1, 0x67])
        || access_unit.windows(4).any(|nal| nal == [0, 0, 1, 0x67])
}

#[cfg(not(windows))]
#[derive(Default)]
pub(crate) struct PlatformFrameDecoder {
    gate: DecoderGate,
}

#[cfg(not(windows))]
impl FrameDecoder for PlatformFrameDecoder {
    fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError> {
        self.gate.accept(&job)?;
        Err(DecoderError::BackendUnavailable)
    }
}

fn repack_nv12(
    source: &[u8],
    stride: usize,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, DecoderError> {
    let pixels = validated_pixel_count(width, height)?;
    let width = usize::try_from(width).map_err(|_| DecoderError::InvalidDimensions)?;
    let height = usize::try_from(height).map_err(|_| DecoderError::InvalidDimensions)?;
    if stride < width {
        return Err(DecoderError::InvalidSurface);
    }
    let rows = height
        .checked_add(height / 2)
        .ok_or(DecoderError::InvalidDimensions)?;
    let required = stride
        .checked_mul(rows)
        .ok_or(DecoderError::InvalidDimensions)?;
    if source.len() < required {
        return Err(DecoderError::InvalidSurface);
    }
    let output_len = pixels
        .checked_add(pixels / 2)
        .filter(|length| *length <= MAX_SURFACE_BYTES)
        .ok_or(DecoderError::InvalidDimensions)?;
    let mut packed = vec![0_u8; output_len];
    for row in 0..height {
        let src_start = row * stride;
        let dst_start = row * width;
        packed[dst_start..dst_start + width].copy_from_slice(&source[src_start..src_start + width]);
    }
    let source_uv = height * stride;
    let destination_uv = pixels;
    for row in 0..height / 2 {
        let src_start = source_uv + row * stride;
        let dst_start = destination_uv + row * width;
        packed[dst_start..dst_start + width].copy_from_slice(&source[src_start..src_start + width]);
    }
    Ok(packed)
}

#[cfg(windows)]
pub(crate) mod native {
    use super::{
        repack_nv12, validated_nv12_len, validated_pixel_count, DecodedFrameJob, DecodedSurface,
        DecoderError, DecoderGate, FrameDecoder, SurfaceFormat,
    };
    use crate::native_worker::WorkerLifecycle;
    use std::collections::VecDeque;
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};
    use std::{mem::ManuallyDrop, ptr};
    use windows::core::Interface;
    use windows::Win32::Media::MediaFoundation::{
        IMF2DBuffer, IMF2DBuffer2, IMFMediaBuffer, IMFShutdown, IMFTransform,
        MF2DBuffer_LockFlags_Read, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
        MFMediaType_Video, MFShutdown, MFStartup, MFTEnumEx, MFVideoFormat_H264,
        MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
        MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_SORTANDFILTER,
        MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
        MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
        MFT_OUTPUT_DATA_BUFFER_INCOMPLETE, MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE,
        MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
        MFT_REGISTER_TYPE_INFO, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_FRAME_SIZE,
        MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
        MF_VERSION,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
    };

    const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
    const REQUEST_TIMEOUT: Duration = Duration::from_millis(250);
    const MAX_PENDING_DECODED_FRAMES: usize = 8;

    enum DecoderCommand {
        Decode(
            DecodedFrameJob,
            SyncSender<Result<Option<DecodedSurface>, DecoderError>>,
        ),
        Shutdown(SyncSender<()>),
    }

    /// Media Foundation H.264 decoder whose COM and MFT objects never leave
    /// the `nexus-client-decoder` worker.
    pub(super) struct MediaFoundationDecoder {
        commands: Option<SyncSender<DecoderCommand>>,
        lifecycle: Arc<WorkerLifecycle>,
        gate: DecoderGate,
    }

    impl MediaFoundationDecoder {
        pub(super) fn start(width: u32, height: u32) -> Result<Self, DecoderError> {
            validated_nv12_len(width, height)?;
            let (commands, receiver) = sync_channel(1);
            let (started_tx, started_rx) = sync_channel(1);
            let worker = thread::Builder::new()
                .name("nexus-client-decoder".to_owned())
                .spawn(move || decoder_main(width, height, receiver, started_tx))
                .map_err(|_| DecoderError::BackendUnavailable)?;
            let lifecycle = WorkerLifecycle::new(worker);
            match started_rx.recv_timeout(STARTUP_TIMEOUT) {
                Ok(Ok(())) => Ok(Self {
                    commands: Some(commands),
                    lifecycle,
                    gate: DecoderGate::default(),
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

        fn stop(&mut self) {
            if let Some(commands) = self.commands.take() {
                let (reply_tx, reply_rx) = sync_channel(1);
                let _ = commands.try_send(DecoderCommand::Shutdown(reply_tx));
                let _ = reply_rx.recv_timeout(REQUEST_TIMEOUT);
            }
            self.lifecycle.join_before(Instant::now() + REQUEST_TIMEOUT);
        }
    }

    impl FrameDecoder for MediaFoundationDecoder {
        fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError> {
            self.gate.accept(&job)?;
            let (reply_tx, reply_rx) = sync_channel(1);
            self.commands
                .as_ref()
                .ok_or(DecoderError::BackendLost)?
                .try_send(DecoderCommand::Decode(job, reply_tx))
                .map_err(|_| DecoderError::BackendLost)?;
            reply_rx
                .recv_timeout(REQUEST_TIMEOUT)
                .map_err(|_| DecoderError::BackendLost)?
        }
    }

    impl Drop for MediaFoundationDecoder {
        fn drop(&mut self) {
            self.stop();
        }
    }

    struct NativeDecoder {
        transform: IMFTransform,
        width: u32,
        height: u32,
        gate: DecoderGate,
        pending_inputs: VecDeque<PendingFrame>,
        pending_surfaces: VecDeque<DecodedSurface>,
        media_foundation_started: bool,
    }

    struct PendingFrame {
        frame_id: u32,
        timestamp_us: u64,
        timestamp_hns: i64,
        keyframe: bool,
    }

    impl NativeDecoder {
        fn start(width: u32, height: u32) -> Result<Self, DecoderError> {
            validated_nv12_len(width, height)?;
            // SAFETY: paired with MFShutdown below on this worker thread.
            unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
                .map_err(|_| DecoderError::BackendUnavailable)?;
            let transform = match select_h264_decoder() {
                Ok(transform) => transform,
                Err(error) => {
                    let _ = unsafe { MFShutdown() };
                    return Err(error);
                }
            };
            let configured: Result<(), DecoderError> = (|| {
                let input = video_type(width, height, MFVideoFormat_H264)?;
                let output = video_type(width, height, MFVideoFormat_NV12)?;
                // SAFETY: types are complete and this worker exclusively owns MFT.
                unsafe { transform.SetInputType(0, &input, 0) }
                    .map_err(|_| DecoderError::BackendUnavailable)?;
                unsafe { transform.SetOutputType(0, &output, 0) }
                    .map_err(|_| DecoderError::BackendUnavailable)?;
                unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }
                    .map_err(|_| DecoderError::BackendUnavailable)?;
                unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }
                    .map_err(|_| DecoderError::BackendUnavailable)
            })();
            if let Err(error) = configured {
                if let Ok(shutdown) = transform.cast::<IMFShutdown>() {
                    let _ = unsafe { shutdown.Shutdown() };
                }
                let _ = unsafe { MFShutdown() };
                return Err(error);
            }
            Ok(Self {
                transform,
                width,
                height,
                gate: DecoderGate::default(),
                pending_inputs: VecDeque::new(),
                pending_surfaces: VecDeque::new(),
                media_foundation_started: true,
            })
        }

        fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError> {
            self.gate.accept(&job)?;
            let sample = input_sample(&job)?;
            let timestamp_hns = timestamp_hns(job.timestamp_us)?;
            if self.pending_inputs.len() == MAX_PENDING_DECODED_FRAMES {
                return Err(DecoderError::BackendLost);
            }
            // SAFETY: this worker owns the transform; `sample` owns a complete
            // access unit and carries its source timestamp in MF time units.
            unsafe { self.transform.ProcessInput(0, &sample, 0) }
                .map_err(|_| DecoderError::BackendLost)?;
            self.pending_inputs.push_back(PendingFrame {
                frame_id: job.frame_id,
                timestamp_us: job.timestamp_us,
                timestamp_hns,
                keyframe: job.keyframe,
            });
            self.drain_outputs()?;
            Ok(self.pending_surfaces.pop_front())
        }

        fn drain_outputs(&mut self) -> Result<(), DecoderError> {
            const MAX_OUTPUTS_PER_SUBMISSION: usize = 8;
            for _ in 0..MAX_OUTPUTS_PER_SUBMISSION {
                let (sample, more_output) = self.take_one_output()?;
                let Some(sample) = sample else {
                    if more_output {
                        continue;
                    }
                    return Ok(());
                };
                let timestamp_hns =
                    unsafe { sample.GetSampleTime() }.map_err(|_| DecoderError::BackendLost)?;
                let metadata_index = self
                    .pending_inputs
                    .iter()
                    .position(|pending| pending.timestamp_hns == timestamp_hns)
                    .ok_or(DecoderError::BackendLost)?;
                let metadata = self
                    .pending_inputs
                    .remove(metadata_index)
                    .ok_or(DecoderError::BackendLost)?;
                let buffer =
                    unsafe { sample.GetBufferByIndex(0) }.map_err(|_| DecoderError::BackendLost)?;
                let (bytes, stride) = copy_buffer_with_stride(&buffer, self.width, self.height)?;
                let packed = repack_nv12(&bytes, stride, self.width, self.height)?;
                let surface = DecodedSurface {
                    frame_id: metadata.frame_id,
                    timestamp_us: metadata.timestamp_us,
                    keyframe: metadata.keyframe,
                    width: self.width,
                    height: self.height,
                    format: SurfaceFormat::Nv12,
                    bytes: packed,
                };
                surface.validate()?;
                if self.pending_surfaces.len() == MAX_PENDING_DECODED_FRAMES {
                    return Err(DecoderError::BackendLost);
                }
                self.pending_surfaces.push_back(surface);
                if !more_output {
                    return Ok(());
                }
            }
            Err(DecoderError::BackendLost)
        }

        fn take_one_output(
            &mut self,
        ) -> Result<
            (
                Option<windows::Win32::Media::MediaFoundation::IMFSample>,
                bool,
            ),
            DecoderError,
        > {
            let info = unsafe { self.transform.GetOutputStreamInfo(0) }
                .map_err(|_| DecoderError::BackendLost)?;
            let transform_provides_sample = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                    | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
                != 0;
            let sample = if transform_provides_sample {
                None
            } else {
                let expected = validated_nv12_len(self.width, self.height)?;
                let announced =
                    usize::try_from(info.cbSize).map_err(|_| DecoderError::InvalidDimensions)?;
                let capacity = expected.max(announced);
                if capacity > super::MAX_SURFACE_BYTES {
                    return Err(DecoderError::InvalidSurface);
                }
                let capacity =
                    u32::try_from(capacity).map_err(|_| DecoderError::InvalidDimensions)?;
                let sample = unsafe { MFCreateSample() }.map_err(|_| DecoderError::BackendLost)?;
                let buffer = unsafe { MFCreateMemoryBuffer(capacity) }
                    .map_err(|_| DecoderError::BackendLost)?;
                unsafe { sample.AddBuffer(&buffer) }.map_err(|_| DecoderError::BackendLost)?;
                Some(sample)
            };
            let mut output = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0_u32;
            // SAFETY: the MFT and the optional output sample belong to this
            // worker; both ManuallyDrop fields are reclaimed exactly once.
            let result = unsafe {
                self.transform
                    .ProcessOutput(0, std::slice::from_mut(&mut output), &mut status)
            };
            let sample = unsafe { ManuallyDrop::take(&mut output.pSample) };
            let events = unsafe { ManuallyDrop::take(&mut output.pEvents) };
            drop(events);
            match result {
                Ok(()) if output.dwStatus & MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE.0 as u32 != 0 => Ok((
                    None,
                    output.dwStatus & MFT_OUTPUT_DATA_BUFFER_INCOMPLETE.0 as u32 != 0,
                )),
                Ok(()) => Ok((
                    sample,
                    output.dwStatus & MFT_OUTPUT_DATA_BUFFER_INCOMPLETE.0 as u32 != 0,
                )),
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok((None, false)),
                Err(_) => Err(DecoderError::BackendLost),
            }
        }

        fn shutdown(&mut self) {
            let _ = unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) };
            if let Ok(shutdown) = self.transform.cast::<IMFShutdown>() {
                let _ = unsafe { shutdown.Shutdown() };
            }
            if self.media_foundation_started {
                let _ = unsafe { MFShutdown() };
                self.media_foundation_started = false;
            }
        }
    }

    impl Drop for NativeDecoder {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    fn decoder_main(
        width: u32,
        height: u32,
        receiver: Receiver<DecoderCommand>,
        started: SyncSender<Result<(), DecoderError>>,
    ) {
        // SAFETY: COM is initialized and uninitialized on this named worker.
        if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
            let _ = started.send(Err(DecoderError::BackendUnavailable));
            return;
        }
        let mut decoder = match NativeDecoder::start(width, height) {
            Ok(decoder) => decoder,
            Err(error) => {
                let _ = started.send(Err(error));
                unsafe { CoUninitialize() };
                return;
            }
        };
        let _ = started.send(Ok(()));
        while let Ok(command) = receiver.recv() {
            match command {
                DecoderCommand::Decode(job, reply) => {
                    let _ = reply.send(decoder.decode(job));
                }
                DecoderCommand::Shutdown(reply) => {
                    decoder.shutdown();
                    let _ = reply.send(());
                    break;
                }
            }
        }
        drop(decoder);
        unsafe { CoUninitialize() };
    }

    fn select_h264_decoder() -> Result<IMFTransform, DecoderError> {
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
        let mut activations = std::ptr::null_mut();
        let mut count = 0;
        // SAFETY: output pointers are valid and the returned CoTaskMem block is
        // released after all activation references have been consumed.
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_DECODER,
                flags,
                Some(&input),
                Some(&output),
                &mut activations,
                &mut count,
            )
        }
        .map_err(|_| DecoderError::BackendUnavailable)?;
        if activations.is_null() || count == 0 {
            if !activations.is_null() {
                unsafe { CoTaskMemFree(Some(activations.cast())) };
            }
            return Err(DecoderError::BackendUnavailable);
        }
        let selected = unsafe {
            let items = std::slice::from_raw_parts_mut(activations, count as usize);
            let mut selected = None;
            for activation in items {
                let activation = activation.take();
                if selected.is_none() {
                    selected = activation;
                }
                // Every unselected activation is dropped before its backing
                // CoTaskMem allocation is released below.
            }
            CoTaskMemFree(Some(activations.cast()));
            selected
        }
        .ok_or(DecoderError::BackendUnavailable)?;
        unsafe { selected.ActivateObject::<IMFTransform>() }
            .map_err(|_| DecoderError::BackendUnavailable)
    }

    fn video_type(
        width: u32,
        height: u32,
        subtype: windows::core::GUID,
    ) -> Result<windows::Win32::Media::MediaFoundation::IMFMediaType, DecoderError> {
        let media_type =
            unsafe { MFCreateMediaType() }.map_err(|_| DecoderError::BackendUnavailable)?;
        let size = (u64::from(width) << 32) | u64::from(height);
        unsafe { media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }
            .map_err(|_| DecoderError::BackendUnavailable)?;
        unsafe { media_type.SetGUID(&MF_MT_SUBTYPE, &subtype) }
            .map_err(|_| DecoderError::BackendUnavailable)?;
        unsafe { media_type.SetUINT64(&MF_MT_FRAME_SIZE, size) }
            .map_err(|_| DecoderError::BackendUnavailable)?;
        unsafe { media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1_u64 << 32) }
            .map_err(|_| DecoderError::BackendUnavailable)?;
        unsafe {
            media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
        }
        .map_err(|_| DecoderError::BackendUnavailable)?;
        Ok(media_type)
    }

    fn input_sample(
        job: &DecodedFrameJob,
    ) -> Result<windows::Win32::Media::MediaFoundation::IMFSample, DecoderError> {
        let length = u32::try_from(job.access_unit.len()).map_err(|_| {
            DecoderError::Frame(super::RenderQueueError::AccessUnitTooLarge {
                actual: job.access_unit.len(),
                limit: super::super::renderer::MAX_RENDER_ACCESS_UNIT_SIZE,
            })
        })?;
        let buffer =
            unsafe { MFCreateMemoryBuffer(length) }.map_err(|_| DecoderError::BackendLost)?;
        let mut destination = ptr::null_mut();
        unsafe { buffer.Lock(&mut destination, None, None) }
            .map_err(|_| DecoderError::BackendLost)?;
        if destination.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err(DecoderError::BackendLost);
        }
        // SAFETY: the buffer is allocated for the exact source length and is
        // locked until this copy completes on the owning decoder thread.
        unsafe {
            ptr::copy_nonoverlapping(job.access_unit.as_ptr(), destination, job.access_unit.len())
        };
        unsafe { buffer.Unlock() }.map_err(|_| DecoderError::BackendLost)?;
        unsafe { buffer.SetCurrentLength(length) }.map_err(|_| DecoderError::BackendLost)?;
        let sample = unsafe { MFCreateSample() }.map_err(|_| DecoderError::BackendLost)?;
        unsafe { sample.AddBuffer(&buffer) }.map_err(|_| DecoderError::BackendLost)?;
        let timestamp = timestamp_hns(job.timestamp_us)?;
        unsafe { sample.SetSampleTime(timestamp) }.map_err(|_| DecoderError::BackendLost)?;
        Ok(sample)
    }

    fn timestamp_hns(timestamp_us: u64) -> Result<i64, DecoderError> {
        timestamp_us
            .checked_mul(10)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(DecoderError::BackendLost)
    }

    fn copy_buffer_with_stride(
        buffer: &IMFMediaBuffer,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, usize), DecoderError> {
        let width_usize = usize::try_from(width).map_err(|_| DecoderError::InvalidDimensions)?;
        let max_length = usize::try_from(
            unsafe { buffer.GetMaxLength() }.map_err(|_| DecoderError::BackendLost)?,
        )
        .map_err(|_| DecoderError::InvalidSurface)?;
        if max_length > super::MAX_SURFACE_BYTES {
            return Err(DecoderError::InvalidSurface);
        }
        let actual_length = usize::try_from(
            unsafe { buffer.GetCurrentLength() }.map_err(|_| DecoderError::BackendLost)?,
        )
        .map_err(|_| DecoderError::InvalidSurface)?;
        if actual_length > max_length {
            return Err(DecoderError::InvalidSurface);
        }
        let rows = usize::try_from(height)
            .ok()
            .and_then(|height| height.checked_add(height / 2))
            .ok_or(DecoderError::InvalidDimensions)?;
        let expected = validated_nv12_len(width, height)?;
        if actual_length < expected {
            return Err(DecoderError::InvalidSurface);
        }
        if let Ok(buffer_2d) = buffer.cast::<IMF2DBuffer2>() {
            let mut source = ptr::null_mut();
            let mut buffer_start = ptr::null_mut();
            let mut signed_pitch = 0_i32;
            let mut buffer_length = 0_u32;
            unsafe {
                buffer_2d.Lock2DSize(
                    MF2DBuffer_LockFlags_Read,
                    &mut source,
                    &mut signed_pitch,
                    &mut buffer_start,
                    &mut buffer_length,
                )
            }
            .map_err(|_| DecoderError::BackendLost)?;
            let result = (|| {
                if source.is_null() || buffer_start.is_null() || signed_pitch < 0 {
                    return Err(DecoderError::InvalidSurface);
                }
                let pitch =
                    usize::try_from(signed_pitch).map_err(|_| DecoderError::InvalidSurface)?;
                if pitch < width_usize {
                    return Err(DecoderError::InvalidSurface);
                }
                let allocation_length =
                    usize::try_from(buffer_length).map_err(|_| DecoderError::InvalidSurface)?;
                if allocation_length > max_length || allocation_length > super::MAX_SURFACE_BYTES {
                    return Err(DecoderError::InvalidSurface);
                }
                let source_address = source as usize;
                let allocation_address = buffer_start as usize;
                let offset = source_address
                    .checked_sub(allocation_address)
                    .ok_or(DecoderError::InvalidSurface)?;
                let length = pitch
                    .checked_mul(rows)
                    .ok_or(DecoderError::InvalidDimensions)?;
                if offset
                    .checked_add(length)
                    .ok_or(DecoderError::InvalidDimensions)?
                    > allocation_length
                {
                    return Err(DecoderError::InvalidSurface);
                }
                // SAFETY: Lock2DSize supplied the mapped allocation base and
                // byte length. The checked offset and pitch*rows prove that
                // every byte is inside that allocation until Unlock2D.
                Ok((
                    unsafe { std::slice::from_raw_parts(source, length) }.to_vec(),
                    pitch,
                ))
            })();
            let unlock = unsafe { buffer_2d.Unlock2D() };
            unlock.map_err(|_| DecoderError::BackendLost)?;
            return result;
        }
        if let Ok(buffer_2d) = buffer.cast::<IMF2DBuffer>() {
            let contiguous_length = usize::try_from(
                unsafe { buffer_2d.GetContiguousLength() }
                    .map_err(|_| DecoderError::BackendLost)?,
            )
            .map_err(|_| DecoderError::InvalidSurface)?;
            if contiguous_length != expected || contiguous_length > max_length {
                return Err(DecoderError::InvalidSurface);
            }
            let mut packed = vec![0_u8; contiguous_length];
            // ContiguousCopyTo writes only into the caller-owned bounded
            // destination, so no mapped pointer extent is inferred here.
            unsafe { buffer_2d.ContiguousCopyTo(&mut packed) }
                .map_err(|_| DecoderError::BackendLost)?;
            return Ok((packed, width_usize));
        }
        let mut source = ptr::null_mut();
        let mut locked_max = 0_u32;
        let mut locked_current = 0_u32;
        unsafe {
            buffer.Lock(
                &mut source,
                Some(&mut locked_max),
                Some(&mut locked_current),
            )
        }
        .map_err(|_| DecoderError::BackendLost)?;
        if source.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err(DecoderError::BackendLost);
        }
        let locked_max = usize::try_from(locked_max).map_err(|_| DecoderError::InvalidSurface);
        let locked_current =
            usize::try_from(locked_current).map_err(|_| DecoderError::InvalidSurface);
        let bytes_result = match (locked_max, locked_current) {
            (Ok(locked_max), Ok(locked_current))
                if locked_max <= max_length
                    && locked_max <= super::MAX_SURFACE_BYTES
                    && locked_current >= expected
                    && locked_current <= locked_max =>
            {
                // SAFETY: Lock reported a bounded allocation and current
                // length covering exactly the bytes read below.
                Ok(unsafe { std::slice::from_raw_parts(source, expected) }.to_vec())
            }
            _ => Err(DecoderError::InvalidSurface),
        };
        // Unlock even when the reported extent is invalid; leaving a native
        // Media Foundation buffer locked can deadlock the decoder on teardown.
        unsafe { buffer.Unlock() }.map_err(|_| DecoderError::BackendLost)?;
        let bytes = bytes_result?;
        Ok((bytes, width_usize))
    }
}

#[cfg(windows)]
pub(crate) fn native_decoder_smoke(job: DecodedFrameJob) -> Result<DecodedSurface, DecoderError> {
    let mut decoder = native::MediaFoundationDecoder::start(1_280, 720)?;
    decoder.decode(job)?.ok_or(DecoderError::BackendUnavailable)
}
