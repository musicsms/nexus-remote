//! Windows-specific backend boundary for Phase 1.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendError {
    UnsupportedPlatform,
}

pub trait CaptureBackend {
    fn start(&mut self) -> Result<(), BackendError>;
}

pub trait EncoderBackend {
    fn configure(&mut self, width: u32, height: u32) -> Result<(), BackendError>;
}

pub trait InputBackend {
    fn dispatch(&mut self, event: &[u8]) -> Result<(), BackendError>;
}

pub trait CursorBackend {
    fn snapshot(&mut self) -> Result<Vec<u8>, BackendError>;
}

pub struct UnsupportedBackend;

#[cfg(not(windows))]
impl CaptureBackend for UnsupportedBackend {
    fn start(&mut self) -> Result<(), BackendError> {
        Err(BackendError::UnsupportedPlatform)
    }
}

impl EncoderBackend for UnsupportedBackend {
    fn configure(&mut self, _width: u32, _height: u32) -> Result<(), BackendError> {
        Err(BackendError::UnsupportedPlatform)
    }
}

impl InputBackend for UnsupportedBackend {
    fn dispatch(&mut self, _event: &[u8]) -> Result<(), BackendError> {
        Err(BackendError::UnsupportedPlatform)
    }
}

impl CursorBackend for UnsupportedBackend {
    fn snapshot(&mut self) -> Result<Vec<u8>, BackendError> {
        Err(BackendError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_backend_fails_closed() {
        let mut backend = UnsupportedBackend;
        assert_eq!(backend.start(), Err(BackendError::UnsupportedPlatform));
    }
}
