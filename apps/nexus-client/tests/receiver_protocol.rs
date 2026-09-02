use nexus_client::{ClientInputError, ClientInputSender, ClientReceiver};
use nexus_crypto::NonceSequence;
use nexus_input::{InputError, InputEvent, KeyAction, Modifiers};
use nexus_protocol::{
    video_packet, CursorPosition, KeyEvent, MonitorInfo, MouseButton as ProtoMouseButton,
    MouseMove, MouseWheel, TextInput, VideoPacketHeader,
};
use nexus_transport::{
    control::{decode_framed_control, encode_framed_control},
    video::{
        encode_video_datagram, packetize_video_frame, seal_video_frame, MAX_VIDEO_DATAGRAM_SIZE,
    },
};

const KEY: [u8; 32] = [0xA5; 32];
const NONCE_DOMAIN: u32 = 0x0102_0304;

fn frame_datagrams(
    sequence: &mut NonceSequence,
    frame_id: u32,
    timestamp_us: u64,
    keyframe: bool,
    access_unit: &[u8],
) -> Vec<Vec<u8>> {
    let mut header = VideoPacketHeader {
        version: video_packet::CURRENT_VERSION,
        flags: if keyframe {
            video_packet::flags::KEYFRAME
        } else {
            0
        },
        stream_id: 1,
        frame_id,
        packet_id: 0,
        packet_count: 0,
        timestamp_us,
        nonce_sequence: 0,
        payload_len: 0,
    };
    let sealed = seal_video_frame(&KEY, sequence, &header, 1, access_unit).unwrap();
    header.nonce_sequence = u64::from_be_bytes(sealed.nonce[4..].try_into().unwrap());
    packetize_video_frame(&header, &sealed.ciphertext, 1000)
        .unwrap()
        .into_iter()
        .map(|(header, payload)| encode_video_datagram(&header, &payload).unwrap())
        .collect()
}

fn deliver(receiver: &mut ClientReceiver, datagrams: &[Vec<u8>]) {
    for datagram in datagrams {
        receiver.accept_datagram(datagram).unwrap();
    }
}

#[test]
fn emits_only_an_authenticated_frame_and_preserves_encoded_metadata() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let datagrams = frame_datagrams(&mut sender_sequence, 42, 1_234_567, true, b"encoded-h264");
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    deliver(&mut receiver, &datagrams);

    assert_eq!(
        receiver.drain_latest_frame(),
        Some(nexus_client::DecodedFrameJob {
            frame_id: 42,
            timestamp_us: 1_234_567,
            keyframe: true,
            access_unit: b"encoded-h264".to_vec(),
        })
    );
}

#[test]
fn rejects_truncated_oversized_and_malformed_datagrams_without_emitting_jobs() {
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);
    let malformed_header = VideoPacketHeader {
        version: video_packet::CURRENT_VERSION,
        flags: video_packet::flags::FRAME_START | video_packet::flags::FRAME_END,
        stream_id: 1,
        frame_id: 1,
        packet_id: 1,
        packet_count: 1,
        timestamp_us: 0,
        nonce_sequence: 0,
        payload_len: 1,
    };
    let malformed = encode_video_datagram(&malformed_header, &[0]).unwrap();

    assert!(receiver.accept_datagram(&[0; 10]).is_err());
    assert!(receiver
        .accept_datagram(&vec![0; MAX_VIDEO_DATAGRAM_SIZE + 1])
        .is_err());
    assert!(receiver.accept_datagram(&malformed).is_err());
    assert_eq!(receiver.drain_latest_frame(), None);
}

#[test]
fn rejects_a_header_modified_after_aead_seal_without_emitting_a_job() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let mut datagram = frame_datagrams(&mut sender_sequence, 7, 8, false, b"secret-frame")
        .into_iter()
        .next()
        .unwrap();
    datagram[7] = 8;
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    assert!(receiver.accept_datagram(&datagram).is_err());
    assert_eq!(receiver.drain_latest_frame(), None);
}

