//! Session listener, capability verification, and worker process lifecycle manager.
//! Part of Nexus Remote Desktop Platform.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ed25519_dalek::VerifyingKey;
use nexus_auth::verifier::{CapabilityVerificationError, CapabilityVerifier};
use nexus_common::id::SessionId;
use nexus_common::time::{Clock, SystemClock, UnixTimestamp};
use nexus_protocol::SessionCapability;
use nexus_session::SessionState;
use thiserror::Error;

/// Errors arising during session handling on the host agent.
#[derive(Debug, Error)]
pub enum SessionHandlerError {
    #[error("Capability verification error: {0}")]
    Capability(#[from] CapabilityVerificationError),

    #[error("Session already active with ID: {0}")]
    SessionAlreadyActive(SessionId),

    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    #[error("Protocol error: {0}")]
    Protocol(String),
}

/// Active session instance tracked by the host agent.
#[derive(Debug, Clone)]
pub struct HostSession {
    pub session_id: SessionId,
    pub capability: SessionCapability,
    pub negotiated_protocol: u32,
    pub established_at: UnixTimestamp,
    pub state: SessionState,
}

/// Core agent session coordinator that verifies capabilities and manages worker sessions.
pub struct AgentSessionManager {
    control_plane_verifying_key: VerifyingKey,
    target_device_id: String,
    verifier: CapabilityVerifier,
    active_sessions: Arc<Mutex<HashMap<SessionId, HostSession>>>,
}

impl AgentSessionManager {
    /// Creates a new `AgentSessionManager` bound to the host device ID.
    pub fn new(
        control_plane_verifying_key: VerifyingKey,
        target_device_id: impl Into<String>,
    ) -> Self {
        let target_device_id = target_device_id.into();
        let verifier = CapabilityVerifier::new(
            control_plane_verifying_key,
            Duration::from_secs(120),
            10_000,
        )
        .with_target_device_id(target_device_id.clone());

        Self {
            control_plane_verifying_key,
            target_device_id,
            verifier,
            active_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns the control plane verifying key.
    pub fn control_plane_verifying_key(&self) -> &VerifyingKey {
        &self.control_plane_verifying_key
    }

    /// Returns the target device ID.
    pub fn target_device_id(&self) -> &str {
        &self.target_device_id
    }

    /// Validates an incoming [`SessionCapability`] from a connecting client.
    pub fn verify_and_accept_session(
        &mut self,
        capability: &SessionCapability,
        negotiated_protocol: u32,
    ) -> Result<HostSession, SessionHandlerError> {
        let now = SystemClock.now();
        let inst_now = Instant::now();

        // 1. Verify capability cryptographic signature, TTL window, protocol pinning, and replay
        self.verifier
            .verify(capability, negotiated_protocol, now, inst_now)?;

        let session_id = SessionId::new(capability.session_id.clone())
            .map_err(|e| SessionHandlerError::Protocol(format!("invalid session id: {e}")))?;

        // 2. Register active session
        let mut sessions = self.active_sessions.lock().unwrap();
        if sessions.contains_key(&session_id) {
            return Err(SessionHandlerError::SessionAlreadyActive(session_id));
        }

        let host_session = HostSession {
            session_id: session_id.clone(),
            capability: capability.clone(),
            negotiated_protocol,
            established_at: now,
            state: SessionState::Active,
        };

        sessions.insert(session_id, host_session.clone());
        Ok(host_session)
    }

    /// Terminates and removes an active session.
    pub fn terminate_session(
        &self,
        session_id: &SessionId,
    ) -> Result<HostSession, SessionHandlerError> {
        let mut sessions = self.active_sessions.lock().unwrap();
        sessions
            .remove(session_id)
            .ok_or_else(|| SessionHandlerError::SessionNotFound(session_id.clone()))
    }

    /// Returns the number of currently active sessions.
    pub fn active_session_count(&self) -> usize {
        self.active_sessions.lock().unwrap().len()
    }
}
