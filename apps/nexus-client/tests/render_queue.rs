use nexus_client::{DecodedFrameJob, RenderQueue, RenderQueueError};

fn frame(frame_id: u32, access_unit: &[u8]) -> DecodedFrameJob {
    DecodedFrameJob {
        frame_id,
        timestamp_us: u64::from(frame_id) * 1_000,
        keyframe: frame_id == 1,
        access_unit: access_unit.to_vec(),
    }
}

#[test]
fn replacement_keeps_only_the_newest_frame_and_counts_the_drop() {
    let queue = RenderQueue::new();
    queue.push_latest(frame(1, b"first")).unwrap();
    queue.push_latest(frame(2, b"second")).unwrap();

    assert_eq!(queue.dropped_frames(), 1);
    assert_eq!(queue.take_latest(), Some(frame(2, b"second")));
    assert_eq!(queue.take_latest(), None);
}

#[test]
fn rejects_an_empty_access_unit_without_replacing_the_current_frame() {
    let queue = RenderQueue::new();
    queue.push_latest(frame(1, b"first")).unwrap();

    assert_eq!(
        queue.push_latest(frame(2, b"")),
        Err(RenderQueueError::EmptyAccessUnit)
    );
    assert_eq!(queue.take_latest(), Some(frame(1, b"first")));
}

#[test]
fn refuses_new_frames_after_shutdown() {
    let queue = RenderQueue::new();
    queue.shutdown();

    assert_eq!(
        queue.push_latest(frame(1, b"frame")),
        Err(RenderQueueError::Shutdown)
    );
    assert_eq!(queue.take_latest(), None);
}
