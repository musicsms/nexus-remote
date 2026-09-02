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
        if self.width == 0
            || self.height == 0
            || self.width > MAX_SURFACE_WIDTH
            || self.height > MAX_SURFACE_HEIGHT
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
        {
            return Err(DecoderError::InvalidDimensions);
        }
        let pixels = usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(DecoderError::InvalidDimensions)?;
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

fn validate_decoder_job(job: &DecodedFrameJob) -> Result<(), DecoderError> {
    validate_frame(job)?;
    if !job.keyframe || !contains_h264_sequence_header(&job.access_unit) {
        return Err(DecoderError::MissingSequenceHeader);
    }
    Ok(())
}

fn contains_h264_sequence_header(access_unit: &[u8]) -> bool {
    access_unit.windows(5).any(|nal| nal == [0, 0, 0, 1, 0x67])
        || access_unit.windows(4).any(|nal| nal == [0, 0, 1, 0x67])
}

#[cfg(not(windows))]
pub(crate) struct PlatformFrameDecoder;

#[cfg(not(windows))]
impl FrameDecoder for PlatformFrameDecoder {
    fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError> {
        validate_decoder_job(&job)?;
        Err(DecoderError::BackendUnavailable)
    }
}

#[cfg(windows)]
pub(crate) mod native {
    use super::{
        contains_h264_sequence_header, validate_decoder_job, DecodedFrameJob, DecodedSurface,
        DecoderError, FrameDecoder, SurfaceFormat,
    };
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;
    use std::{mem::ManuallyDrop, ptr};
    use windows::core::Interface;
    use windows::Win32::Media::MediaFoundation::{
        IMFMediaBuffer, IMFShutdown, IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer,
        MFCreateSample, MFMediaType_Video, MFShutdown, MFStartup, MFTEnumEx, MFVideoFormat_H264,
        MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
        MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG, MFT_ENUM_FLAG_SORTANDFILTER,
        MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
        MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
        MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE, MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
        MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MF_E_TRANSFORM_NEED_MORE_INPUT,
        MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO,
        MF_MT_SUBTYPE, MF_VERSION,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED,
    };

    const STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
    const REQUEST_TIMEOUT: Duration = Duration::from_millis(250);

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
        worker: Option<JoinHandle<()>>,
    }

    impl MediaFoundationDecoder {
        pub(super) fn start(width: u32, height: u32) -> Result<Self, DecoderError> {
            let (commands, receiver) = sync_channel(1);
            let (started_tx, started_rx) = sync_channel(1);
            let worker = thread::Builder::new()
                .name("nexus-client-decoder".to_owned())
                .spawn(move || decoder_main(width, height, receiver, started_tx))
                .map_err(|_| DecoderError::BackendUnavailable)?;
            match started_rx.recv_timeout(STARTUP_TIMEOUT) {
                Ok(Ok(())) => Ok(Self {
                    commands: Some(commands),
                    worker: Some(worker),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    Err(error)
                }
                Err(_) => {
                    drop(worker);
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
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    impl FrameDecoder for MediaFoundationDecoder {
        fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError> {
            validate_decoder_job(&job)?;
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
        media_foundation_started: bool,
    }

    impl NativeDecoder {
        fn start(width: u32, height: u32) -> Result<Self, DecoderError> {
            if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
                return Err(DecoderError::InvalidDimensions);
            }
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
                media_foundation_started: true,
            })
        }

        fn decode(&mut self, job: DecodedFrameJob) -> Result<Option<DecodedSurface>, DecoderError> {
            if !job.keyframe || !contains_h264_sequence_header(&job.access_unit) {
                return Err(DecoderError::MissingSequenceHeader);
            }
            let sample = input_sample(&job)?;
            // SAFETY: this worker owns the transform; `sample` owns a complete
            // access unit and carries its source timestamp in MF time units.
            unsafe { self.transform.ProcessInput(0, &sample, 0) }
                .map_err(|_| DecoderError::BackendLost)?;
            self.take_output(&job)
        }

        fn take_output(
            &mut self,
            source: &DecodedFrameJob,
        ) -> Result<Option<DecodedSurface>, DecoderError> {
            let info = unsafe { self.transform.GetOutputStreamInfo(0) }
                .map_err(|_| DecoderError::BackendLost)?;
            let transform_provides_sample = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                    | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
                != 0;
            let sample = if transform_provides_sample {
                None
            } else {
                let expected = nv12_len(self.width, self.height)?;
                let capacity = u32::try_from(expected.max(info.cbSize as usize))
                    .map_err(|_| DecoderError::InvalidDimensions)?;
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
                Ok(()) if output.dwStatus & MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE.0 as u32 != 0 => {
                    Ok(None)
                }
                Ok(()) => {
                    let sample = sample.ok_or(DecoderError::BackendLost)?;
                    let buffer = unsafe { sample.ConvertToContiguousBuffer() }
                        .map_err(|_| DecoderError::BackendLost)?;
                    let mut bytes = copy_buffer(&buffer)?;
                    let expected = nv12_len(self.width, self.height)?;
                    if bytes.len() < expected {
                        return Err(DecoderError::InvalidSurface);
                    }
                    // ConvertToContiguousBuffer may retain row padding.  The
                    // renderer contract is tightly packed NV12, so do not let
                    // padding cross the private media boundary.
                    bytes.truncate(expected);
                    let surface = DecodedSurface {
                        frame_id: source.frame_id,
                        timestamp_us: source.timestamp_us,
                        keyframe: source.keyframe,
                        width: self.width,
                        height: self.height,
                        format: SurfaceFormat::Nv12,
                        bytes,
                    };
                    surface.validate()?;
                    Ok(Some(surface))
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => Ok(None),
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
            let selected = items.iter_mut().find_map(Option::take);
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
        let timestamp = job
            .timestamp_us
            .checked_mul(10)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(DecoderError::BackendLost)?;
        unsafe { sample.SetSampleTime(timestamp) }.map_err(|_| DecoderError::BackendLost)?;
        Ok(sample)
    }

    fn nv12_len(width: u32, height: u32) -> Result<usize, DecoderError> {
        usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_add(pixels / 2))
            .ok_or(DecoderError::InvalidDimensions)
    }

    fn copy_buffer(buffer: &IMFMediaBuffer) -> Result<Vec<u8>, DecoderError> {
        let length = unsafe { buffer.GetCurrentLength() }.map_err(|_| DecoderError::BackendLost)?;
        let mut source = ptr::null_mut();
        unsafe { buffer.Lock(&mut source, None, None) }.map_err(|_| DecoderError::BackendLost)?;
        if source.is_null() {
            let _ = unsafe { buffer.Unlock() };
            return Err(DecoderError::BackendLost);
        }
        // SAFETY: Media Foundation reports the current length for its live
        // locked buffer, and this worker copies it before Unlock.
        let bytes = unsafe { std::slice::from_raw_parts(source, length as usize) }.to_vec();
        unsafe { buffer.Unlock() }.map_err(|_| DecoderError::BackendLost)?;
        Ok(bytes)
    }
}

#[cfg(windows)]
pub(crate) fn native_decoder_smoke() -> Result<(), DecoderError> {
    let decoder = native::MediaFoundationDecoder::start(1_280, 720)?;
    drop(decoder);
    Ok(())
}
