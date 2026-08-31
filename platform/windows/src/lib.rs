//! Safe, platform-neutral contracts for Windows Phase 1 backends.
//!
//! Windows API types stay in private `cfg(windows)` implementation modules.

mod cursor;
mod error;
pub mod input;

pub use cursor::{CursorSnapshot, NativeCursorApi};
pub use error::{BackendError, BackendErrorKind, BackendResult};
pub use input::{InputInjector, MonitorBounds, SystemInputApi};
#[doc(hidden)]
pub use input::{InputRecord, NativeInputApi};
