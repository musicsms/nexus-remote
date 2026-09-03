use crate::{CodecError, EncodedFrame, EncoderConfig, VideoEncoder};
use bytes::{BufMut, Bytes, BytesMut};
use nexus_capture::CapturedFrame;

pub const DEFAULT_KEYFRAME_INTERVAL: u32 = 60;

pub const NAL_START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];
pub const NAL_TYPE_SPS: u8 = 0x67;
pub const NAL_TYPE_PPS: u8 = 0x68;
pub const NAL_TYPE_IDR: u8 = 0x65;
pub const NAL_TYPE_NON_IDR: u8 = 0x41;

/// Software fallback video encoder implementing simulated H.264 Annex B stream encapsulation.
#[derive(Debug, Clone)]
pub struct SoftwareFallbackEncoder {
    config: Option<EncoderConfig>,
    keyframe_interval: u32,
    frame_count: u64,
    force_keyframe: bool,
}

impl Default for SoftwareFallbackEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftwareFallbackEncoder {
    pub fn new() -> Self {
        Self {
            config: None,
            keyframe_interval: DEFAULT_KEYFRAME_INTERVAL,
            frame_count: 0,
            force_keyframe: true,
        }
    }

    pub fn with_keyframe_interval(keyframe_interval: u32) -> Self {
        Self {
            config: None,
            keyframe_interval,
            frame_count: 0,
            force_keyframe: true,
        }
    }

    pub fn config(&self) -> Option<&EncoderConfig> {
        self.config.as_ref()
    }

    pub fn keyframe_interval(&self) -> u32 {
        self.keyframe_interval
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    fn build_annex_b_payload(
        &self,
        config: &EncoderConfig,
        is_keyframe: bool,
        frame_data: &[u8],
    ) -> Bytes {
        let mut buffer = BytesMut::new();

        if is_keyframe {
            // NAL 1: SPS (Sequence Parameter Set) simulation
            buffer.extend_from_slice(&NAL_START_CODE);
            buffer.put_u8(NAL_TYPE_SPS);
            buffer.put_u32(config.width);
            buffer.put_u32(config.height);
            buffer.put_u32(config.max_fps);

            // NAL 2: PPS (Picture Parameter Set) simulation
            buffer.extend_from_slice(&NAL_START_CODE);
            buffer.put_u8(NAL_TYPE_PPS);
            buffer.put_u8(0x01);

            // NAL 3: IDR Slice
            buffer.extend_from_slice(&NAL_START_CODE);
            buffer.put_u8(NAL_TYPE_IDR);
            buffer.extend_from_slice(frame_data);
        } else {
            // NAL 1: Non-IDR Slice
            buffer.extend_from_slice(&NAL_START_CODE);
            buffer.put_u8(NAL_TYPE_NON_IDR);
            buffer.extend_from_slice(frame_data);
        }

        buffer.freeze()
    }
}

impl VideoEncoder for SoftwareFallbackEncoder {
    fn configure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
        let validated = config.validate()?;
        self.config = Some(validated);
        self.force_keyframe = true;
        self.frame_count = 0;
        Ok(())
    }

    fn encode(&mut self, frame: CapturedFrame) -> Result<EncodedFrame, CodecError> {
        let config = self.config.as_ref().ok_or(CodecError::NotConfigured)?;

        if frame.width != config.width || frame.height != config.height {
            return Err(CodecError::FrameDimensionsMismatch);
        }

        let is_keyframe = self.force_keyframe
            || (self.keyframe_interval > 0
                && self
                    .frame_count
                    .is_multiple_of(self.keyframe_interval as u64));

        self.force_keyframe = false;
        self.frame_count += 1;

        let payload = self.build_annex_b_payload(config, is_keyframe, &frame.data);

        Ok(EncodedFrame {
            frame_id: frame.frame_id,
            timestamp_us: frame.timestamp_us,
            keyframe: is_keyframe,
            data: payload,
        })
    }

    fn request_keyframe(&mut self) -> Result<(), CodecError> {
        self.force_keyframe = true;
        Ok(())
    }

