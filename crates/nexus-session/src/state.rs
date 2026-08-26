use std::time::{Duration, Instant};
use thiserror::Error;

pub const DEFAULT_RECONNECT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SessionIdError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 {
            return Err(SessionIdError::InvalidLength);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionIdError {
    #[error("session ID must contain between 1 and 128 bytes")]
    InvalidLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub window: Duration,
}

impl ReconnectPolicy {
    pub fn new(window: Duration) -> Result<Self, SessionPolicyError> {
        if window.is_zero() {
            Err(SessionPolicyError::ZeroReconnectWindow)
        } else {
            Ok(Self { window })
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionPolicyError {
    #[error("reconnect window must be greater than zero")]
    ZeroReconnectWindow,
    #[error("session duration must be greater than zero")]
    ZeroSessionDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionDurationPolicy {
    pub max_duration: Duration,
}

impl SessionDurationPolicy {
    pub fn new(max_duration: Duration) -> Result<Self, SessionPolicyError> {
        if max_duration.is_zero() {
            Err(SessionPolicyError::ZeroSessionDuration)
        } else {
            Ok(Self { max_duration })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Requested,
    Authorized,
    Connecting,
    Established,
    Active,
    Disconnected,
    Ended,
    Denied,
    Expired,
    Failed,
    Revoked,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid session transition from {from:?} to {to:?}")]
pub struct InvalidTransition {
    pub from: SessionState,
    pub to: SessionState,
}

impl SessionState {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Requested,
                Self::Authorized | Self::Denied | Self::Expired | Self::Failed
            ) | (
                Self::Authorized,
                Self::Connecting | Self::Denied | Self::Expired | Self::Revoked
            ) | (
                Self::Connecting,
                Self::Established | Self::Failed | Self::Expired | Self::Revoked
            ) | (
                Self::Established,
                Self::Active | Self::Failed | Self::Revoked
            ) | (
                Self::Active,
                Self::Disconnected | Self::Revoked | Self::Ended | Self::Expired
            ) | (Self::Disconnected, Self::Connecting | Self::Ended)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStateMachine {
    state: SessionState,
    reconnect_deadline: Option<Instant>,
    established_at: Option<Instant>,
    max_duration: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    id: SessionId,
    machine: SessionStateMachine,
}

impl Session {
    pub fn new(id: SessionId) -> Self {
        Self {
            id,
            machine: SessionStateMachine::new(),
        }
    }
    pub fn id(&self) -> &SessionId {
        &self.id
    }
    pub fn state(&self) -> SessionState {
        self.machine.state()
    }
    pub fn transition(&mut self, next: SessionState) -> Result<(), InvalidTransition> {
        self.machine.transition(next)
    }
    pub fn mark_disconnected(
        &mut self,
        policy: ReconnectPolicy,
        now: Instant,
    ) -> Result<(), InvalidTransition> {
        self.machine.mark_disconnected(policy, now)
    }
    pub fn reconnect_allowed(&self, now: Instant) -> bool {
        self.machine.reconnect_allowed(now)
    }
    pub fn mark_established_at(
        &mut self,
        now: Instant,
        policy: SessionDurationPolicy,
    ) -> Result<(), InvalidTransition> {
        self.machine.mark_established_at(now, policy)
    }
    pub fn duration_expired(&self, now: Instant) -> bool {
        self.machine.duration_expired(now)
    }
}

impl SessionStateMachine {
    pub const fn new() -> Self {
        Self {
            state: SessionState::Requested,
            reconnect_deadline: None,
            established_at: None,
            max_duration: None,
        }
    }

    pub const fn state(self) -> SessionState {
        self.state
    }

    pub fn mark_disconnected(
        &mut self,
        policy: ReconnectPolicy,
        now: Instant,
    ) -> Result<(), InvalidTransition> {
        self.transition(SessionState::Disconnected)?;
        self.reconnect_deadline = Some(now + policy.window);
        Ok(())
    }

    pub fn reconnect_allowed(&self, now: Instant) -> bool {
        self.state == SessionState::Disconnected
            && self
                .reconnect_deadline
                .is_some_and(|deadline| now <= deadline)
    }

    pub fn mark_established_at(
        &mut self,
        now: Instant,
        policy: SessionDurationPolicy,
    ) -> Result<(), InvalidTransition> {
        self.transition(SessionState::Established)?;
        if self.established_at.is_none() {
            self.established_at = Some(now);
            self.max_duration = Some(policy.max_duration);
        }
        Ok(())
    }

    pub fn duration_expired(&self, now: Instant) -> bool {
        matches!(self.state, SessionState::Established | SessionState::Active)
            && self
                .established_at
                .zip(self.max_duration)
                .is_some_and(|(at, max)| now.saturating_duration_since(at) >= max)
    }

    pub fn transition(&mut self, next: SessionState) -> Result<(), InvalidTransition> {
        if next == self.state || self.state.can_transition_to(next) {
            self.state = next;
            if next != SessionState::Disconnected {
                self.reconnect_deadline = None;
            }
            Ok(())
        } else {
            Err(InvalidTransition {
                from: self.state,
                to: next,
            })
        }
    }
}

impl Default for SessionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_happy_path_and_allows_idempotent_transition() {
        let mut session = SessionStateMachine::new();
        for state in [
            SessionState::Authorized,
            SessionState::Connecting,
            SessionState::Established,
            SessionState::Active,
        ] {
            session.transition(state).unwrap();
        }
        assert!(session.transition(SessionState::Active).is_ok());
    }

    #[test]
    fn rejects_skip_and_reconnect_after_end() {
        let mut session = SessionStateMachine::new();
        assert!(session.transition(SessionState::Active).is_err());
        session.transition(SessionState::Denied).unwrap();
        assert!(session.transition(SessionState::Connecting).is_err());
    }

    #[test]
    fn reconnect_window_expires() {
        let now = Instant::now();
        let mut session = SessionStateMachine::new();
        for state in [
            SessionState::Authorized,
            SessionState::Connecting,
            SessionState::Established,
            SessionState::Active,
        ] {
            session.transition(state).unwrap();
        }
        session
            .mark_disconnected(ReconnectPolicy::new(Duration::from_secs(5)).unwrap(), now)
            .unwrap();
        assert!(session.reconnect_allowed(now + Duration::from_secs(5)));
        assert!(!session.reconnect_allowed(now + Duration::from_secs(6)));
    }

    #[test]
    fn session_id_is_non_empty_and_stable() {
        let id = SessionId::new("ses_01").unwrap();
        assert_eq!(id.as_str(), "ses_01");
        assert!(SessionId::new("").is_err());
        assert!(SessionId::new("x".repeat(129)).is_err());
    }

    #[test]
    fn established_duration_expires_at_boundary() {
        let now = Instant::now();
        let mut machine = SessionStateMachine::new();
        let policy = SessionDurationPolicy::new(Duration::from_secs(30)).unwrap();
        machine.transition(SessionState::Authorized).unwrap();
        machine.transition(SessionState::Connecting).unwrap();
        machine.mark_established_at(now, policy).unwrap();
        assert!(!machine.duration_expired(now + Duration::from_secs(29)));
        assert!(machine.duration_expired(now + Duration::from_secs(30)));
        machine.transition(SessionState::Active).unwrap();
        machine.transition(SessionState::Expired).unwrap();
        let mut revoked = SessionStateMachine::new();
        revoked.transition(SessionState::Authorized).unwrap();
        revoked.transition(SessionState::Revoked).unwrap();
    }

    #[test]
    fn duration_policy_rejects_zero() {
        assert_eq!(
            SessionDurationPolicy::new(Duration::ZERO),
            Err(SessionPolicyError::ZeroSessionDuration)
        );
    }

    #[test]
    fn reconnect_does_not_reset_established_duration() {
        let start = Instant::now();
        let policy = SessionDurationPolicy::new(Duration::from_secs(30)).unwrap();
        let mut machine = SessionStateMachine::new();
        machine.transition(SessionState::Authorized).unwrap();
        machine.transition(SessionState::Connecting).unwrap();
        machine.mark_established_at(start, policy).unwrap();
        machine.transition(SessionState::Active).unwrap();
        machine
            .mark_disconnected(
                ReconnectPolicy::new(Duration::from_secs(5)).unwrap(),
                start + Duration::from_secs(10),
            )
            .unwrap();
        machine.transition(SessionState::Connecting).unwrap();
        machine
            .mark_established_at(start + Duration::from_secs(20), policy)
            .unwrap();
        assert!(machine.duration_expired(start + Duration::from_secs(30)));
    }

    #[test]
    fn aggregate_keeps_id_across_reconnect() {
        let now = Instant::now();
        let id = SessionId::new("ses_01").unwrap();
        let mut session = Session::new(id.clone());
        for state in [
            SessionState::Authorized,
            SessionState::Connecting,
            SessionState::Established,
            SessionState::Active,
        ] {
            session.transition(state).unwrap();
        }
        session
            .mark_disconnected(ReconnectPolicy::new(Duration::from_secs(1)).unwrap(), now)
            .unwrap();
        session.transition(SessionState::Connecting).unwrap();
        assert_eq!(session.id(), &id);
    }
}
