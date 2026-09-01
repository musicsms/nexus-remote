#![cfg(windows)]

use bytes::Bytes;
use nexus_capture::CapturedFrame;
use nexus_codec::{CodecError, CodecKind, EncoderConfig, VideoEncoder};
use platform_windows::WindowsH264Encoder;

#[test]
#[ignore = "requires Media Foundation H.264 encoder availability"]
fn media_foundation_encodes_a_deterministic_bgra_frame() {
    let mut encoder = WindowsH264Encoder::new().expect("Media Foundation encoder must start");
    encoder
        .configure(EncoderConfig {
            codec: CodecKind::H264,
            width: 64,
            height: 64,
            max_fps: 30,
            bitrate_bps: 1_000_000,
        })
        .expect("H.264 encoder must configure");
    let pixels: Vec<u8> = (0_u8..=255).cycle().take(64 * 64 * 4).collect();
    let mut output = None;
    for frame_id in 1..=8 {
        let frame = CapturedFrame::new_bgra(
            frame_id,
            frame_id * 33_333,
            64,
            64,
            Bytes::from(pixels.clone()),
        )
        .unwrap();
        match encoder.encode(frame) {
            Ok(encoded) => {
                output = Some(encoded);
                break;
            }
            Err(CodecError::OutputPending) => {}
            Err(error) => panic!("BGRA frame must encode: {error}"),
        }
    }
    let output = output.expect("the MFT must emit an H.264 access unit after bounded pumping");

    assert!(output.keyframe, "the first output must be a keyframe");
    assert!(
        !output.data.is_empty(),
        "the encoder must emit an H.264 access unit"
    );
}