    fn reconfigure(&mut self, config: EncoderConfig) -> Result<(), CodecError> {
        let validated = config.validate()?;
        self.config = Some(validated);
        self.force_keyframe = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodecKind;
    use nexus_capture::PixelFormat;

    fn sample_frame(frame_id: u64, timestamp_us: u64, width: u32, height: u32) -> CapturedFrame {
        let len = (width * height * 4) as usize;
        CapturedFrame {
            frame_id,
            timestamp_us,
            width,
            height,
            format: PixelFormat::Bgra8,
            data: Bytes::from(vec![0xAA; len]),
        }
    }

    fn valid_config(width: u32, height: u32) -> EncoderConfig {
        EncoderConfig {
            codec: CodecKind::H264,
            width,
            height,
            max_fps: 30,
            bitrate_bps: 2_000_000,
        }
    }

    #[test]
    fn unconfigured_encoder_rejects_encode() {
        let mut encoder = SoftwareFallbackEncoder::new();
        let frame = sample_frame(1, 0, 640, 480);
        assert_eq!(encoder.encode(frame), Err(CodecError::NotConfigured));
    }

    #[test]
    fn encoder_returns_one_encoded_frame_per_input() {
        let mut encoder = SoftwareFallbackEncoder::new();
        encoder.configure(valid_config(640, 480)).unwrap();

        let output = encoder.encode(sample_frame(7, 99, 640, 480)).unwrap();

        assert_eq!(output.frame_id, 7);
        assert_eq!(output.timestamp_us, 99);
    }

    #[test]
    fn encodes_frames_and_respects_keyframe_interval() {
        let mut encoder = SoftwareFallbackEncoder::with_keyframe_interval(3);
        let config = valid_config(640, 480);
        encoder.configure(config).unwrap();
        assert_eq!(encoder.config(), Some(&config));
        assert_eq!(encoder.keyframe_interval(), 3);
        assert_eq!(encoder.frame_count(), 0);

        // Frame 1 -> should be keyframe (first frame)
        let f1 = encoder.encode(sample_frame(1, 0, 640, 480)).unwrap();
        assert_eq!(f1.frame_id, 1);
        assert_eq!(f1.timestamp_us, 0);
        assert!(f1.keyframe, "first frame must be a keyframe");
        assert!(!f1.data.is_empty());
        // Verify Annex B start code and SPS/PPS/IDR headers
        assert_eq!(&f1.data[0..4], &NAL_START_CODE);
        assert_eq!(f1.data[4], NAL_TYPE_SPS);
        assert_eq!(encoder.frame_count(), 1);

        // Frame 2 -> delta frame
        let f2 = encoder.encode(sample_frame(2, 33333, 640, 480)).unwrap();
        assert_eq!(f2.frame_id, 2);
        assert_eq!(f2.timestamp_us, 33333);
        assert!(!f2.keyframe, "second frame should not be keyframe");
        assert_eq!(&f2.data[0..4], &NAL_START_CODE);
        assert_eq!(f2.data[4], NAL_TYPE_NON_IDR);
        assert_eq!(encoder.frame_count(), 2);

        // Frame 3 -> delta frame
        let f3 = encoder.encode(sample_frame(3, 66666, 640, 480)).unwrap();
        assert_eq!(f3.frame_id, 3);
        assert!(!f3.keyframe, "third frame should not be keyframe");
        assert_eq!(encoder.frame_count(), 3);

        // Frame 4 (frame_count 3 % interval 3 == 0) -> periodic keyframe
        let f4 = encoder.encode(sample_frame(4, 99999, 640, 480)).unwrap();
        assert_eq!(f4.frame_id, 4);
        assert!(f4.keyframe, "fourth frame (interval 3) must be keyframe");
        assert_eq!(encoder.frame_count(), 4);
    }

    #[test]
    fn forced_keyframe_request() {
        let mut encoder = SoftwareFallbackEncoder::with_keyframe_interval(100);
        encoder.configure(valid_config(640, 480)).unwrap();

        let f1 = encoder.encode(sample_frame(1, 0, 640, 480)).unwrap();
        assert!(f1.keyframe);

        let f2 = encoder.encode(sample_frame(2, 33333, 640, 480)).unwrap();
        assert!(!f2.keyframe);

        // Request keyframe explicitly
        encoder.request_keyframe().unwrap();

        let f3 = encoder.encode(sample_frame(3, 66666, 640, 480)).unwrap();
        assert!(
            f3.keyframe,
            "frame after request_keyframe must be a keyframe"
        );

        let f4 = encoder.encode(sample_frame(4, 99999, 640, 480)).unwrap();
        assert!(!f4.keyframe, "frame after forced keyframe reverts to delta");
    }

    #[test]
    fn rejects_dimension_mismatch() {
        let mut encoder = SoftwareFallbackEncoder::new();
        encoder.configure(valid_config(1920, 1080)).unwrap();

        let mismatch_frame = sample_frame(1, 0, 1280, 720);
        assert_eq!(
            encoder.encode(mismatch_frame),
            Err(CodecError::FrameDimensionsMismatch)
        );
    }

    #[test]
    fn rejects_invalid_configure_and_reconfigure() {
        let mut encoder = SoftwareFallbackEncoder::new();
        let invalid_config = EncoderConfig {
            codec: CodecKind::H264,
            width: 0,
            height: 1080,
            max_fps: 30,
            bitrate_bps: 1_000_000,
        };
        assert_eq!(
            encoder.configure(invalid_config),
            Err(CodecError::InvalidDimensions)
        );
        assert_eq!(
            encoder.reconfigure(invalid_config),
            Err(CodecError::InvalidDimensions)
        );
    }

    #[test]
    fn reconfigure_updates_config_and_forces_keyframe() {
        let mut encoder = SoftwareFallbackEncoder::with_keyframe_interval(100);
        encoder.configure(valid_config(640, 480)).unwrap();

        let f1 = encoder.encode(sample_frame(1, 0, 640, 480)).unwrap();
        assert!(f1.keyframe);
        let f2 = encoder.encode(sample_frame(2, 33333, 640, 480)).unwrap();
        assert!(!f2.keyframe);

        // Reconfigure with new dimensions
        let new_config = valid_config(1280, 720);
        encoder.reconfigure(new_config).unwrap();
        assert_eq!(encoder.config(), Some(&new_config));

        // Old dimensions should now fail
        assert_eq!(
            encoder.encode(sample_frame(3, 66666, 640, 480)),
            Err(CodecError::FrameDimensionsMismatch)
        );

        // New dimensions should succeed and be a keyframe
        let f3 = encoder.encode(sample_frame(3, 66666, 1280, 720)).unwrap();
        assert_eq!(f3.frame_id, 3);
        assert!(
            f3.keyframe,
            "first frame after reconfigure must be keyframe"
        );
    }
}
