use crate::{BackendErrorKind, BackendResult};

/// A bounded RGBA cursor image and its position in desktop coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub visible: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub rgba: Vec<u8>,
}

impl CursorSnapshot {
    pub const MAX_DIMENSION: u32 = 256;
    pub const MAX_RGBA_BYTES: usize = 256 * 256 * 4;

    /// Validates the bounded cursor shape before it reaches a native API.
    pub fn validate(&self) -> BackendResult<()> {
        if !self.visible && self.width == 0 && self.height == 0 && self.rgba.is_empty() {
            return Ok(());
        }

        if self.width == 0
            || self.height == 0
            || self.width > Self::MAX_DIMENSION
            || self.height > Self::MAX_DIMENSION
        {
            return Err(BackendErrorKind::InvalidCursorDimensions.into());
        }

        let expected_len = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(BackendErrorKind::CursorPayloadLength)?;

        if expected_len > Self::MAX_RGBA_BYTES || self.rgba.len() != expected_len {
            return Err(BackendErrorKind::CursorPayloadLength.into());
        }

        if self.visible && (self.hotspot_x >= self.width || self.hotspot_y >= self.height) {
            return Err(BackendErrorKind::HotspotOutOfBounds.into());
        }

        Ok(())
    }
}

/// Native cursor calls are deliberately expressed without Windows types.
#[allow(dead_code)]
pub(crate) trait NativeCursorApi {
    fn snapshot(&mut self) -> BackendResult<CursorSnapshot>;
}
