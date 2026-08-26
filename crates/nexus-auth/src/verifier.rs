//! SessionCapability verification, cryptographic validation, and replay defense.
//! Part of Nexus Remote Desktop Platform.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use nexus_common::time::UnixTimestamp;
use nexus_protocol::SessionCapability;
use prost::Message;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::replay::NonceReplayCache;

/// Errors arising during SessionCapability verification.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum CapabilityVerificationError {
    /// Capability has expired prior to session establishment.
    #[error("capability establishment window expired: expired at {expired_at}, current time {current_time}")]
    Expired {
        expired_at: UnixTimestamp,
        current_time: UnixTimestamp,
    },

    /// Capability is not valid yet.
    #[error("capability not yet active: not before {not_before}, current time {current_time}")]
    NotYetActive {
        not_before: UnixTimestamp,
        current_time: UnixTimestamp,
    },

    /// Cryptographic signature verification failed.
    #[error("invalid capability signature: {0}")]
    InvalidSignature(String),

    /// Nonce replay detected or replay cache capacity saturated.
    #[error("capability nonce rejected: replay detected or cache saturated")]
    ReplayDetected,

    /// Negotiated protocol version falls outside signed agent protocol range (ADR-016).
    #[error("protocol version {negotiated} outside signed range [{min}, {max}]")]
    ProtocolRangeViolation { negotiated: u32, min: u32, max: u32 },

    /// Target device mismatch.
    #[error("target device mismatch: expected {expected}, got {actual}")]
    TargetDeviceMismatch { expected: String, actual: String },

    /// Client device mismatch.
    #[error("client device mismatch: expected {expected}, got {actual}")]
    ClientDeviceMismatch { expected: String, actual: String },

    /// Validation error from nexus-protocol schema.
    #[error("protocol validation error: {0}")]
    ProtocolValidation(String),
}

/// Verifier that enforces cryptographic integrity, protocol pinning (ADR-016),
/// TTL establishment windows (ADR-014), and replay prevention for [`SessionCapability`].
pub struct CapabilityVerifier {
    control_plane_verifying_key: VerifyingKey,
    replay_cache: NonceReplayCache,
    target_device_id: Option<String>,
}

impl CapabilityVerifier {
    /// Creates a new verifier with the given control plane public key and replay cache configuration.
    pub fn new(
        control_plane_verifying_key: VerifyingKey,
        replay_ttl: Duration,
        replay_capacity: usize,
    ) -> Self {
        Self {
            control_plane_verifying_key,
            replay_cache: NonceReplayCache::new(replay_ttl, replay_capacity),
            target_device_id: None,
        }
    }

    /// Sets expected target device ID to enforce local host binding.
    pub fn with_target_device_id(mut self, target_device_id: impl Into<String>) -> Self {
        self.target_device_id = Some(target_device_id.into());
        self
    }