#[test]
fn recovers_after_a_lost_earlier_nonce_sequence() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let _lost = frame_datagrams(&mut sender_sequence, 1, 100, false, b"lost");
    let delivered = frame_datagrams(&mut sender_sequence, 2, 101, true, b"received");
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    deliver(&mut receiver, &delivered);

    assert_eq!(
        receiver.drain_latest_frame(),
        Some(nexus_client::DecodedFrameJob {
            frame_id: 2,
            timestamp_us: 101,
            keyframe: true,
            access_unit: b"received".to_vec(),
        })
    );
}

#[test]
fn forged_high_frame_id_cannot_poison_authenticated_freshness() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let mut forged = frame_datagrams(&mut sender_sequence, 100, 1_000, false, b"forged");
    let legitimate = frame_datagrams(&mut sender_sequence, 2, 101, true, b"legitimate");
    let last = forged.last_mut().unwrap().last_mut().unwrap();
    *last ^= 1;
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    assert!(receiver.accept_datagram(&forged.remove(0)).is_err());
    deliver(&mut receiver, &legitimate);

    assert_eq!(
        receiver.drain_latest_frame(),
        Some(nexus_client::DecodedFrameJob {
            frame_id: 2,
            timestamp_us: 101,
            keyframe: true,
            access_unit: b"legitimate".to_vec(),
        })
    );
}

#[test]
fn rejects_timestamp_and_keyframe_mutations_not_in_the_ciphertext() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let mut timestamp_changed = frame_datagrams(&mut sender_sequence, 3, 300, false, b"frame");
    timestamp_changed[0][12] ^= 1;
    let mut keyframe_changed = frame_datagrams(&mut sender_sequence, 4, 400, false, b"frame");
    keyframe_changed[0][1] ^= video_packet::flags::KEYFRAME;
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    assert!(receiver
        .accept_datagram(&timestamp_changed.remove(0))
        .is_err());
    assert!(receiver
        .accept_datagram(&keyframe_changed.remove(0))
        .is_err());
    assert_eq!(receiver.drain_latest_frame(), None);
}

#[test]
fn rejects_a_replayed_nonce_sequence_on_a_different_frame() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let first = frame_datagrams(&mut sender_sequence, 1, 100, false, b"first");
    let mut replayed_nonce = frame_datagrams(&mut sender_sequence, 2, 101, false, b"second");
    let nonce_start = nexus_protocol::video_packet::NONCE_SEQUENCE_OFFSET;
    replayed_nonce[0][nonce_start..nonce_start + 8].copy_from_slice(&0u64.to_be_bytes());
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    deliver(&mut receiver, &first);
    assert!(receiver.drain_latest_frame().is_some());
    assert!(receiver.accept_datagram(&replayed_nonce.remove(0)).is_err());
    assert_eq!(receiver.drain_latest_frame(), None);
}

#[test]
fn rejects_a_duplicate_nonce_frame_without_emitting_a_second_job() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let datagrams = frame_datagrams(&mut sender_sequence, 9, 10, false, b"once");
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    deliver(&mut receiver, &datagrams);
    assert!(receiver.drain_latest_frame().is_some());
    deliver(&mut receiver, &datagrams);

    assert_eq!(receiver.drain_latest_frame(), None);
}

#[test]
fn keeps_only_the_newest_authenticated_frame_and_drops_stale_frames() {
    let mut sender_sequence = NonceSequence::new(NONCE_DOMAIN);
    let first = frame_datagrams(&mut sender_sequence, 10, 100, false, b"old");
    let second = frame_datagrams(&mut sender_sequence, 11, 101, true, b"new");
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);

    deliver(&mut receiver, &first);
    deliver(&mut receiver, &second);
    deliver(&mut receiver, &first);

    assert_eq!(
        receiver.drain_latest_frame(),
        Some(nexus_client::DecodedFrameJob {
            frame_id: 11,
            timestamp_us: 101,
            keyframe: true,
            access_unit: b"new".to_vec(),
        })
    );
    assert_eq!(receiver.drain_latest_frame(), None);
}

