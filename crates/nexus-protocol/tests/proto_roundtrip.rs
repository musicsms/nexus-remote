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
