//! Phase 0 End-to-End Live Pipeline Integration Test (Spec Section 20, 21, 48, 52; ADR-003, ADR-004, ADR-022, ADR-025).
//!
//! Proves the entire Phase 0 Exit Condition:
//! Host: SyntheticCaptureSource -> LatestFrameQueue -> SoftwareFallbackEncoder -> seal_video_frame -> packetize_video_frame -> QUIC Datagrams
//! Client: QUIC Datagrams -> decode_video_datagram -> VideoFrameReassembler -> open_video_frame -> verified exact match!
//! Also proves reliable QUIC bidirectional stream control message exchange (SessionHello and MouseMove).

use std::sync::Arc;
use std::time::Duration;

use nexus_capture::{CaptureSource, LatestFrameQueue, SyntheticCaptureSource};
use nexus_codec::{CodecKind, EncoderConfig, SoftwareFallbackEncoder, VideoEncoder};
use nexus_crypto::{AeadError, EncryptedFrame, NonceSequence};
use nexus_protocol::{video_packet, MouseMove, SessionHello, VideoPacketHeader};
use nexus_transport::control::{decode_framed_control, encode_framed_control};
use nexus_transport::quic::{make_client_endpoint, make_server_endpoint};
use nexus_transport::video::{
    decode_video_datagram, encode_video_datagram, open_video_frame, packetize_video_frame,
    seal_video_frame, VideoFrameReassembler,
};

const STEP_TIMEOUT: Duration = Duration::from_secs(5);
/// Bounded chunk size satisfying <= 1200 bytes and fitting comfortably within QUIC datagram MTU (1162).
const MAX_DATAGRAM_CHUNK_SIZE: usize = 1000;

