//! nexus-audit crate
//! Part of Nexus Remote Desktop Platform

pub mod event;

pub use event::{
    AuditEvent, AuditEventBuildError, AuditEventBuilder, AuditEventType, EventParseError,
};

pub fn init() {
    // Initializer stub for nexus-audit
}
