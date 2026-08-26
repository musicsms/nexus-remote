//! Application state, device registry, and token management for nexusd.
//! Part of Nexus Remote Desktop Platform.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use ed25519_dalek::SigningKey;
use nexus_audit::sink::{AuditSink, MemoryAuditSink};
use nexus_auth::credential::DeviceCredential;
use nexus_auth::enrollment::EnrollmentToken;
use nexus_common::id::{DeviceId, SessionId, TenantId};
use nexus_common::time::UnixTimestamp;
use nexus_policy::PolicyEngine;
use thiserror::Error;

/// Errors arising during Control Plane state operations.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
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

    #[error("session {0} not found")]
    SessionNotFound(SessionId),
}

/// Metadata stored in the control plane device registry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegisteredDevice {
    pub credential: DeviceCredential,
    pub hostname: String,
    pub agent_version: String,
    pub enrolled_at: UnixTimestamp,
    pub is_active: bool,
}

/// State tracking for active enrollment tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedEnrollmentToken {
    pub token: EnrollmentToken,
    pub uses_count: u32,
}

/// Central state container for the `nexusd` control plane.
#[derive(Clone)]
pub struct AppState {
    /// Control plane Ed25519 master signing key.
    pub signing_key: Arc<SigningKey>,
    /// Unique identifier for this control plane instance.
    pub control_plane_id: String,
    /// Default relay server identifier.
    pub default_relay_id: String,
    /// Registered devices mapped by DeviceId.
    devices: Arc<RwLock<HashMap<DeviceId, RegisteredDevice>>>,
    /// Active enrollment tokens mapped by token_id.
    enrollment_tokens: Arc<Mutex<HashMap<String, TrackedEnrollmentToken>>>,
    /// Policy evaluation engine.
    pub policy_engine: Arc<PolicyEngine>,
    /// Cryptographic hash chain for audit events.
    audit_chain: Arc<Mutex<nexus_audit::chain::AuditChain>>,
    /// Tamper-evident audit sink.
    pub audit_sink: Arc<dyn AuditSink>,
}

impl AppState {
    /// Creates a new `AppState` instance with default in-memory components.
    pub fn new(signing_key: SigningKey, control_plane_id: impl Into<String>) -> Self {
        let mut policy_engine = PolicyEngine::default();
        let admin_role = nexus_policy::Role::new("admin")
            .with_actions(nexus_policy::ActionSet::all())
            .with_conditions(nexus_policy::PolicyConditions::default());
        let operator_role = nexus_policy::Role::new("operator")
            .with_action(nexus_policy::Action::DesktopView)
            .with_action(nexus_policy::Action::DesktopControl)
            .with_conditions(nexus_policy::PolicyConditions::default());
        policy_engine.add_role(admin_role);
        policy_engine.add_role(operator_role);

        Self {
            signing_key: Arc::new(signing_key),
            control_plane_id: control_plane_id.into(),
            default_relay_id: "relay-nexus-primary".into(),
            devices: Arc::new(RwLock::new(HashMap::new())),
            enrollment_tokens: Arc::new(Mutex::new(HashMap::new())),
            policy_engine: Arc::new(policy_engine),
            audit_chain: Arc::new(Mutex::new(nexus_audit::chain::AuditChain::new(None))),
            audit_sink: Arc::new(MemoryAuditSink::new()),
        }
    }

    /// Appends and records an audit event to the hash chain and sink.
    pub async fn record_audit(&self, event: nexus_audit::event::AuditEvent) {
        let chained = {
            let mut chain = self.audit_chain.lock().unwrap();
            chain.append(event).expect("audit chain append cannot fail")
        };
        let _ = self.audit_sink.record(&chained).await;
    }

    /// Builder method to override default relay identifier.
    pub fn with_default_relay_id(mut self, relay_id: impl Into<String>) -> Self {
        self.default_relay_id = relay_id.into();
        self
    }

    /// Builder method to override audit sink.
    pub fn with_audit_sink(mut self, audit_sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = audit_sink;
        self
    }

    /// Stores a new pre-signed enrollment token.
    pub fn store_enrollment_token(&self, token: EnrollmentToken) {
        let mut tokens = self.enrollment_tokens.lock().unwrap();
        tokens.insert(
            token.token_id.clone(),
            TrackedEnrollmentToken {
                token,
                uses_count: 0,
            },
        );
    }

    /// Validates and consumes one use of an enrollment token.
    pub fn consume_enrollment_token(
        &self,
        token_id: &str,
        now: UnixTimestamp,
    ) -> Result<EnrollmentToken, StateError> {
        let mut tokens = self.enrollment_tokens.lock().unwrap();
        let tracked = tokens
            .get_mut(token_id)
            .ok_or_else(|| StateError::EnrollmentTokenNotFound(token_id.to_string()))?;

        if now > tracked.token.expires_at {
            return Err(StateError::EnrollmentTokenExpired {
                expired_at: tracked.token.expires_at,
                current_time: now,
            });
        }

        if tracked.uses_count >= tracked.token.max_uses {
            return Err(StateError::EnrollmentTokenExhausted(
                token_id.to_string(),
                tracked.uses_count,
                tracked.token.max_uses,
            ));
        }

        tracked.uses_count += 1;
        Ok(tracked.token.clone())
    }

    /// Registers a newly enrolled device.
    pub fn register_device(
        &self,
        credential: DeviceCredential,
        hostname: String,
        agent_version: String,
        enrolled_at: UnixTimestamp,
    ) {
        let mut map = self.devices.write().unwrap();
        let device_id = credential.device_id.clone();
        map.insert(
            device_id,
            RegisteredDevice {
                credential,
                hostname,
                agent_version,
                enrolled_at,
                is_active: true,
            },
        );
    }

    /// Retrieves registered device details.
    pub fn get_device(&self, device_id: &DeviceId) -> Option<RegisteredDevice> {
        self.devices.read().unwrap().get(device_id).cloned()
    }

    /// Lists registered devices belonging to a tenant organization.
    pub fn list_devices(&self, org_id: &TenantId) -> Vec<RegisteredDevice> {
        self.devices
            .read()
            .unwrap()
            .values()
            .filter(|d| &d.credential.organization_id == org_id)
            .cloned()
            .collect()
    }
}