#[tokio::test]
async fn phase0_full_e2e_pipeline_test() {
    tracing_subscriber::fmt::try_init().ok();

    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let cert_der = server.cert_der.clone();

    let session_root_key: [u8; 32] = [0xA5; 32];
    let codec_config_id = 1u32;
    let video_channel_domain = 1u32;

    // Server (Host) Task
    let server_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(STEP_TIMEOUT, server.endpoint.accept())
            .await
            .expect("timed out waiting for incoming connection")
            .expect("no incoming connection");
        let host_connection = tokio::time::timeout(STEP_TIMEOUT, incoming)
            .await
            .expect("timed out waiting for connection handshake")
            .expect("handshake failed");

        // 1. Receive reliable control stream messages: SessionHello
        let (_send, mut recv) = tokio::time::timeout(STEP_TIMEOUT, host_connection.accept_bi())
            .await
            .expect("timed out waiting for SessionHello stream")
            .expect("no stream");
        let hello_bytes = tokio::time::timeout(STEP_TIMEOUT, recv.read_to_end(4096))
            .await
            .expect("timed out reading SessionHello")
            .expect("failed reading SessionHello stream");
        let received_hello: SessionHello =
            decode_framed_control(&hello_bytes).expect("failed to decode SessionHello");
        received_hello
            .validate()
            .expect("SessionHello failed validation");

        // 2. Receive reliable control stream messages: MouseMove
        let (_send, mut recv) = tokio::time::timeout(STEP_TIMEOUT, host_connection.accept_bi())
            .await
            .expect("timed out waiting for MouseMove stream")
            .expect("no stream");
        let input_bytes = tokio::time::timeout(STEP_TIMEOUT, recv.read_to_end(1024))
            .await
            .expect("timed out reading MouseMove")
            .expect("failed reading MouseMove stream");
        let received_mouse_move: MouseMove =
            decode_framed_control(&input_bytes).expect("failed to decode MouseMove");

        // 3. Synthetic Desktop Capture (640x360 @ 30fps)
        let width = 640;
        let height = 360;
        let fps = 30;
        let mut capture_source = SyntheticCaptureSource::new(width, height, fps);
        let captured_frame = capture_source.next_frame().expect("capture next frame");
        assert_eq!(captured_frame.frame_id, 1);
        assert_eq!(captured_frame.width, width);
        assert_eq!(captured_frame.height, height);

        // 4. LatestFrameQueue buffering
        let frame_queue = Arc::new(LatestFrameQueue::new());
        frame_queue.replace(captured_frame);
        let raw_frame = frame_queue.take().expect("frame available in queue");

        // 5. Software Fallback Video Encoder
        let mut encoder = SoftwareFallbackEncoder::new();
        let encoder_config = EncoderConfig {
            codec: CodecKind::H264,
            width,
            height,
            max_fps: fps,
            bitrate_bps: 2_000_000,
        };
        encoder
            .configure(encoder_config)
            .expect("configure encoder");
        let encoded_frame = encoder.encode(raw_frame).expect("encode frame");
        assert!(encoded_frame.keyframe);
        assert!(!encoded_frame.data.is_empty());

        // 6. AEAD Encryption (ADR-025)
        let mut host_nonce_seq = NonceSequence::new(video_channel_domain);
        let base_header = VideoPacketHeader {
            version: video_packet::CURRENT_VERSION,
            flags: if encoded_frame.keyframe {
                video_packet::flags::KEYFRAME
            } else {
                0
            },
            stream_id: 1,
            frame_id: encoded_frame.frame_id as u32,
            packet_id: 0,
            packet_count: 0,
            timestamp_us: encoded_frame.timestamp_us,
            payload_len: 0,
        };

        let encrypted_frame = seal_video_frame(
            &session_root_key,
            &mut host_nonce_seq,
            &base_header,
            codec_config_id,
            &encoded_frame.data,
        )
        .expect("seal video frame");

        // 7. Packetization into bounded MTU chunks (<= 1200 bytes)
        let fragments = packetize_video_frame(
            &base_header,
            &encrypted_frame.ciphertext,
            MAX_DATAGRAM_CHUNK_SIZE,
        )
        .expect("packetize video frame");
        assert!(
            fragments.len() > 1,
            "920KB frame must produce multiple fragments"
        );

        // 8. Transmit fragments via QUIC Datagrams
        for (header, payload) in fragments {
            let datagram_bytes =
                encode_video_datagram(&header, &payload).expect("encode video datagram");
            host_connection
                .send_datagram(datagram_bytes.into())
                .expect("send video datagram");
            tokio::task::yield_now().await;
        }

        // Wait for client completion signal stream so connection remains alive until client has read all datagrams
        let (_send, mut ack_recv) = tokio::time::timeout(STEP_TIMEOUT, host_connection.accept_bi())
            .await
            .expect("timed out waiting for client ack stream")
            .expect("no ack stream");
        let mut ack_buf = [0u8; 4];
        let _ = ack_recv.read_exact(&mut ack_buf).await;

        (
            received_hello,
            received_mouse_move,
            encoded_frame.data.to_vec(),
        )
    });

    // Client Task
    let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let client_connection = client
        .connect(server_addr, "localhost")
        .expect("connect() setup failed")
        .await
        .expect("handshake failed");

    // 1. Send SessionHello over reliable stream
    let sent_hello = SessionHello {
        protocol_version: 1,
        session_id: "phase0-e2e-session-001".to_string(),
        device_id: "phase0-client-device-001".to_string(),
        capability: vec![0xCA, 0xFE, 0xBA, 0xBE],
        ephemeral_public_key: vec![0x01; 32],
    };
    sent_hello.validate().expect("valid SessionHello");
    let hello_buf = encode_framed_control(&sent_hello).expect("encode SessionHello");
    let (mut send_stream, _recv) = client_connection
        .open_bi()
        .await
        .expect("open stream for SessionHello");
    send_stream
        .write_all(&hello_buf)
        .await
        .expect("write SessionHello");
    send_stream.finish().expect("finish SessionHello stream");

    // 2. Send MouseMove over reliable stream
    let sent_mouse_move = MouseMove { x: 480, y: 270 };
    let input_buf = encode_framed_control(&sent_mouse_move).expect("encode MouseMove");
    let (mut send_stream, _recv) = client_connection
        .open_bi()
        .await
        .expect("open stream for MouseMove");
    send_stream
        .write_all(&input_buf)
        .await
        .expect("write MouseMove");
    send_stream.finish().expect("finish MouseMove stream");

    // 3. Receive QUIC Datagrams -> Reassemble Frame -> Decrypt
    let mut reassembler = VideoFrameReassembler::default();
    let mut assembled_frame = None;

    while assembled_frame.is_none() {
        let datagram = tokio::time::timeout(STEP_TIMEOUT, client_connection.read_datagram())
            .await
            .expect("timed out waiting for video datagram")
            .expect("failed to read video datagram");

        let (header, payload) =
            decode_video_datagram(&datagram).expect("failed to decode video datagram");
        if let Some(assembled) = reassembler
            .process_packet(&header, payload)
            .expect("process packet failed")
        {
            assembled_frame = Some(assembled);
        }
    }

    let assembled = assembled_frame.expect("assembled frame must be present");
    assert_eq!(assembled.header.frame_id, 1);
    assert_ne!(
        assembled.header.flags & video_packet::flags::KEYFRAME,
        0,
        "must be marked as keyframe"
    );

    // 4. Decrypt Reassembled Frame via open_video_frame
    let mut client_nonce_seq = NonceSequence::new(video_channel_domain);
    let client_nonce = client_nonce_seq.next_nonce().expect("nonce allocated");
    let encrypted_payload = EncryptedFrame {
        nonce: client_nonce,
        ciphertext: assembled.payload,
    };

    let recovered_encoded_bytes = open_video_frame(
        &session_root_key,
        &assembled.header,
        codec_config_id,
        &encrypted_payload,
    )
    .expect("decryption and AEAD verification failed");

    // 5. Send acknowledgment stream to server to signal client completion
    let (mut ack_send, _recv) = client_connection
        .open_bi()
        .await
        .expect("open stream for ack");
    ack_send.write_all(b"done").await.expect("write ack");
    ack_send.finish().expect("finish ack stream");

    // Join Server Task and verify all aspects of Phase 0 Exit Condition
    let (received_hello, received_mouse_move, original_encoded_bytes) =
        server_task.await.expect("server task panicked");

    assert_eq!(received_hello, sent_hello);
    assert_eq!(received_mouse_move, sent_mouse_move);
    assert_eq!(recovered_encoded_bytes, original_encoded_bytes);
}

