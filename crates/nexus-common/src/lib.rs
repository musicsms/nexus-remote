//! nexus-common crate
//! Part of Nexus Remote Desktop Platform

pub mod id;
pub mod time;

pub use id::{ClientId, DeviceId, IdError, NodeId, SessionId, TenantId, UserId};
pub use time::{Clock, MockClock, SystemClock, UnixTimestamp};

pub fn init() {
    // Initializer stub for nexus-common
}
