use bytes::Bytes;
use nexus_capture::CapturedFrame;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    H264,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub codec: CodecKind,
    pub width: u32,
    pub height: u32,
    pub max_fps: u32,
    pub bitrate_bps: u32,
}

impl EncoderConfig {
    pub fn validate(self) -> Result<Self, CodecError> {
        if self.width == 0 || self.height == 0 || self.max_fps == 0 || self.bitrate_bps == 0 {
            return Err(CodecError::InvalidDimensions);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    pub frame_id: u64,
    pub timestamp_us: u64,
    pub keyframe: bool,
    pub data: Bytes,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("encoder configuration dimensions must be non-zero")]
    InvalidDimensions,
    #[error("encoder frame dimensions do not match configured dimensions")]
    FrameDimensionsMismatch,
}

pub trait VideoEncoder: Send {
    fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError>;
    fn encode(&mut self, frame: CapturedFrame) -> Result<EncodedFrame, CodecError>;
    fn request_keyframe(&mut self) -> Result<(), CodecError>;
    fn reconfigure(&mut self, config: EncoderConfig) -> Result<(), CodecError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn encoded_frame_preserves_capture_timing() {
        let frame = EncodedFrame {
            frame_id: 7,
            timestamp_us: 99,
            keyframe: true,
            data: Bytes::from_static(&[1, 2]),
        };
        assert_eq!(frame.frame_id, 7);
        assert!(frame.keyframe);
    }

    #[test]
    fn rejects_zero_encoder_parameters() {
        let config = EncoderConfig {
            codec: CodecKind::H264,
            width: 1920,
            height: 1080,
            max_fps: 0,
            bitrate_bps: 1,
        };
        assert_eq!(config.validate(), Err(CodecError::InvalidDimensions));
    }
}
