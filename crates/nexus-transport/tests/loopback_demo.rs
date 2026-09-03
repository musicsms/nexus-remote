//! Sprint 1 demo (Spec Section 52): transmit a synthetic input message
//! over a reliable QUIC stream, and a synthetic video packet over an
//! unreliable QUIC datagram, between a loopback client and server —
//! proving Section 14's reliable-stream-for-control /
//! datagram-for-video split actually works end to end.

use std::time::Duration;

use nexus_protocol::{video_packet, MouseMove, VideoPacketHeader};
use nexus_transport::control::{decode_framed_control, encode_framed_control};
use nexus_transport::quic::{make_client_endpoint, make_server_endpoint};
use nexus_transport::video::{decode_video_datagram, encode_video_datagram};

/// Fail-fast budget for every `.await` the server task blocks on. QUIC
/// datagrams are unreliable by definition, so a regression that drops one
/// would otherwise hang this test until CI's outer job timeout instead of
/// reporting which step stalled.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn synthetic_frame_and_input_travel_over_quic_loopback() {
    tracing_subscriber::fmt::try_init().ok();

    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let cert_der = server.cert_der.clone();

    let server_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(STEP_TIMEOUT, server.endpoint.accept())
            .await
            .expect("timed out waiting for incoming connection")
            .expect("no incoming connection");
        let connection = tokio::time::timeout(STEP_TIMEOUT, incoming)
            .await
            .expect("timed out waiting for connection handshake")
            .expect("handshake failed");
        tracing::info!(remote = %connection.remote_address(), "server: connection established");

        // Reliable stream: input message (Section 14).
        let (_send, mut recv) = tokio::time::timeout(STEP_TIMEOUT, connection.accept_bi())
            .await
            .expect("timed out waiting for incoming stream")
            .expect("no incoming stream");
        let stream_bytes = tokio::time::timeout(STEP_TIMEOUT, recv.read_to_end(1024))
            .await
            .expect("timed out reading input stream")
            .expect("failed to read input stream");
        let received_mouse_move =
            decode_framed_control::<MouseMove>(&stream_bytes).expect("failed to decode MouseMove");
        tracing::info!(?received_mouse_move, "server: decoded input message");

        // Unreliable datagram: video packet (Section 14).
        let datagram = tokio::time::timeout(STEP_TIMEOUT, connection.read_datagram())
            .await
            .expect("timed out waiting for video datagram")
            .expect("failed to read video datagram");
        let (received_header, received_payload) =
            decode_video_datagram(&datagram).expect("failed to decode video packet header");
        tracing::info!(
            ?received_header,
            payload_len = received_payload.len(),
            "server: decoded video packet"
        );

        (
            received_mouse_move,
            received_header,
            received_payload.to_vec(),
        )
    });

    let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let connection = client
        .connect(server_addr, "localhost")
        .expect("connect() setup failed")
        .await
        .expect("handshake failed");
    tracing::info!(remote = %connection.remote_address(), "client: connection established");

    // Send the input message over a reliable bidirectional stream.
    let sent_mouse_move = MouseMove { x: 640, y: 360 };
    let mouse_move_buf = encode_framed_control(&sent_mouse_move).unwrap();

    let (mut send, _recv) = connection.open_bi().await.expect("failed to open stream");
    send.write_all(&mouse_move_buf)
        .await
        .expect("failed to write input stream");
    send.finish().expect("failed to finish stream");

    // Send the synthetic video packet over an unreliable datagram.
    let sent_header = VideoPacketHeader {
        version: video_packet::CURRENT_VERSION,
        flags: video_packet::flags::KEYFRAME
            | video_packet::flags::FRAME_START
            | video_packet::flags::FRAME_END,
        stream_id: 0,
        frame_id: 1,
        packet_id: 0,
        packet_count: 1,
        timestamp_us: 42,
        nonce_sequence: 0,
        payload_len: 4,
    };
    let sent_payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
    let datagram_buf = encode_video_datagram(&sent_header, &sent_payload).unwrap();
    connection
        .send_datagram(datagram_buf.into())
        .expect("failed to send video datagram");

    let (received_mouse_move, received_header, received_payload) =
        server_task.await.expect("server task panicked");

    assert_eq!(received_mouse_move, sent_mouse_move);
    assert_eq!(received_header, sent_header);
    assert_eq!(received_payload, sent_payload);
}
