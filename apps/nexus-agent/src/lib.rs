//! nexus-agent host daemon library.
//! Part of Nexus Remote Desktop Platform.

pub mod enroll;
pub mod identity;
pub mod session_manager;

pub use enroll::{EnrollmentClient, EnrollmentClientError};
pub use identity::{AgentIdentity, IdentityError};
pub use session_manager::{AgentSessionManager, HostSession, SessionHandlerError};