#[test]
fn rejects_invalid_semantic_input_before_encoding() {
    let event = InputEvent::Key {
        physical_code: u32::from(u16::MAX) + 1,
        logical_code: 0,
        action: KeyAction::Down,
        modifiers: Modifiers::NONE,
    };

    assert_eq!(
        ClientInputSender::encode(event),
        Err(ClientInputError::Input(InputError::InvalidPhysicalKeyCode))
    );
}

#[test]
fn frames_valid_semantic_input_for_the_control_stream() {
    let bytes = ClientInputSender::encode(InputEvent::MouseMove { x: -12, y: 34 }).unwrap();

    assert_eq!(
        decode_framed_control::<MouseMove>(&bytes).unwrap(),
        MouseMove { x: -12, y: 34 }
    );
}

#[test]
fn frames_semantic_key_input_for_the_control_stream() {
    let bytes = ClientInputSender::encode(InputEvent::Key {
        physical_code: 45,
        logical_code: 46,
        action: KeyAction::Up,
        modifiers: Modifiers::CTRL,
    })
    .unwrap();

    assert_eq!(
        decode_framed_control::<KeyEvent>(&bytes).unwrap(),
        KeyEvent {
            physical_code: 45,
            logical_code: 46,
            pressed: false,
            modifiers: 2,
        }
    );
}

#[test]
fn frames_semantic_text_input_for_the_control_stream() {
    let bytes = ClientInputSender::encode(InputEvent::Text("hello".into())).unwrap();

    assert_eq!(
        decode_framed_control::<TextInput>(&bytes).unwrap(),
        TextInput {
            text: "hello".into()
        }
    );
}

#[test]
fn frames_semantic_mouse_button_input_for_the_control_stream() {
    let bytes = ClientInputSender::encode(InputEvent::MouseButton {
        button: nexus_input::MouseButton::Forward,
        pressed: true,
    })
    .unwrap();

    assert_eq!(
        decode_framed_control::<ProtoMouseButton>(&bytes).unwrap(),
        ProtoMouseButton {
            button: 4,
            pressed: true,
        }
    );
}

#[test]
fn frames_semantic_mouse_wheel_input_for_the_control_stream() {
    let bytes = ClientInputSender::encode(InputEvent::MouseWheel {
        delta_x: -120,
        delta_y: 240,
    })
    .unwrap();

    assert_eq!(
        decode_framed_control::<MouseWheel>(&bytes).unwrap(),
        MouseWheel {
            delta_x: -120,
            delta_y: 240,
        }
    );
}

#[test]
fn accepts_a_well_framed_inbound_cursor_control_message() {
    let bytes = encode_framed_control(&CursorPosition {
        x: 20,
        y: -30,
        visible: true,
        shape_id: 4,
    })
    .unwrap();
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);
    receiver
        .set_cursor_monitor(MonitorInfo {
            id: 1,
            origin_x: 0,
            origin_y: -100,
            width: 100,
            height: 200,
            scale: 1.0,
        })
        .unwrap();

    assert_eq!(
        receiver.accept_control(&bytes).unwrap(),
        CursorPosition {
            x: 20,
            y: -30,
            visible: true,
            shape_id: 4,
        }
    );
}

#[test]
fn rejects_cursor_control_outside_the_configured_monitor() {
    let bytes = encode_framed_control(&CursorPosition {
        x: 100,
        y: 0,
        visible: true,
        shape_id: 4,
    })
    .unwrap();
    let mut receiver = ClientReceiver::new(KEY, NONCE_DOMAIN);
    receiver
        .set_cursor_monitor(MonitorInfo {
            id: 1,
            origin_x: 0,
            origin_y: 0,
            width: 100,
            height: 100,
            scale: 1.0,
        })
        .unwrap();

    assert!(receiver.accept_control(&bytes).is_err());
}
