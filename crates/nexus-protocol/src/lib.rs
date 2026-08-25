//! nexus-protocol crate
//! Part of Nexus Remote Desktop Platform

pub mod proto;
pub mod video_packet;

pub use proto::{KeyEvent, MouseButton, MouseMove, MouseWheel, SessionHello, TextInput};
pub use video_packet::{VideoPacketError, VideoPacketHeader};

pub fn init() {
    // Initializer stub for nexus-protocol
}
