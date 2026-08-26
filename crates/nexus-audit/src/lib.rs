//! nexus-audit crate
//! Part of Nexus Remote Desktop Platform

pub mod chain;
pub mod event;
pub mod sink;

pub use chain::{
    compute_event_hash, compute_hash, verify_chain, AuditChain, ChainError, ChainVerificationError,
    ChainedAuditEvent, GENESIS_HASH,
};
pub use event::{
    AuditEvent, AuditEventBuildError, AuditEventBuilder, AuditEventType, EventParseError,
};
pub use sink::{AuditSink, BroadcastAuditSink, MemoryAuditSink, SinkError};

pub fn init() {
    // Initializer stub for nexus-audit
}