    /// Verifies a [`SessionCapability`] against local device constraints and cryptographic signature.
    pub fn verify(
        &mut self,
        capability: &SessionCapability,
        negotiated_protocol: u32,
        now: UnixTimestamp,
        instant_now: Instant,
    ) -> Result<(), CapabilityVerificationError> {
        // 1. Basic schema validation from protobuf definition
        capability
            .validate()
            .map_err(|e| CapabilityVerificationError::ProtocolValidation(e.to_string()))?;

        // 2. Enforce local target device ID binding if configured
        if let Some(expected_target) = &self.target_device_id {
            if &capability.target_device_id != expected_target {
                return Err(CapabilityVerificationError::TargetDeviceMismatch {
                    expected: expected_target.clone(),
                    actual: capability.target_device_id.clone(),
                });
            }
        }

        // 3. ADR-014: Capability TTL governs establishment window
        if now.as_secs() < capability.not_before {
            return Err(CapabilityVerificationError::NotYetActive {
                not_before: UnixTimestamp::from_secs(capability.not_before),
                current_time: now,
            });
        }
        if now.as_secs() > capability.expires_at {
            return Err(CapabilityVerificationError::Expired {
                expired_at: UnixTimestamp::from_secs(capability.expires_at),
                current_time: now,
            });
        }

        // 4. ADR-016: Negotiated protocol version must be inside signed range
        if negotiated_protocol < capability.agent_min_protocol
            || negotiated_protocol > capability.agent_max_protocol
        {
            return Err(CapabilityVerificationError::ProtocolRangeViolation {
                negotiated: negotiated_protocol,
                min: capability.agent_min_protocol,
                max: capability.agent_max_protocol,
            });
        }

        // 5. Nonce replay defense
        if !self.replay_cache.accept(&capability.nonce, instant_now) {
            return Err(CapabilityVerificationError::ReplayDetected);
        }

        // 6. Cryptographic signature verification over unsigned capability fields
        let mut unsigned_cap = capability.clone();
        let sig_bytes = std::mem::take(&mut unsigned_cap.signature);

        let mut raw_bytes = Vec::with_capacity(unsigned_cap.encoded_len());
        unsigned_cap
            .encode(&mut raw_bytes)
            .map_err(|e| CapabilityVerificationError::ProtocolValidation(e.to_string()))?;

        let sig_array: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
            CapabilityVerificationError::InvalidSignature("signature must be 64 bytes".into())
        })?;
        let signature = Signature::from_bytes(&sig_array);

        self.control_plane_verifying_key
            .verify(&raw_bytes, &signature)
            .map_err(|e| CapabilityVerificationError::InvalidSignature(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn create_test_capability(
        signing_key: &SigningKey,
        target_device: &str,
        expires_at: u64,
        nonce: Vec<u8>,
    ) -> SessionCapability {
        let mut cap = SessionCapability {
            version: 1,
            issuer: "control-plane-1".into(),
            session_id: "sess-test-1".into(),
            subject_user_id: "user-alice".into(),
            client_device_id: "dev-client-1".into(),
            target_device_id: target_device.into(),
            permissions: vec!["desktop.view".into(), "desktop.control".into()],
            restrictions: vec![],
            not_before: 100,
            expires_at,
            nonce,
            agent_min_protocol: 1,
            agent_max_protocol: 2,
            client_ephemeral_public_key: vec![1u8; 32],
            signature: vec![],
        };

        let mut raw_bytes = Vec::new();
        cap.encode(&mut raw_bytes).unwrap();
        let sig = signing_key.sign(&raw_bytes);
        cap.signature = sig.to_bytes().to_vec();
        cap
    }

    #[test]
    fn test_capability_verifier_happy_path() {
        let signing_key = SigningKey::from_bytes(&[10u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let mut verifier = CapabilityVerifier::new(verifying_key, Duration::from_secs(60), 100)
            .with_target_device_id("dev-host-target");

        let cap = create_test_capability(&signing_key, "dev-host-target", 500, vec![1, 2, 3, 4]);

        let now = UnixTimestamp::from_secs(200);
        let inst = Instant::now();

        assert!(verifier.verify(&cap, 1, now, inst).is_ok());

        // Replay attempt with same nonce fails
        let replay_err = verifier
            .verify(&cap, 1, now, inst + Duration::from_secs(1))
            .unwrap_err();
        assert_eq!(replay_err, CapabilityVerificationError::ReplayDetected);
    }

    #[test]
    fn test_capability_verifier_protocol_range_rejection() {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let mut verifier = CapabilityVerifier::new(verifying_key, Duration::from_secs(60), 100);

        let cap = create_test_capability(&signing_key, "dev-host-1", 500, vec![5, 6, 7, 8]);

        let now = UnixTimestamp::from_secs(200);
        let inst = Instant::now();

        // Protocol version 3 is outside range [1, 2]
        let err = verifier.verify(&cap, 3, now, inst).unwrap_err();
        assert!(matches!(
            err,
            CapabilityVerificationError::ProtocolRangeViolation { .. }
        ));
    }
}
