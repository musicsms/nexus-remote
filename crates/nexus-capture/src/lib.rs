//! Platform-independent desktop capture contracts and freshness buffering.

mod frame;
mod latest_queue;

pub use frame::{CaptureError, CaptureSource, CapturedFrame, PixelFormat};
pub use latest_queue::LatestFrameQueue;

pub fn init() {
    // Reserved for platform backend registration.
}
