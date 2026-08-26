//! Platform-independent desktop capture contracts and freshness buffering.

mod frame;
mod latest_queue;
pub mod synthetic;

pub use frame::{CaptureError, CaptureSource, CapturedFrame, PixelFormat};
pub use latest_queue::LatestFrameQueue;
pub use synthetic::SyntheticCaptureSource;

pub fn init() {
    // Reserved for platform backend registration.
}