#[tokio::test]
async fn phase0_multi_frame_stream_pipeline() {
    tracing_subscriber::fmt::try_init().ok();

    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let cert_der = server.cert_der.clone();

    let session_root_key: [u8; 32] = [0x7B; 32];
    let codec_config_id = 2u32;
    let video_channel_domain = 1u32;
    let num_frames = 3;

    let server_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(STEP_TIMEOUT, server.endpoint.accept())
            .await
            .expect("accept connection timeout")
            .expect("no connection");
        let host_connection = tokio::time::timeout(STEP_TIMEOUT, incoming)
            .await
            .expect("handshake timeout")
            .expect("handshake failed");

        let mut capture_source = SyntheticCaptureSource::new(320, 240, 30);
        let queue = Arc::new(LatestFrameQueue::new());
        let mut encoder = SoftwareFallbackEncoder::with_keyframe_interval(2);
        encoder
            .configure(EncoderConfig {
                codec: CodecKind::H264,
                width: 320,
                height: 240,
                max_fps: 30,
                bitrate_bps: 1_000_000,
            })
            .unwrap();

        let mut host_nonce_seq = NonceSequence::new(video_channel_domain);
        let mut expected_frames_data = Vec::new();

        for _ in 0..num_frames {
            let captured = capture_source.next_frame().unwrap();
            queue.replace(captured);
            let frame = queue.take().unwrap();
            let encoded = encoder.encode(frame).unwrap();
            expected_frames_data.push(encoded.data.to_vec());

            let base_header = VideoPacketHeader {
                version: video_packet::CURRENT_VERSION,
                flags: if encoded.keyframe {
                    video_packet::flags::KEYFRAME
                } else {
                    0
                },
                stream_id: 1,
                frame_id: encoded.frame_id as u32,
                packet_id: 0,
                packet_count: 0,
                timestamp_us: encoded.timestamp_us,
                payload_len: 0,
            };

            let encrypted = seal_video_frame(
                &session_root_key,
                &mut host_nonce_seq,
                &base_header,
                codec_config_id,
                &encoded.data,
            )
            .unwrap();

            let fragments =
                packetize_video_frame(&base_header, &encrypted.ciphertext, MAX_DATAGRAM_CHUNK_SIZE)
                    .unwrap();

            for (header, payload) in fragments {
                let datagram = encode_video_datagram(&header, &payload).unwrap();
                host_connection
                    .send_datagram(datagram.into())
                    .expect("send datagram");
                tokio::task::yield_now().await;
            }
        }

        // Wait for client ack
        let (_send, mut ack_recv) = host_connection.accept_bi().await.unwrap();
        let mut ack_buf = [0u8; 4];
        let _ = ack_recv.read_exact(&mut ack_buf).await;

        expected_frames_data
    });

    let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let client_connection = client
        .connect(server_addr, "localhost")
        .unwrap()
        .await
        .unwrap();

    let mut reassembler = VideoFrameReassembler::default();
    let mut client_nonce_seq = NonceSequence::new(video_channel_domain);
    let mut received_frames_data = Vec::new();

    while received_frames_data.len() < num_frames {
        let datagram = tokio::time::timeout(STEP_TIMEOUT, client_connection.read_datagram())
            .await
            .expect("timeout reading datagram")
            .expect("read datagram failed");

        let (header, payload) = decode_video_datagram(&datagram).unwrap();
        if let Some(assembled) = reassembler.process_packet(&header, payload).unwrap() {
            let nonce = client_nonce_seq.next_nonce().unwrap();
            let encrypted = EncryptedFrame {
                nonce,
                ciphertext: assembled.payload,
            };
            let decrypted = open_video_frame(
                &session_root_key,
                &assembled.header,
                codec_config_id,
                &encrypted,
            )
            .expect("decryption failed for frame");
            received_frames_data.push(decrypted);
        }
    }

    // Signal client completion
    let (mut ack_send, _recv) = client_connection.open_bi().await.unwrap();
    ack_send.write_all(b"done").await.unwrap();
    ack_send.finish().unwrap();

    let expected_frames_data = server_task.await.unwrap();
    assert_eq!(received_frames_data.len(), num_frames);
    for i in 0..num_frames {
        assert_eq!(received_frames_data[i], expected_frames_data[i]);
    }
}

