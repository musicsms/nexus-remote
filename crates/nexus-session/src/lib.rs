//! Session lifecycle state machine and reconnect semantics.

mod state;

pub use state::{
    InvalidTransition, ReconnectPolicy, Session, SessionId, SessionIdError, SessionPolicyError,
    SessionState, SessionStateMachine, DEFAULT_RECONNECT_WINDOW,
};

pub fn init() {
    // Initializer stub for nexus-session
}
