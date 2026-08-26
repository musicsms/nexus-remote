use std::sync::{Mutex, PoisonError};

#[derive(Debug)]
struct QueueInner<T> {
    slot: Option<T>,
    dropped: u64,
}

/// Depth-one queue where the newest frame replaces a stale queued frame.
#[derive(Debug)]
pub struct LatestFrameQueue<T> {
    inner: Mutex<QueueInner<T>>,
}

impl<T> Default for LatestFrameQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LatestFrameQueue<T> {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(QueueInner {
                slot: None,
                dropped: 0,
            }),
        }
    }

    pub fn replace(&self, frame: T) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.slot.replace(frame).is_some() {
            inner.dropped += 1;
        }
    }

    pub fn take(&self) -> Option<T> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .slot
            .take()
    }

    pub fn dropped_count(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dropped
    }
}

#[cfg(test)]
mod tests {
    use super::LatestFrameQueue;

    #[test]
    fn newest_frame_replaces_stale_frame() {
        let queue = LatestFrameQueue::new();
        queue.replace(1);
        queue.replace(2);
        assert_eq!(queue.take(), Some(2));
        assert_eq!(queue.dropped_count(), 1);
        assert_eq!(queue.take(), None);
    }

    #[test]
    fn empty_queue_is_non_blocking() {
        let queue: LatestFrameQueue<u8> = LatestFrameQueue::new();
        assert_eq!(queue.take(), None);
        assert_eq!(queue.dropped_count(), 0);
    }

    #[test]
    fn producer_and_consumer_can_share_queue() {
        let queue = std::sync::Arc::new(LatestFrameQueue::new());
        let producer_queue = queue.clone();
        let producer = std::thread::spawn(move || {
            for frame in 0..100_u32 {
                producer_queue.replace(frame);
            }
        });
        producer.join().unwrap();
        assert!(queue.take().is_some());
        assert!(queue.dropped_count() > 0);
    }
}
