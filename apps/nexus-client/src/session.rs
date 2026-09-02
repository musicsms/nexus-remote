use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use nexus_common::{Clock, UnixTimestamp};
use nexus_protocol::SessionCapability;
use nexus_session::{ReconnectPolicy, SessionDurationPolicy};
use std::time::Duration;
use thiserror::Error;

pub const DEFAULT_MAX_SESSION_DURATION: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Expired,
}

/// Relay claims needed by the client. Signature and private key material are
/// deliberately not part of this portable lifecycle type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayTokenMetadata {
    pub relay_id: String,
    pub session_id: String,
    pub client_device_id: String,
    pub target_device_id: String,
    pub expires_at: UnixTimestamp,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ClientVerification {
    pub capability_key: VerifyingKey,
    pub relay_key: VerifyingKey,
    pub relay_id: String,
}

#[derive(Debug, Clone, Copy)]
pub struct SessionPolicy {
    pub max_duration: Duration,
    pub reconnect_window: Duration,
}

impl SessionPolicy {
    pub fn new(
        max_duration: Duration,
        reconnect_window: Duration,
    ) -> Result<Self, nexus_session::SessionPolicyError> {
        SessionDurationPolicy::new(max_duration)?;
        ReconnectPolicy::new(reconnect_window)?;
        Ok(Self {
            max_duration,
            reconnect_window,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClientError {
    #[error("client session has expired")]
    Expired,
    #[error("invalid client transition from {from:?} to {to:?}")]
    InvalidTransition { from: ClientState, to: ClientState },
    #[error("capability and relay token identities do not match")]
    IdentityMismatch,
    #[error("capability is not active yet")]
    CapabilityNotActive,
    #[error("capability has expired")]
    CapabilityExpired,
    #[error("relay token has expired")]
    RelayTokenExpired,
    #[error("reconnect window has elapsed")]
    ReconnectWindowElapsed,
    #[error("capability signature is invalid")]
    InvalidCapabilitySignature,
    #[error("relay token signature is invalid")]
    InvalidRelaySignature,
}

pub struct ClientSession {
    capability: SessionCapability,
    relay_token: RelayTokenMetadata,
    clock: Box<dyn Clock>,
    state: ClientState,
    reconnect_deadline: Option<UnixTimestamp>,
    established_at: Option<UnixTimestamp>,
    duration_policy: SessionDurationPolicy,
    reconnect_policy: ReconnectPolicy,
    verification: ClientVerification,
}

impl ClientSession {
    pub fn new<C: Clock + 'static>(
        capability: SessionCapability,
        relay_token: RelayTokenMetadata,
        clock: C,
        policy: SessionPolicy,
        verification: ClientVerification,
    ) -> Self {
        Self {
            capability,
            relay_token,
            clock: Box::new(clock),
            state: ClientState::Disconnected,
            reconnect_deadline: None,
            established_at: None,
            duration_policy: SessionDurationPolicy::new(policy.max_duration)
                .expect("validated policy"),
            reconnect_policy: ReconnectPolicy::new(policy.reconnect_window)
                .expect("validated policy"),
            verification,
        }
    }

    pub fn state(&self) -> ClientState {
        self.state
    }

    pub fn session_id(&self) -> &str {
        &self.capability.session_id
    }

    /// Revalidates the signed claims and established-duration policy while a
    /// connected runtime is pumping network/UI work.
    pub fn ensure_active(&mut self, now: UnixTimestamp) -> Result<(), ClientError> {
        if self.state == ClientState::Expired {
            return Err(ClientError::Expired);
        }
        if self.state != ClientState::Connected {
            return Err(ClientError::InvalidTransition {
                from: self.state,
                to: ClientState::Connected,
            });
        }
        self.validate_claims(now)?;
        if self.session_duration_expired(now) {
            self.state = ClientState::Expired;
            self.reconnect_deadline = None;
            return Err(ClientError::Expired);
        }
        Ok(())
    }

    pub fn begin_connect(&mut self, now: UnixTimestamp) -> Result<(), ClientError> {
        if self.state == ClientState::Expired {
            return Err(ClientError::Expired);
        }
        if self.state == ClientState::Reconnecting {
            if !self.can_reconnect(now) {
                self.state = ClientState::Expired;
                return Err(ClientError::ReconnectWindowElapsed);
            }
        } else if self.state != ClientState::Disconnected {
            return Err(ClientError::InvalidTransition {
                from: self.state,
                to: ClientState::Connecting,
            });
        }
        self.validate_claims(now)?;
        self.state = ClientState::Connecting;
        Ok(())
    }

    pub fn connected(&mut self, now: UnixTimestamp) -> Result<(), ClientError> {
        if self.state == ClientState::Expired {
            return Err(ClientError::Expired);
        }
        if !matches!(self.state, ClientState::Connecting) {
            return Err(ClientError::InvalidTransition {
                from: self.state,
                to: ClientState::Connected,
            });
        }
        if self.established_at.is_some()
            && self
                .reconnect_deadline
                .is_some_and(|deadline| now > deadline)
        {
            self.state = ClientState::Expired;
            return Err(ClientError::ReconnectWindowElapsed);
        }
        self.validate_claims(now)?;
        if self
            .established_at
            .is_some_and(|_| self.session_duration_expired(now))
        {
            self.state = ClientState::Expired;
            return Err(ClientError::Expired);
        }
        if self.established_at.is_none() {
            self.established_at = Some(now);
        }
        self.reconnect_deadline = None;
        self.state = ClientState::Connected;
        Ok(())
    }

    pub fn transport_lost(&mut self, now: UnixTimestamp) -> Result<(), ClientError> {
        if self.state == ClientState::Expired {
            return Err(ClientError::Expired);
        }
        if self.state != ClientState::Connected {
            return Err(ClientError::InvalidTransition {
                from: self.state,
                to: ClientState::Reconnecting,
            });
        }
        if self.session_duration_expired(now) {
            self.state = ClientState::Expired;
            return Err(ClientError::Expired);
        }
        self.reconnect_deadline = Some(now.saturating_add(self.reconnect_policy.window));
        self.state = ClientState::Reconnecting;
        Ok(())
    }

    pub fn reconnect_deadline(&self) -> Option<UnixTimestamp> {
        self.reconnect_deadline
    }

    pub fn expire(&mut self) -> Result<(), ClientError> {
        if self.state == ClientState::Expired {
            return Err(ClientError::Expired);
        }
        self.state = ClientState::Expired;
        self.reconnect_deadline = None;
        Ok(())
    }

    pub fn can_reconnect(&self, now: UnixTimestamp) -> bool {
        self.state == ClientState::Reconnecting
            && self
                .reconnect_deadline
                .is_some_and(|deadline| now <= deadline)
            && !self.session_duration_expired(now)
    }

    pub fn session_duration_expired(&self, now: UnixTimestamp) -> bool {
        self.established_at.is_some_and(|at| {
            now.saturating_duration_since(at) >= self.duration_policy.max_duration
        })
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    fn validate_claims(&self, now: UnixTimestamp) -> Result<(), ClientError> {
        self.validate_identity()?;
        self.capability
            .validate()
            .map_err(|_| ClientError::IdentityMismatch)?;
        if now.as_secs() < self.capability.not_before {
            return Err(ClientError::CapabilityNotActive);
        }
        if now.as_secs() >= self.capability.expires_at {
            return Err(ClientError::CapabilityExpired);
        }
        if now >= self.relay_token.expires_at {
            return Err(ClientError::RelayTokenExpired);
        }
        let signature: [u8; 64] = self
            .capability
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| ClientError::InvalidCapabilitySignature)?;
        self.verification
            .capability_key
            .verify(
                &self.capability.signing_bytes(),
                &Signature::from_bytes(&signature),
            )
            .map_err(|_| ClientError::InvalidCapabilitySignature)?;
        if self.relay_token.relay_id != self.verification.relay_id
            || self.relay_token.signature.len() != 64
        {
            return Err(ClientError::InvalidRelaySignature);
        }
        let signature: [u8; 64] = self
            .relay_token
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| ClientError::InvalidRelaySignature)?;
        self.verification
            .relay_key
            .verify(
                &self.relay_token.signing_bytes(),
                &Signature::from_bytes(&signature),
            )
            .map_err(|_| ClientError::InvalidRelaySignature)?;
        Ok(())
    }

    fn validate_identity(&self) -> Result<(), ClientError> {
        if self.capability.session_id != self.relay_token.session_id
            || self.capability.client_device_id != self.relay_token.client_device_id
            || self.capability.target_device_id != self.relay_token.target_device_id
        {
            return Err(ClientError::IdentityMismatch);
        }
        Ok(())
    }
}

impl RelayTokenMetadata {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = b"nexus-relay-token/v1\0".to_vec();
        for value in [
            &self.session_id,
            &self.relay_id,
            &self.client_device_id,
            &self.target_device_id,
        ] {
            out.extend_from_slice(&(value.len() as u16).to_be_bytes());
            out.extend_from_slice(value.as_bytes());
        }
        out.push(0); // client endpoint role
        out.extend_from_slice(&self.expires_at.as_secs().to_be_bytes());
        out
    }
}
