//! nexus-protocol crate
//! Part of Nexus Remote Desktop Platform

pub mod proto;
pub mod video_packet;

pub use proto::{
    CursorPosition, CursorShape, KeyEvent, MonitorInfo, MouseButton, MouseMove, MouseWheel,
    SessionCapability, SessionHello, TextInput,
};
pub use video_packet::{VideoPacketError, VideoPacketHeader};

use thiserror::Error;

pub const CURRENT_PROTOCOL_VERSION: u32 = 1;
pub const MAX_SESSION_ID_LEN: usize = 128;
pub const MAX_DEVICE_ID_LEN: usize = 128;
pub const MAX_CAPABILITY_LEN: usize = 16 * 1024;
pub const MAX_EPHEMERAL_KEY_LEN: usize = 128;
pub const MAX_CAPABILITY_PERMISSIONS: usize = 32;
pub const MAX_NONCE_LEN: usize = 64;

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

#[derive(Debug, Error, PartialEq)]
pub enum MonitorInfoError {
    #[error("monitor dimensions must be non-zero")]
    InvalidDimensions,
    #[error("monitor scale must be finite and greater than zero")]
    InvalidScale,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CursorShapeError {
    #[error("cursor dimensions must be non-zero")]
    InvalidDimensions,
    #[error("cursor hotspot is outside the cursor bounds")]
    InvalidHotspot,
    #[error("cursor payload exceeds {max} bytes")]
    DataTooLarge { max: usize },
}

impl CursorShape {
    pub const MAX_DATA_LEN: usize = 1024 * 1024;

    pub fn validate(&self) -> Result<(), CursorShapeError> {
        if self.width == 0 || self.height == 0 {
            return Err(CursorShapeError::InvalidDimensions);
        }
        if self.hotspot_x >= self.width || self.hotspot_y >= self.height {
            return Err(CursorShapeError::InvalidHotspot);
        }
        if self.data.len() > Self::MAX_DATA_LEN {
            return Err(CursorShapeError::DataTooLarge {
                max: Self::MAX_DATA_LEN,
            });
        }
        Ok(())
    }
}

impl MonitorInfo {
    pub fn validate(&self) -> Result<(), MonitorInfoError> {
        if self.width == 0 || self.height == 0 {
            return Err(MonitorInfoError::InvalidDimensions);
        }
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return Err(MonitorInfoError::InvalidScale);
        }
        Ok(())
    }
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionCapabilityError {
    #[error("capability version must be 1")]
    UnsupportedVersion,
    #[error("capability identity field is empty or too long")]
    InvalidIdentity,
    #[error("capability must have not_before before expires_at")]
    InvalidValidityWindow,
    #[error("capability has too many permissions")]
    TooManyPermissions,
    #[error("capability nonce is empty or too long")]
    InvalidNonce,
    #[error("agent protocol range is invalid")]
    InvalidProtocolRange,
}

impl SessionCapability {
    pub fn validate(&self) -> Result<(), SessionCapabilityError> {
        if self.version != 1 {
            return Err(SessionCapabilityError::UnsupportedVersion);
        }
        for value in [
            &self.issuer,
            &self.session_id,
            &self.subject_user_id,
            &self.client_device_id,
            &self.target_device_id,
        ] {
            if value.is_empty() || value.len() > MAX_SESSION_ID_LEN {
                return Err(SessionCapabilityError::InvalidIdentity);
            }
        }
        if self.not_before >= self.expires_at {
            return Err(SessionCapabilityError::InvalidValidityWindow);
        }
        if self.permissions.is_empty() || self.permissions.len() > MAX_CAPABILITY_PERMISSIONS {
            return Err(SessionCapabilityError::TooManyPermissions);
        }
        if self.nonce.is_empty() || self.nonce.len() > MAX_NONCE_LEN {
            return Err(SessionCapabilityError::InvalidNonce);
        }
        if self.agent_min_protocol > self.agent_max_protocol {
            return Err(SessionCapabilityError::InvalidProtocolRange);
        }
        Ok(())
    }
}

pub fn init() {
    // Initializer stub for nexus-protocol
}
