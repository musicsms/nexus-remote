//! Integration test verifying host worker video streaming pipeline and input handling.
//! Part of Nexus Remote Desktop Platform.

use std::sync::Arc;
use tokio::sync::mpsc;

use nexus_common::id::SessionId;
use nexus_desktop_host::worker::DesktopHostWorker;
use nexus_protocol::{video_packet::CURRENT_VERSION, KeyEvent, MouseMove};
use nexus_transport::video::decode_video_datagram;
use prost::Message;

#[tokio::test]
async fn test_desktop_host_worker_streaming_and_input_flow() {
    let session_id = SessionId::new("sess-host-worker-01").unwrap();
    let worker = DesktopHostWorker::new(session_id);

    let aead_key = [77u8; 32];
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1000);

    // 1. Spawn streaming worker in background
    let worker_arc = Arc::new(worker);
    let worker_clone = worker_arc.clone();

    let stream_handle = tokio::spawn(async move {
        worker_clone
            .run_streamer(aead_key, 64, 64, tx)
            .await
            .unwrap();
    });

    // 2. Receive multiple video datagrams produced by the worker
    let mut datagrams = Vec::new();
    for _ in 0..5 {
        if let Some(dg) = rx.recv().await {
            datagrams.push(dg);
        }
    }

    assert!(!datagrams.is_empty());

    // 3. Verify datagrams decode cleanly
    for dg in &datagrams {
        let (header, payload) = decode_video_datagram(dg).unwrap();
        assert_eq!(header.version, CURRENT_VERSION);
        assert!(!payload.is_empty());
    }

    // 4. Test input handler with synthetic protobuf events
    let key_evt = KeyEvent {
        physical_code: 65,
        logical_code: 65,
        pressed: true,
        modifiers: 0,
    };
    let mut key_bytes = Vec::new();
    key_evt.encode(&mut key_bytes).unwrap();
    worker_arc
        .input_handler
        .handle_key_event(&key_bytes)
        .unwrap();

    let mouse_evt = MouseMove { x: 500, y: 400 };
    let mut mouse_bytes = Vec::new();
    mouse_evt.encode(&mut mouse_bytes).unwrap();
    worker_arc
        .input_handler
        .handle_mouse_move(&mouse_bytes)
        .unwrap();

    assert_eq!(worker_arc.input_handler.events_received(), 2);

    // 5. Clean worker shutdown
    worker_arc.stop();
    let _ = stream_handle.await;
    assert!(!worker_arc.is_running());
}
