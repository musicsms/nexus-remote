use nexus_common::{Clock, UnixTimestamp};
use nexus_protocol::SessionCapability;
use nexus_session::{ReconnectPolicy, SessionDurationPolicy, DEFAULT_RECONNECT_WINDOW};
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
    pub session_id: String,
    pub client_device_id: String,
    pub target_device_id: String,
    pub expires_at: UnixTimestamp,
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
}

impl ClientSession {
    pub fn new<C: Clock + 'static>(
        capability: SessionCapability,
        relay_token: RelayTokenMetadata,
        clock: C,
    ) -> Self {
        Self {
            capability,
            relay_token,
            clock: Box::new(clock),
            state: ClientState::Disconnected,
            reconnect_deadline: None,
            established_at: None,
            duration_policy: SessionDurationPolicy::new(DEFAULT_MAX_SESSION_DURATION)
                .expect("constant duration is non-zero"),
            reconnect_policy: ReconnectPolicy::new(DEFAULT_RECONNECT_WINDOW)
                .expect("constant reconnect window is non-zero"),
        }
    }

    pub fn state(&self) -> ClientState {
        self.state
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
        if self.state == ClientState::Disconnected {
            self.validate_claims(now)?;
        } else {
            self.validate_identity()?;
        }
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
        self.validate_identity()?;
        if self.established_at.is_none() {
            if now.as_secs() < self.capability.not_before {
                return Err(ClientError::CapabilityNotActive);
            }
            if now.as_secs() >= self.capability.expires_at || now >= self.relay_token.expires_at {
                return Err(ClientError::CapabilityExpired);
            }
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
