#![cfg(feature = "test-support")]

use bytes::Bytes;
use nexus_capture::{CapturedFrame, PixelFormat};
use nexus_codec::{CodecError, CodecKind, EncodedFrame, EncoderConfig, VideoEncoder};
use platform_windows::{EncoderTransform, WindowsH264Encoder};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TransformCall {
    Configure(EncoderConfig),
    Encode { frame_id: u64, force_keyframe: bool },
    Drain,
    Shutdown,
}

struct RecordingTransform {
    calls: Arc<Mutex<Vec<TransformCall>>>,
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
    ) -> Result<EncodedFrame, CodecError> {
        self.calls.lock().unwrap().push(TransformCall::Encode {
            frame_id: frame.frame_id,
            force_keyframe,
        });
        Ok(EncodedFrame {
            frame_id: frame.frame_id,
            timestamp_us: frame.timestamp_us,
            keyframe: force_keyframe,
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
        })
    }

    fn drain(&mut self) -> Result<(), CodecError> {
        self.calls.lock().unwrap().push(TransformCall::Drain);
        Ok(())
    }

    fn shutdown(&mut self) {
        self.calls.lock().unwrap().push(TransformCall::Shutdown);
    }
}

fn encoder() -> (WindowsH264Encoder, Arc<Mutex<Vec<TransformCall>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let transform = RecordingTransform {
        calls: Arc::clone(&calls),
        fail_configure_call: None,
        configure_count: 0,
    };
    (WindowsH264Encoder::with_transform(transform), calls)
}

fn encoder_failing_second_configure() -> (WindowsH264Encoder, Arc<Mutex<Vec<TransformCall>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let transform = RecordingTransform {
        calls: Arc::clone(&calls),
        fail_configure_call: Some(2),
        configure_count: 0,
    };
    (WindowsH264Encoder::with_transform(transform), calls)
}

fn config(width: u32, height: u32, bitrate_bps: u32) -> EncoderConfig {
    EncoderConfig {
        codec: CodecKind::H264,
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

#[test]
fn configuration_invalid_values_never_reach_the_transform() {
    let (mut encoder, calls) = encoder();

    assert_eq!(
        encoder.configure(config(0, 64, 1_000_000)),
        Err(CodecError::InvalidDimensions)
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn configuration_valid_h264_values_reach_the_transform_once() {
    let (mut encoder, calls) = encoder();
    let expected = config(64, 64, 1_000_000);

    encoder.configure(expected).unwrap();

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[TransformCall::Configure(expected)]
    );
}

#[test]
fn configuration_is_required_before_encoding() {
    let (mut encoder, calls) = encoder();

    assert_eq!(
        encoder.encode(frame(1, 64, 64)),
        Err(CodecError::NotConfigured)
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn configuration_dimensions_must_match_each_frame() {
    let (mut encoder, calls) = encoder();
    encoder.configure(config(64, 64, 1_000_000)).unwrap();

    assert_eq!(
        encoder.encode(frame(1, 32, 64)),
        Err(CodecError::FrameDimensionsMismatch)
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[TransformCall::Configure(config(64, 64, 1_000_000))]
    );
}

#[test]
fn configuration_malformed_bgra_frame_never_reaches_the_transform() {
    let (mut encoder, calls) = encoder();
    encoder.configure(config(64, 64, 1_000_000)).unwrap();
    let malformed = CapturedFrame {
        data: Bytes::new(),
        ..frame(1, 64, 64)
    };

    assert_eq!(encoder.encode(malformed), Err(CodecError::InvalidFrame));
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[TransformCall::Configure(config(64, 64, 1_000_000))]
    );
}

#[test]
fn keyframe_request_forces_exactly_the_next_output() {
    let (mut encoder, calls) = encoder();
    encoder.configure(config(64, 64, 1_000_000)).unwrap();

    assert!(encoder.encode(frame(1, 64, 64)).unwrap().keyframe);
    assert!(!encoder.encode(frame(2, 64, 64)).unwrap().keyframe);
    encoder.request_keyframe().unwrap();
    assert!(encoder.encode(frame(3, 64, 64)).unwrap().keyframe);
    assert!(!encoder.encode(frame(4, 64, 64)).unwrap().keyframe);

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
            TransformCall::Encode {
                frame_id: 3,
                force_keyframe: true,
            },
            TransformCall::Encode {
                frame_id: 4,
                force_keyframe: false,
            },
        ]
    );
}

#[test]
fn keyframe_reconfiguration_drains_before_configuring_the_transform() {
    let (mut encoder, calls) = encoder();
    encoder.configure(config(64, 64, 1_000_000)).unwrap();

    encoder.reconfigure(config(128, 64, 2_000_000)).unwrap();

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
fn keyframe_dimension_change_forces_the_next_output() {
    let (mut encoder, _calls) = encoder();
    encoder.configure(config(64, 64, 1_000_000)).unwrap();
    encoder.encode(frame(1, 64, 64)).unwrap();
    assert!(!encoder.encode(frame(2, 64, 64)).unwrap().keyframe);

    encoder.reconfigure(config(128, 64, 2_000_000)).unwrap();

    assert!(encoder.encode(frame(3, 128, 64)).unwrap().keyframe);
    assert!(!encoder.encode(frame(4, 128, 64)).unwrap().keyframe);
}

#[test]
fn keyframe_bitrate_only_reconfiguration_preserves_normal_cadence() {
    let (mut encoder, _calls) = encoder();
    encoder.configure(config(64, 64, 1_000_000)).unwrap();
    encoder.encode(frame(1, 64, 64)).unwrap();
    assert!(!encoder.encode(frame(2, 64, 64)).unwrap().keyframe);

    encoder.reconfigure(config(64, 64, 2_000_000)).unwrap();

    assert!(!encoder.encode(frame(3, 64, 64)).unwrap().keyframe);
    encoder.request_keyframe().unwrap();
    assert!(encoder.encode(frame(4, 64, 64)).unwrap().keyframe);
}

#[test]
fn keyframe_failed_reconfiguration_leaves_the_encoder_unavailable() {
    let (mut encoder, calls) = encoder_failing_second_configure();
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
fn lifecycle_drop_drains_before_shutting_down_the_transform() {
    let (mut encoder, calls) = encoder();
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

#[cfg(not(windows))]
#[test]
fn lifecycle_native_constructor_fails_closed_off_windows() {
    assert_eq!(
        WindowsH264Encoder::new().unwrap_err(),
        CodecError::BackendUnavailable
    );
}
