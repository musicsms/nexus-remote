//! nexus-protocol crate
//! Part of Nexus Remote Desktop Platform

pub mod proto;
pub mod video_packet;

pub use proto::{KeyEvent, MouseButton, MouseMove, MouseWheel, SessionHello, TextInput};
pub use video_packet::{VideoPacketError, VideoPacketHeader};

use thiserror::Error;

pub const CURRENT_PROTOCOL_VERSION: u32 = 1;
pub const MAX_SESSION_ID_LEN: usize = 128;
pub const MAX_DEVICE_ID_LEN: usize = 128;
pub const MAX_CAPABILITY_LEN: usize = 16 * 1024;
pub const MAX_EPHEMERAL_KEY_LEN: usize = 128;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionHelloError {
    #[error("unsupported protocol version: expected {expected}, got {actual}")]
    UnsupportedVersion { expected: u32, actual: u32 },
    #[error("session ID is empty or exceeds {max} bytes")]
    InvalidSessionId { max: usize },
    #[error("device ID is empty or exceeds {max} bytes")]
    InvalidDeviceId { max: usize },
    #[error("capability exceeds {max} bytes")]
    CapabilityTooLarge { max: usize },
    #[error("ephemeral public key exceeds {max} bytes")]
    EphemeralKeyTooLarge { max: usize },
}

impl SessionHello {
    pub fn validate(&self) -> Result<(), SessionHelloError> {
        if self.protocol_version != CURRENT_PROTOCOL_VERSION {
            return Err(SessionHelloError::UnsupportedVersion {
                expected: CURRENT_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        if self.session_id.is_empty() || self.session_id.len() > MAX_SESSION_ID_LEN {
            return Err(SessionHelloError::InvalidSessionId {
                max: MAX_SESSION_ID_LEN,
            });
        }
        if self.device_id.is_empty() || self.device_id.len() > MAX_DEVICE_ID_LEN {
            return Err(SessionHelloError::InvalidDeviceId {
                max: MAX_DEVICE_ID_LEN,
            });
        }
        if self.capability.len() > MAX_CAPABILITY_LEN {
            return Err(SessionHelloError::CapabilityTooLarge {
                max: MAX_CAPABILITY_LEN,
            });
        }
        if self.ephemeral_public_key.len() > MAX_EPHEMERAL_KEY_LEN {
            return Err(SessionHelloError::EphemeralKeyTooLarge {
                max: MAX_EPHEMERAL_KEY_LEN,
            });
        }
        Ok(())
    }
}

pub fn init() {
    // Initializer stub for nexus-protocol
}
