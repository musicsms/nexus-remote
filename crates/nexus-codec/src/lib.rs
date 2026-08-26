pub mod software;
mod types;

pub use software::SoftwareFallbackEncoder;
pub use types::{CodecError, CodecKind, EncodedFrame, EncoderConfig, VideoEncoder};

pub fn init() {
    // Initializer stub for nexus-codec
}
