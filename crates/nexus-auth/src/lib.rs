//! Authentication, device enrollment, credential management, and capability verification.
//! Part of Nexus Remote Desktop Platform.

pub mod credential;
pub mod enrollment;
pub mod replay;
pub mod verifier;

pub use credential::{
    DeviceCredential, DeviceCredentialBuilder, DeviceCredentialError, DeviceRegistrationRequest,
};
pub use enrollment::{DeviceType, EnrollmentToken, EnrollmentTokenBuilder, EnrollmentTokenError};
pub use replay::NonceReplayCache;
pub use verifier::{CapabilityVerificationError, CapabilityVerifier};
