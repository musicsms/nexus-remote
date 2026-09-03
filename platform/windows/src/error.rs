use std::error::Error;
use std::fmt::{Display, Formatter};

/// The category of a Windows backend failure, independent of Windows API types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorKind {
    UnsupportedPlatform,
    UnsupportedApi,
    PermissionDenied,
    InvalidConfiguration,
    InitializationFailed,
    Timeout,
    DeviceLost,
    FrameUnavailable,
    InvalidFrame,
    InvalidInput,
    InvalidCursorDimensions,
    HotspotOutOfBounds,
    CursorPayloadLength,
    NativeFailure,
    Stopped,
}

/// A structured Windows backend error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    kind: BackendErrorKind,
}

impl BackendError {
    pub const fn new(kind: BackendErrorKind) -> Self {
        Self { kind }
    }

    pub const fn kind(&self) -> BackendErrorKind {
        self.kind
    }
}

impl From<BackendErrorKind> for BackendError {
    fn from(kind: BackendErrorKind) -> Self {
        Self::new(kind)
    }
}

impl Display for BackendError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Windows backend error: {:?}", self.kind)
    }
}

impl Error for BackendError {}

/// Result type returned by Windows backend contracts.
pub type BackendResult<T> = Result<T, BackendError>;
