//! nexus-policy crate
//! Part of Nexus Remote Desktop Platform

pub mod action;
pub mod engine;
pub mod model;
pub mod narrowing;

pub use action::{Action, ActionParseError, ActionSet};
pub use engine::{
    ip_matches_range, ActiveSessionSummary, DenialReason, EvaluationDecision, PolicyEngine,
    ResourceContext, SubjectContext,
};
pub use model::{
    DeviceLabelSelector, PolicyConditions, Role, SessionRestrictions, DEFAULT_MAX_DURATION_SECONDS,
};
pub use narrowing::{validate_policy_narrowing, NarrowingError};

pub fn init() {
    // Initializer stub for nexus-policy
}
