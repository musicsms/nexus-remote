#![cfg(windows)]

use bytes::Bytes;
use nexus_capture::CapturedFrame;
use nexus_client::{interactive_windows_media_smoke, ClientReceiver};
use nexus_codec::{CodecError, CodecKind, EncoderConfig, VideoEncoder};
use nexus_crypto::{nonce_from_sequence, seal_session_payload, EncodedFrameMetadata};
use nexus_protocol::{video_packet, VideoPacketHeader};
use nexus_transport::video::{encode_video_datagram, packetize_video_frame};
use platform_windows::WindowsH264Encoder;

const KEY: [u8; 32] = [0xA5; 32];
const NONCE_DOMAIN: u32 = 0x0102_0304;

fn generated_h264_keyframe() -> Vec<u8> {
    let mut encoder = WindowsH264Encoder::new().expect("Media Foundation encoder must start");
    encoder
        .configure(EncoderConfig {
            codec: CodecKind::H264,
            width: 1_280,
            height: 720,
            max_fps: 30,
            bitrate_bps: 2_000_000,
        })
        .expect("H.264 encoder must configure");
    let pixels = vec![0_u8; 1_280 * 720 * 4];
    for frame_id in 1..=8 {
        let frame = CapturedFrame::new_bgra(
            frame_id,
            frame_id * 33_333,
            1_280,
            720,
            Bytes::from(pixels.clone()),
        )
        .expect("synthetic BGRA frame must validate");
        match encoder.encode(frame) {
            Ok(encoded) if encoded.keyframe && !encoded.data.is_empty() => {
                return encoded.data.to_vec();
            }
            Ok(_) | Err(CodecError::OutputPending) => {}
            Err(error) => panic!("H.264 frame must encode: {error}"),
        }
    }
    panic!("the encoder did not produce a keyframe after bounded pumping");
}

/// This test is intentionally kept off Linux and CI: it needs an interactive
/// Windows D3D11 + Media Foundation environment, not merely a cross-compile.
#[test]
#[ignore = "requires an interactive Windows D3D11 and Media Foundation environment"]
fn authenticated_h264_frame_decodes_and_presents_to_operator_hwnd() {
    let hwnd = std::env::var("NEXUS_CLIENT_SMOKE_HWND")
        .expect("set NEXUS_CLIENT_SMOKE_HWND to an interactive Win32 HWND")
        .parse::<isize>()
        .expect("NEXUS_CLIENT_SMOKE_HWND must be a decimal HWND value");
    let mut header = VideoPacketHeader {
        version: video_packet::CURRENT_VERSION,
        flags: video_packet::flags::KEYFRAME,
        stream_id: 1,
        frame_id: 1,
        packet_id: 0,
        packet_count: 0,
        timestamp_us: 1_000,
        nonce_sequence: 7,
        payload_len: 0,
    };
    let h264_keyframe = generated_h264_keyframe();
    let ciphertext = seal_session_payload(
        &KEY,
        &nonce_from_sequence(NONCE_DOMAIN, header.nonce_sequence),
        &EncodedFrameMetadata {
            protocol_version: header.version as u32,
            channel: header.stream_id as u32,
            frame_id: header.frame_id,
            codec_config_id: 1,
            timestamp_us: header.timestamp_us,
            keyframe: true,
        }
        .aad(),
        &h264_keyframe,
    )
    .unwrap();
    let datagrams = packetize_video_frame(&header, &ciphertext, 1_000).unwrap();
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);
    for (packet_header, payload) in datagrams {
        header = packet_header;
        receiver
            .accept_datagram(&encode_video_datagram(&header, &payload).unwrap())
            .unwrap();
    }
    let job = receiver
        .drain_latest_frame()
        .expect("authenticated receiver must emit the smoke frame");

    interactive_windows_media_smoke(hwnd, job).unwrap();
}
