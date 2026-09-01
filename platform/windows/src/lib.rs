//! Safe, platform-neutral contracts for Windows Phase 1 backends.
//!
//! Windows API types stay in private `cfg(windows)` implementation modules.

mod capture;
mod codec;
mod cursor;
mod error;
mod input;

pub use capture::{CaptureApi, CaptureConfig, CaptureState, WindowsCaptureSource};
#[cfg(feature = "test-support")]
#[doc = "Deterministic encoder adapter enabled only by `test-support`."]
pub use codec::EncoderTransform;
pub use codec::WindowsH264Encoder;
pub use cursor::CursorSnapshot;
pub use error::{BackendError, BackendErrorKind, BackendResult};
pub use input::{InputInjector, MonitorBounds, SystemInputApi};
