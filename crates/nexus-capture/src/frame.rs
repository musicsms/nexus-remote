use bytes::Bytes;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("captured frame dimensions must be non-zero")]
    InvalidDimensions,
    #[error("captured frame size overflows the platform size type")]
    FrameSizeOverflow,
    #[error("BGRA frame payload has {actual} bytes; expected {expected}")]
    InvalidBgraPayload { actual: usize, expected: usize },
}

/// Pixel representation produced by a capture backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// GPU/native texture handle; the backend owns the handle lifetime.
    Native,
    /// 8-bit BGRA pixels in row-major order.
    Bgra8,
}

/// A captured desktop frame and the metadata needed by the encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub frame_id: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Bytes,
}

impl CapturedFrame {
    pub fn expected_bgra_len(width: u32, height: u32) -> Result<usize, CaptureError> {
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or(CaptureError::FrameSizeOverflow)?;
        pixels.checked_mul(4).ok_or(CaptureError::FrameSizeOverflow)
    }

    pub fn new_bgra(
        frame_id: u64,
        timestamp_us: u64,
        width: u32,
        height: u32,
        data: Bytes,
    ) -> Result<Self, CaptureError> {
        let frame = Self {
            frame_id,
            timestamp_us,
            width,
            height,
            format: PixelFormat::Bgra8,
            data,
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.width == 0 || self.height == 0 {
            return Err(CaptureError::InvalidDimensions);
        }
        if self.format == PixelFormat::Bgra8 {
            let expected = Self::expected_bgra_len(self.width, self.height)?;
            if self.data.len() != expected {
                return Err(CaptureError::InvalidBgraPayload {
                    actual: self.data.len(),
                    expected,
                });
            }
        }
        Ok(())
    }
}

/// Platform capture contract. Implementations must not block indefinitely.
pub trait CaptureSource: Send {
    type Error: std::error::Error + Send + Sync + 'static;

    fn next_frame(&mut self) -> Result<CapturedFrame, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bgra_frame_size() {
        let frame = CapturedFrame::new_bgra(1, 2, 2, 1, Bytes::from_static(&[0; 8])).unwrap();
        assert_eq!(frame.validate(), Ok(()));
    }

    #[test]
    fn constructor_rejects_malformed_payload() {
        assert_eq!(
            CapturedFrame::new_bgra(1, 2, 1, 1, Bytes::new()),
            Err(CaptureError::InvalidBgraPayload {
                actual: 0,
                expected: 4
            })
        );
    }

    #[test]
    fn rejects_invalid_dimensions_and_payload() {
        let frame = CapturedFrame {
            frame_id: 1,
            timestamp_us: 2,
            width: 0,
            height: 1,
            format: PixelFormat::Bgra8,
            data: Bytes::new(),
        };
        assert_eq!(frame.validate(), Err(CaptureError::InvalidDimensions));

        let frame = CapturedFrame {
            width: 1,
            height: 1,
            data: Bytes::new(),
            ..frame
        };
        assert_eq!(
            frame.validate(),
            Err(CaptureError::InvalidBgraPayload {
                actual: 0,
                expected: 4
            })
        );
    }

    #[test]
    fn bgra_size_uses_checked_arithmetic() {
        assert_eq!(
            CapturedFrame::expected_bgra_len(u32::MAX, u32::MAX),
            Err(CaptureError::FrameSizeOverflow)
        );
        assert_eq!(CapturedFrame::expected_bgra_len(2, 3), Ok(24));
    }
}
