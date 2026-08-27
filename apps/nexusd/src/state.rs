//! Application state and durable control-plane operations for nexusd.

use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use nexus_audit::sink::{AuditSink, MemoryAuditSink};
use nexus_auth::credential::DeviceCredential;
use nexus_auth::enrollment::EnrollmentToken;
use nexus_common::id::{DeviceId, SessionId, TenantId};
use nexus_common::time::UnixTimestamp;
use nexus_policy::PolicyEngine;
use thiserror::Error;

use crate::storage::{AuthorizedSessionRecord, EnrollmentError, SqliteStorage, StorageError};

#[derive(Debug, Error)]
pub enum StateError {
    #[error("device {0} not found")]
    DeviceNotFound(DeviceId),
    #[error("enrollment token {0} not found")]
    EnrollmentTokenNotFound(String),
    #[error("enrollment token {0} has exceeded maximum uses ({1}/{2})")]
    EnrollmentTokenExhausted(String, u32, u32),
    #[error("enrollment token expired at {expired_at}, current time {current_time}")]
    EnrollmentTokenExpired {
        expired_at: UnixTimestamp,
        current_time: UnixTimestamp,
    },
    #[error("enrollment token not active until {not_before}, current time {current_time}")]
    EnrollmentTokenNotYetActive {
        not_before: UnixTimestamp,
        current_time: UnixTimestamp,
    },
    #[error("session {0} not found")]
    SessionNotFound(SessionId),
    #[error("control-plane storage failure: {0}")]
    Storage(#[from] StorageError),
    #[error("audit sink failure: {0}")]
    AuditSink(String),
    #[error("audit chain failure: {0}")]
    AuditChain(String),
}

impl From<EnrollmentError> for StateError {
    fn from(error: EnrollmentError) -> Self {
        match error {
            EnrollmentError::NotFound { token_id } => Self::EnrollmentTokenNotFound(token_id),
            EnrollmentError::Expired {
                expired_at,
                current_time,
            } => Self::EnrollmentTokenExpired {
                expired_at,
                current_time,
            },
            EnrollmentError::NotYetActive {
                not_before,
                current_time,
            } => Self::EnrollmentTokenNotYetActive {
                not_before,
                current_time,
            },
            EnrollmentError::Exhausted {
                token_id,
                uses_count,
                max_uses,
            } => Self::EnrollmentTokenExhausted(token_id, uses_count, max_uses),
            EnrollmentError::Storage(error) => Self::Storage(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisteredDevice {
    pub credential: DeviceCredential,
    pub hostname: String,
    pub agent_version: String,
    pub enrolled_at: UnixTimestamp,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedEnrollmentToken {
    pub token: EnrollmentToken,
    pub uses_count: u32,
}

#[derive(Clone)]
pub struct AppState {
    pub signing_key: Arc<SigningKey>,
    pub control_plane_id: String,
    pub default_relay_id: String,
    pub storage: SqliteStorage,
    pub policy_engine: Arc<PolicyEngine>,
    audit_chain: Arc<Mutex<nexus_audit::chain::AuditChain>>,
    pub audit_sink: Arc<dyn AuditSink>,
}

impl AppState {
    pub fn new(
        signing_key: SigningKey,
        control_plane_id: impl Into<String>,
        storage: SqliteStorage,
    ) -> Self {
        let mut policy_engine = PolicyEngine::default();
        policy_engine.add_role(
            nexus_policy::Role::new("admin")
                .with_actions(nexus_policy::ActionSet::all())
                .with_conditions(nexus_policy::PolicyConditions::default()),
        );
        policy_engine.add_role(
            nexus_policy::Role::new("operator")
                .with_action(nexus_policy::Action::DesktopView)
                .with_action(nexus_policy::Action::DesktopControl)
                .with_conditions(nexus_policy::PolicyConditions::default()),
        );
        Self {
            signing_key: Arc::new(signing_key),
            control_plane_id: control_plane_id.into(),
            default_relay_id: "relay-nexus-primary".into(),
            storage,
            policy_engine: Arc::new(policy_engine),
            audit_chain: Arc::new(Mutex::new(nexus_audit::chain::AuditChain::new(None))),
            audit_sink: Arc::new(MemoryAuditSink::new()),
        }
    }

    pub async fn store_enrollment_token(&self, token: EnrollmentToken) -> Result<(), StateError> {
        self.storage.store_enrollment_token(&token).await?;
        Ok(())
    }

    pub async fn enroll_device(
        &self,
        token_id: &str,
        now: UnixTimestamp,
        device: RegisteredDevice,
    ) -> Result<EnrollmentToken, StateError> {
        Ok(self.storage.enroll_device(token_id, now, device).await?)
    }

    pub async fn get_device(
        &self,
        device_id: &DeviceId,
    ) -> Result<Option<RegisteredDevice>, StateError> {
        Ok(self.storage.get_device(device_id).await?)
    }

    pub async fn list_devices(
        &self,
        org_id: &TenantId,
    ) -> Result<Vec<RegisteredDevice>, StateError> {
        Ok(self.storage.list_devices(org_id).await?)
    }

    pub async fn persist_authorized_session(
        &self,
        record: &AuthorizedSessionRecord,
    ) -> Result<(), StateError> {
        self.storage.insert_authorized_session(record).await?;
        Ok(())
    }

    pub async fn record_audit(
        &self,
        event: nexus_audit::event::AuditEvent,
    ) -> Result<(), StateError> {
        let (previous_chain, chained) = {
            let mut chain = self.audit_chain.lock().unwrap();
            let previous = chain.clone();
            let chained = chain
                .append(event)
                .map_err(|e| StateError::AuditChain(e.to_string()))?;
            (previous, chained)
        };
        if let Err(error) = self.storage.insert_audit_event(&chained).await {
            *self.audit_chain.lock().unwrap() = previous_chain;
            return Err(error.into());
        }
        self.audit_sink
            .record(&chained)
            .await
            .map_err(|e| StateError::AuditSink(e.to_string()))
    }

    pub fn with_default_relay_id(mut self, relay_id: impl Into<String>) -> Self {
        self.default_relay_id = relay_id.into();
        self
    }

    pub fn with_audit_sink(mut self, audit_sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = audit_sink;
        self
    }
}
