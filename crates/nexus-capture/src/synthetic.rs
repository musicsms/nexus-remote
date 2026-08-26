use crate::{CaptureSource, CapturedFrame, PixelFormat};
use bytes::Bytes;
use std::convert::Infallible;

/// A synthetic frame source generating deterministic BGRA pattern frames for testing.
#[derive(Debug, Clone)]
pub struct SyntheticCaptureSource {
    width: u32,
    height: u32,
    fps: u32,
    frame_count: u64,
}

impl SyntheticCaptureSource {
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self {
            width,
            height,
            fps: fps.max(1),
            frame_count: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    fn generate_bgra_pattern(&self, frame_id: u64) -> Bytes {
        let total_pixels = (self.width as usize) * (self.height as usize);
        let mut buffer = Vec::with_capacity(total_pixels * 4);

        let shift = (frame_id % 256) as u8;
        for y in 0..self.height {
            for x in 0..self.width {
                let b = (x as u8).wrapping_add(shift);
                let g = (y as u8).wrapping_add(shift);
                let r = ((x + y) as u8).wrapping_add(shift);
                let a = 255u8;
                buffer.extend_from_slice(&[b, g, r, a]);
            }
        }

        Bytes::from(buffer)
    }
}

impl CaptureSource for SyntheticCaptureSource {
    type Error = Infallible;

    fn next_frame(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.frame_count += 1;
        let frame_id = self.frame_count;
        let timestamp_us = (frame_id.saturating_sub(1)) * 1_000_000 / (self.fps as u64);
        let data = self.generate_bgra_pattern(frame_id);

        Ok(CapturedFrame {
            frame_id,
            timestamp_us,
            width: self.width,
            height: self.height,
            format: PixelFormat::Bgra8,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_bgra_frames_with_monotonic_metadata() {
        let mut source = SyntheticCaptureSource::new(640, 480, 30);
        assert_eq!(source.width(), 640);
        assert_eq!(source.height(), 480);
        assert_eq!(source.fps(), 30);
        assert_eq!(source.frame_count(), 0);

        let f1 = source.next_frame().unwrap();
        assert_eq!(f1.frame_id, 1);
        assert_eq!(f1.timestamp_us, 0);
        assert_eq!(f1.width, 640);
        assert_eq!(f1.height, 480);
        assert_eq!(f1.format, PixelFormat::Bgra8);
        assert_eq!(f1.data.len(), 640 * 480 * 4);
        assert!(f1.validate().is_ok());

        let f2 = source.next_frame().unwrap();
        assert_eq!(f2.frame_id, 2);
        assert_eq!(f2.timestamp_us, 33_333);
        assert_eq!(f2.width, 640);
        assert_eq!(f2.height, 480);
        assert_eq!(f2.format, PixelFormat::Bgra8);
        assert_eq!(f2.data.len(), 640 * 480 * 4);
        assert!(f2.validate().is_ok());

        let f3 = source.next_frame().unwrap();
        assert_eq!(f3.frame_id, 3);
        assert_eq!(f3.timestamp_us, 66_666);
        assert!(f3.validate().is_ok());

        // Pattern changes across frames
        assert_ne!(f1.data, f2.data);
        assert_ne!(f2.data, f3.data);

        // Alpha channel is always 255
        for &[_, _, _, a] in f1.data.as_chunks::<4>().0 {
            assert_eq!(a, 255);
        }
    }

    #[test]
    fn custom_dimensions_and_framerate() {
        let mut source = SyntheticCaptureSource::new(1920, 1080, 60);
        assert_eq!(source.fps(), 60);

        let frame = source.next_frame().unwrap();
        assert_eq!(frame.width, 1920);
        assert_eq!(frame.height, 1080);
        assert_eq!(frame.data.len(), 1920 * 1080 * 4);
        assert!(frame.validate().is_ok());

        let frame2 = source.next_frame().unwrap();
        assert_eq!(frame2.timestamp_us, 16_666);
    }

    #[test]
    fn zero_fps_fallback_to_minimum() {
        let mut source = SyntheticCaptureSource::new(320, 240, 0);
        assert_eq!(source.fps(), 1);

        let frame = source.next_frame().unwrap();
        assert_eq!(frame.timestamp_us, 0);

        let frame2 = source.next_frame().unwrap();
        assert_eq!(frame2.timestamp_us, 1_000_000);
    }
}
