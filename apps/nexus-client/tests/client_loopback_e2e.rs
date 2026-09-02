use nexus_client::{ClientRuntime, WindowEvent};
use nexus_crypto::NonceSequence;
use nexus_input::InputEvent;
use nexus_protocol::{video_packet, MonitorInfo, MouseMove, VideoPacketHeader};
use nexus_transport::{
    control::decode_framed_control,
    quic::{make_client_endpoint, make_server_endpoint},
    video::{encode_video_datagram, packetize_video_frame, seal_video_frame},
};
use std::time::Duration;

const KEY: [u8; 32] = [0x3C; 32];
const NONCE_DOMAIN: u32 = 0x1020_3040;

fn monitor() -> MonitorInfo {
    MonitorInfo {
        id: 1,
        origin_x: 0,
        origin_y: 0,
        width: 1_280,
        height: 720,
        scale: 1.0,
    }
}

fn video_datagram() -> Vec<u8> {
    let mut sequence = NonceSequence::new(NONCE_DOMAIN);
    let mut header = VideoPacketHeader {
        version: video_packet::CURRENT_VERSION,
        flags: video_packet::flags::KEYFRAME,
        stream_id: 1,
        frame_id: 9,
        packet_id: 0,
        packet_count: 0,
        timestamp_us: 123_456,
        nonce_sequence: 0,
        payload_len: 0,
    };
    let sealed = seal_video_frame(&KEY, &mut sequence, &header, 1, b"loopback-h264").unwrap();
    header.nonce_sequence = u64::from_be_bytes(sealed.nonce[4..].try_into().unwrap());
    let (header, payload) = packetize_video_frame(&header, &sealed.ciphertext, 1_200)
        .unwrap()
        .remove(0);
    encode_video_datagram(&header, &payload).unwrap()
}

#[tokio::test]
async fn loopback_authenticates_video_and_emits_one_semantic_input() {
    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let cert = server.cert_der.clone();
    let server_task = tokio::spawn(async move {
        let incoming = server.endpoint.accept().await.unwrap();
        let connection = incoming.await.unwrap();
        connection.send_datagram(video_datagram().into()).unwrap();
        let control = tokio::time::timeout(Duration::from_secs(2), connection.read_datagram())
            .await
            .unwrap()
            .unwrap()
            .to_vec();
        let decoded: MouseMove = decode_framed_control(&control).unwrap();
        assert_eq!((decoded.x, decoded.y), (80, 90));
    });

    let client_endpoint = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert).unwrap();
    let connection = client_endpoint
        .connect(server_addr, "localhost")
        .unwrap()
        .await
        .unwrap();
    let mut runtime = ClientRuntime::connect(connection, KEY, NONCE_DOMAIN, monitor()).unwrap();
    runtime
        .handle_window_event(WindowEvent::Focused(true))
        .unwrap();
    runtime
        .handle_window_event(WindowEvent::Input(InputEvent::MouseMove { x: 80, y: 90 }))
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), runtime.run())
        .await
        .unwrap()
        .unwrap();
    let frame = runtime
        .drain_latest_frame()
        .expect("one authenticated frame");
    assert_eq!(frame.frame_id, 9);
    assert_eq!(frame.timestamp_us, 123_456);
    assert!(frame.keyframe);
    assert_eq!(frame.access_unit, b"loopback-h264");
    assert_eq!(runtime.sent_input_count(), 1);
    server_task.await.unwrap();
}
