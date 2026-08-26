use nexus_protocol::MouseMove;
use prost::Message;

#[test]
fn mouse_move_round_trip() {
    let msg = MouseMove { x: -42, y: 1080 };

    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode should not fail");

    let decoded = MouseMove::decode(buf.as_slice()).expect("decode should not fail");

    assert_eq!(decoded, msg);
}

#[test]
fn session_hello_round_trip() {
    use nexus_protocol::SessionHello;

    let msg = SessionHello {
        protocol_version: 1,
        session_id: "ses_01".to_string(),
        device_id: "dev_01".to_string(),
        capability: vec![1, 2, 3, 4],
        ephemeral_public_key: vec![5, 6, 7, 8],
    };

    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode should not fail");

    let decoded = SessionHello::decode(buf.as_slice()).expect("decode should not fail");

    assert_eq!(decoded, msg);
}

#[test]
fn input_messages_round_trip() {
    let key = nexus_protocol::KeyEvent {
        physical_code: 30,
        logical_code: 65,
        pressed: true,
        modifiers: 3,
    };
    let mut encoded = Vec::new();
    key.encode(&mut encoded).unwrap();
    assert_eq!(
        nexus_protocol::KeyEvent::decode(encoded.as_slice()).unwrap(),
        key
    );

    let button = nexus_protocol::MouseButton {
        button: 1,
        pressed: false,
    };
    encoded.clear();
    button.encode(&mut encoded).unwrap();
    assert_eq!(
        nexus_protocol::MouseButton::decode(encoded.as_slice()).unwrap(),
        button
    );

    let wheel = nexus_protocol::MouseWheel {
        delta_x: -1,
        delta_y: 120,
    };
    encoded.clear();
    wheel.encode(&mut encoded).unwrap();
    assert_eq!(
        nexus_protocol::MouseWheel::decode(encoded.as_slice()).unwrap(),
        wheel
    );

    let text = nexus_protocol::TextInput {
        text: "Xin chào".to_owned(),
    };
    encoded.clear();
    text.encode(&mut encoded).unwrap();
    assert_eq!(
        nexus_protocol::TextInput::decode(encoded.as_slice()).unwrap(),
        text
    );
}

#[test]
fn session_hello_validation_rejects_hostile_values() {
    use nexus_protocol::{SessionHelloError, CURRENT_PROTOCOL_VERSION, MAX_CAPABILITY_LEN};
    let mut hello = nexus_protocol::SessionHello {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        session_id: "ses_01".into(),
        device_id: "dev_01".into(),
        capability: vec![],
        ephemeral_public_key: vec![],
    };
    assert!(hello.validate().is_ok());
    hello.protocol_version = CURRENT_PROTOCOL_VERSION + 1;
    assert!(matches!(
        hello.validate(),
        Err(SessionHelloError::UnsupportedVersion { .. })
    ));
    hello.protocol_version = CURRENT_PROTOCOL_VERSION;
    hello.capability = vec![0; MAX_CAPABILITY_LEN + 1];
    assert!(matches!(
        hello.validate(),
        Err(SessionHelloError::CapabilityTooLarge { .. })
    ));
}

#[test]
fn monitor_info_round_trip_and_validation() {
    let mut monitor = nexus_protocol::MonitorInfo {
        id: 1,
        origin_x: -1920,
        origin_y: 0,
        width: 1920,
        height: 1080,
        scale: 1.25,
    };
    assert!(monitor.validate().is_ok());
    let mut bytes = Vec::new();
    monitor.encode(&mut bytes).unwrap();
    assert_eq!(
        nexus_protocol::MonitorInfo::decode(bytes.as_slice()).unwrap(),
        monitor
    );
    monitor.width = 0;
    assert!(matches!(
        monitor.validate(),
        Err(nexus_protocol::MonitorInfoError::InvalidDimensions)
    ));
}

#[test]
fn cursor_shape_and_position_round_trip() {
    let shape = nexus_protocol::CursorShape {
        id: 7,
        width: 16,
        height: 16,
        hotspot_x: 1,
        hotspot_y: 2,
        pixel_format: 1,
        data: vec![0; 16 * 16 * 4],
    };
    assert!(shape.validate().is_ok());
    let position = nexus_protocol::CursorPosition {
        x: -10,
        y: 20,
        visible: true,
        shape_id: 7,
    };
    let mut bytes = Vec::new();
    shape.encode(&mut bytes).unwrap();
    assert_eq!(
        nexus_protocol::CursorShape::decode(bytes.as_slice()).unwrap(),
        shape
    );
    bytes.clear();
    position.encode(&mut bytes).unwrap();
    assert_eq!(
        nexus_protocol::CursorPosition::decode(bytes.as_slice()).unwrap(),
        position
    );
}

#[test]
fn rejects_invalid_monitor_scale_and_cursor_payload() {
    let monitor = nexus_protocol::MonitorInfo {
        id: 1,
        origin_x: 0,
        origin_y: 0,
        width: 1,
        height: 1,
        scale: 100.1,
    };
    assert_eq!(
        monitor.validate(),
        Err(nexus_protocol::MonitorInfoError::ScaleTooLarge)
    );
    let shape = nexus_protocol::CursorShape {
        id: 1,
        width: 2,
        height: 2,
        hotspot_x: 0,
        hotspot_y: 0,
        pixel_format: 1,
        data: vec![0; 3],
    };
    assert_eq!(
        shape.validate(),
        Err(nexus_protocol::CursorShapeError::InvalidPayloadLength)
    );
}

#[test]
fn session_capability_round_trip_and_validation() {
    let mut capability = nexus_protocol::SessionCapability {
        version: 1,
        issuer: "control".into(),
        session_id: "ses_01".into(),
        subject_user_id: "usr_01".into(),
        client_device_id: "dev_c".into(),
        target_device_id: "dev_t".into(),
        permissions: vec!["desktop.control".into()],
        restrictions: vec![],
        not_before: 10,
        expires_at: 20,
        nonce: vec![1, 2, 3],
        agent_min_protocol: 1,
        agent_max_protocol: 1,
        client_ephemeral_public_key: vec![],
        signature: vec![],
    };
    assert!(capability.validate().is_ok());
    let mut bytes = Vec::new();
    capability.encode(&mut bytes).unwrap();
    assert_eq!(
        nexus_protocol::SessionCapability::decode(bytes.as_slice()).unwrap(),
        capability
    );
    capability.expires_at = 10;
    assert!(matches!(
        capability.validate(),
        Err(nexus_protocol::SessionCapabilityError::InvalidValidityWindow)
    ));

    // signing_bytes() is deterministic and independent of the signature field itself
    let signing_payload_1 = capability.signing_bytes();
    capability.signature = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let signing_payload_2 = capability.signing_bytes();
    assert_eq!(signing_payload_1, signing_payload_2);
    assert!(!signing_payload_1.is_empty());
}