#[tokio::test]
async fn phase0_tampered_frame_detection() {
    tracing_subscriber::fmt::try_init().ok();

    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let cert_der = server.cert_der.clone();

    let session_root_key: [u8; 32] = [0x99; 32];
    let codec_config_id = 3u32;
    let video_channel_domain = 1u32;

    let server_task = tokio::spawn(async move {
        let incoming = server.endpoint.accept().await.unwrap();
        let host_connection = incoming.await.unwrap();

        let mut capture_source = SyntheticCaptureSource::new(320, 240, 30);
        let captured = capture_source.next_frame().unwrap();
        let mut encoder = SoftwareFallbackEncoder::new();
        encoder
            .configure(EncoderConfig {
                codec: CodecKind::H264,
                width: 320,
                height: 240,
                max_fps: 30,
                bitrate_bps: 1_000_000,
            })
            .unwrap();
        let encoded = encoder.encode(captured).unwrap();

        let mut host_nonce_seq = NonceSequence::new(video_channel_domain);
        let base_header = VideoPacketHeader {
            version: video_packet::CURRENT_VERSION,
            flags: video_packet::flags::KEYFRAME,
            stream_id: 1,
            frame_id: encoded.frame_id as u32,
            packet_id: 0,
            packet_count: 0,
            timestamp_us: encoded.timestamp_us,
            payload_len: 0,
        };

        let encrypted = seal_video_frame(
            &session_root_key,
            &mut host_nonce_seq,
            &base_header,
            codec_config_id,
            &encoded.data,
        )
        .unwrap();

        let mut fragments =
            packetize_video_frame(&base_header, &encrypted.ciphertext, MAX_DATAGRAM_CHUNK_SIZE)
                .unwrap();

        // Tamper with the ciphertext in the first fragment
        if let Some((_, payload)) = fragments.first_mut() {
            if !payload.is_empty() {
                payload[0] ^= 0xFF; // flip bits
            }
        }

        for (header, payload) in fragments {
            let datagram = encode_video_datagram(&header, &payload).unwrap();
            host_connection
                .send_datagram(datagram.into())
                .expect("send datagram");
            tokio::task::yield_now().await;
        }

        // Wait for client ack
        let (_send, mut ack_recv) = host_connection.accept_bi().await.unwrap();
        let mut ack_buf = [0u8; 4];
        let _ = ack_recv.read_exact(&mut ack_buf).await;
    });

    let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let client_connection = client
        .connect(server_addr, "localhost")
        .unwrap()
        .await
        .unwrap();

    let mut reassembler = VideoFrameReassembler::default();
    let mut assembled_frame = None;

    while assembled_frame.is_none() {
        let datagram = tokio::time::timeout(STEP_TIMEOUT, client_connection.read_datagram())
            .await
            .expect("timeout reading datagram")
            .expect("read datagram failed");

        let (header, payload) = decode_video_datagram(&datagram).unwrap();
        if let Some(assembled) = reassembler.process_packet(&header, payload).unwrap() {
            assembled_frame = Some(assembled);
        }
    }

    let assembled = assembled_frame.unwrap();
    let mut client_nonce_seq = NonceSequence::new(video_channel_domain);
    let nonce = client_nonce_seq.next_nonce().unwrap();
    let encrypted = EncryptedFrame {
        nonce,
        ciphertext: assembled.payload,
    };

    let result = open_video_frame(
        &session_root_key,
        &assembled.header,
        codec_config_id,
        &encrypted,
    );

    // AEAD must fail authentication
    assert_eq!(result, Err(AeadError::AuthenticationFailed));

    // Signal completion
    let (mut ack_send, _recv) = client_connection.open_bi().await.unwrap();
    ack_send.write_all(b"done").await.unwrap();
    ack_send.finish().unwrap();

    server_task.await.unwrap();
}
