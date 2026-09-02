use ed25519_dalek::{Signer, SigningKey};
use nexus_client::session::{ClientState, ClientVerification, RelayTokenMetadata, SessionPolicy};
use nexus_client::{ClientRuntime, ClientRuntimeError, WindowEvent};
use nexus_common::{MockClock, UnixTimestamp};
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

fn session(clock: MockClock) -> nexus_client::session::ClientSession {
    let capability_key = SigningKey::from_bytes(&[1; 32]);
    let relay_key = SigningKey::from_bytes(&[2; 32]);
    let mut capability = nexus_protocol::SessionCapability {
        version: 1,
        issuer: "control-plane".into(),
        session_id: "loopback-session".into(),
        subject_user_id: "user".into(),
        client_device_id: "client".into(),
        target_device_id: "host".into(),
        permissions: vec!["desktop.view".into()],
        restrictions: vec![],
        not_before: 100,
        expires_at: 200,
        nonce: vec![1],
        agent_min_protocol: 1,
        agent_max_protocol: 1,
        client_ephemeral_public_key: vec![],
        signature: vec![],
    };
    capability.signature = capability_key
        .sign(&capability.signing_bytes())
        .to_bytes()
        .to_vec();
    let mut token = RelayTokenMetadata {
        relay_id: "relay".into(),
        session_id: "loopback-session".into(),
        client_device_id: "client".into(),
        target_device_id: "host".into(),
        expires_at: UnixTimestamp::from_secs(200),
        signature: vec![],
    };
    token.signature = relay_key.sign(&token.signing_bytes()).to_bytes().to_vec();
    nexus_client::session::ClientSession::new(
        capability,
        token,
        clock,
        SessionPolicy::new(Duration::from_secs(1_800), Duration::from_secs(60)).unwrap(),
        ClientVerification {
            capability_key: capability_key.verifying_key(),
            relay_key: relay_key.verifying_key(),
            relay_id: "relay".into(),
        },
    )
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
    let mut runtime = ClientRuntime::connect(
        &client_endpoint,
        server_addr,
        "localhost",
        session(MockClock::from_secs(100)),
        KEY,
        NONCE_DOMAIN,
        monitor(),
    )
    .await
    .unwrap();
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
    assert_eq!(runtime.drain_latest_frame(), None);
    assert_eq!(runtime.sent_input_count(), 1);
    assert_eq!(runtime.session_id(), "loopback-session");
    assert_eq!(runtime.session_state(), ClientState::Reconnecting);
    runtime
        .shutdown(std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert_eq!(runtime.session_state(), ClientState::Expired);
    server_task.await.unwrap();
}

#[tokio::test]
async fn invalid_session_is_rejected_before_transport_connect() {
    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &server.cert_der).unwrap();
    let mut invalid = session(MockClock::from_secs(100));
    invalid.expire().unwrap();
    let result = ClientRuntime::connect(
        &endpoint,
        server.endpoint.local_addr().unwrap(),
        "localhost",
        invalid,
        KEY,
        NONCE_DOMAIN,
        monitor(),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("expired session must not connect"),
        Err(error) => error,
    };
    assert!(matches!(error, ClientRuntimeError::Session(_)));
}

#[tokio::test]
async fn session_expiry_clears_runtime_handoffs() {
    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let endpoint = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &server.cert_der).unwrap();
    let server_task = tokio::spawn(async move {
        let incoming = server.endpoint.accept().await.unwrap();
        let connection = incoming.await.unwrap();
        connection.closed().await;
    });
    let clock = MockClock::from_secs(100);
    let mut runtime = ClientRuntime::connect(
        &endpoint,
        server_addr,
        "localhost",
        session(clock.clone()),
        KEY,
        NONCE_DOMAIN,
        monitor(),
    )
    .await
    .unwrap();
    clock.advance_secs(100);
    let result = tokio::time::timeout(Duration::from_secs(1), runtime.run())
        .await
        .unwrap();
    assert!(matches!(result, Err(ClientRuntimeError::Session(_))));
    assert_eq!(runtime.session_state(), ClientState::Expired);
    assert_eq!(runtime.drain_latest_frame(), None);
    assert_eq!(runtime.sent_input_count(), 0);
    let _ = runtime.shutdown(std::time::Instant::now() + Duration::from_secs(1));
    server_task.await.unwrap();
}

#[tokio::test]
async fn window_close_terminates_and_clears_input_and_render_state() {
    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let endpoint = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &server.cert_der).unwrap();
    let server_task = tokio::spawn(async move {
        let incoming = server.endpoint.accept().await.unwrap();
        let connection = incoming.await.unwrap();
        connection.closed().await;
    });
    let mut runtime = ClientRuntime::connect(
        &endpoint,
        server_addr,
        "localhost",
        session(MockClock::from_secs(100)),
        KEY,
        NONCE_DOMAIN,
        monitor(),
    )
    .await
    .unwrap();
    runtime
        .handle_window_event(WindowEvent::Closed)
        .expect_err("window close must terminate the runtime");
    assert_eq!(runtime.session_state(), ClientState::Expired);
    assert_eq!(runtime.drain_latest_frame(), None);
    assert_eq!(runtime.sent_input_count(), 0);
    server_task.await.unwrap();
}
