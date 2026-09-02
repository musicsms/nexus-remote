#![cfg(windows)]

use nexus_client::{interactive_windows_media_smoke, ClientReceiver};
use nexus_crypto::{nonce_from_sequence, seal_session_payload, EncodedFrameMetadata};
use nexus_protocol::{video_packet, VideoPacketHeader};
use nexus_transport::video::{encode_video_datagram, packetize_video_frame};

const KEY: [u8; 32] = [0xA5; 32];
const NONCE_DOMAIN: u32 = 0x0102_0304;

// Annex-B SPS/PPS/IDR access unit for the manual Media Foundation smoke. The
// test still requires a real interactive HWND supplied by the operator.
const H264_KEYFRAME: &[u8] = &[
    0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x01, 0x40, 0x7b, 0x20, 0, 0, 0, 1, 0x68, 0xce, 0x06,
    0xe2, 0, 0, 0, 1, 0x65, 0x88, 0x84, 0, 0x0a, 0xf2, 0xe0,
];

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
        H264_KEYFRAME,
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
