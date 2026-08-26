//! Device enrollment tokens and validation logic.
//! Part of Nexus Remote Desktop Platform.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use nexus_common::id::TenantId;
use nexus_common::time::UnixTimestamp;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Type of device enrolling into the Nexus platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    /// Remote desktop host service (Windows agent).
    Host,
    /// Remote desktop client / viewer application.
    Client,
    /// Relay node.
    Relay,
}

impl DeviceType {
    /// Returns static string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Client => "client",
            Self::Relay => "relay",
        }
    }
}

/// Errors occurring during enrollment token validation.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum EnrollmentTokenError {
    /// Token has passed its expiration timestamp.
    #[error("enrollment token expired: expired at {expired_at}, current time {current_time}")]
    Expired {
        expired_at: UnixTimestamp,
        current_time: UnixTimestamp,
    },

    /// Token is not valid before the given timestamp.
    #[error(
        "enrollment token not yet active: not before {not_before}, current time {current_time}"
    )]
    NotYetActive {
        not_before: UnixTimestamp,
        current_time: UnixTimestamp,
    },

    /// Token signature does not match control plane verifying key.
    #[error("invalid enrollment token signature: {0}")]
    InvalidSignature(String),

    /// Missing required fields during token building.
    #[error("missing field when building enrollment token: {0}")]
    MissingField(&'static str),

    /// Serialization error during canonical bytes generation.
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Pre-signed one-time enrollment token issued by the control plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentToken {
    /// Unique identifier for this enrollment authorization.
    pub token_id: String,
    /// Tenant / organization this device will belong to.
    pub organization_id: TenantId,
    /// Role / device type allowed for enrollment with this token.
    pub device_type: DeviceType,
    /// Timestamp when token became valid.
    pub not_before: UnixTimestamp,
    /// Expiration timestamp for the enrollment window.
    pub expires_at: UnixTimestamp,
    /// Maximum number of devices allowed to enroll with this token (typically 1).
    pub max_uses: u32,
    /// Control plane Ed25519 signature over canonical token payload.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<u8>,
}

impl EnrollmentToken {
    /// Creates a new builder for constructing enrollment tokens.
    pub fn builder() -> EnrollmentTokenBuilder {
        EnrollmentTokenBuilder::default()
    }

    /// Generates canonical deterministic bytes for signing and signature verification.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, EnrollmentTokenError> {
        #[derive(Serialize)]
        struct CanonicalEnrollmentPayload<'a> {
            token_id: &'a str,
            organization_id: &'a str,
            device_type: &'a str,
            not_before: u64,
            expires_at: u64,
            max_uses: u32,
        }

        let payload = CanonicalEnrollmentPayload {
            token_id: &self.token_id,
            organization_id: self.organization_id.as_str(),
            device_type: self.device_type.as_str(),
            not_before: self.not_before.as_secs(),
            expires_at: self.expires_at.as_secs(),
            max_uses: self.max_uses,
        };

        serde_json::to_vec(&payload).map_err(|e| EnrollmentTokenError::Serialization(e.to_string()))
    }

    /// Signs the token using the Control Plane Ed25519 private key.
    pub fn sign(&mut self, signing_key: &SigningKey) -> Result<(), EnrollmentTokenError> {
        let bytes = self.signing_bytes()?;
        let sig = signing_key.sign(&bytes);
        self.signature = sig.to_bytes().to_vec();
        Ok(())
    }

    /// Verifies the token validity and cryptographic signature.
    pub fn verify(
        &self,
        verifying_key: &VerifyingKey,
        now: UnixTimestamp,
    ) -> Result<(), EnrollmentTokenError> {
        if now < self.not_before {
            return Err(EnrollmentTokenError::NotYetActive {
                not_before: self.not_before,
                current_time: now,
            });
        }
        if now > self.expires_at {
            return Err(EnrollmentTokenError::Expired {
                expired_at: self.expires_at,
                current_time: now,
            });
        }

        let sig_bytes: [u8; 64] = self.signature.as_slice().try_into().map_err(|_| {
            EnrollmentTokenError::InvalidSignature("signature length must be 64 bytes".into())
        })?;
        let signature = Signature::from_bytes(&sig_bytes);
        let signing_bytes = self.signing_bytes()?;

        verifying_key
            .verify(&signing_bytes, &signature)
            .map_err(|e| EnrollmentTokenError::InvalidSignature(e.to_string()))
    }
}

/// Builder for constructing [`EnrollmentToken`] instances.
#[derive(Debug, Default)]
pub struct EnrollmentTokenBuilder {
    token_id: Option<String>,
    organization_id: Option<TenantId>,
    device_type: Option<DeviceType>,
    not_before: Option<UnixTimestamp>,
    expires_at: Option<UnixTimestamp>,
    max_uses: Option<u32>,
}

impl EnrollmentTokenBuilder {
    pub fn token_id(mut self, token_id: impl Into<String>) -> Self {
        self.token_id = Some(token_id.into());
        self
    }

    pub fn organization_id(mut self, organization_id: TenantId) -> Self {
        self.organization_id = Some(organization_id);
        self
    }

    pub fn device_type(mut self, device_type: DeviceType) -> Self {
        self.device_type = Some(device_type);
        self
    }

    pub fn not_before(mut self, not_before: UnixTimestamp) -> Self {
        self.not_before = Some(not_before);
        self
    }

    pub fn expires_at(mut self, expires_at: UnixTimestamp) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    pub fn max_uses(mut self, max_uses: u32) -> Self {
        self.max_uses = Some(max_uses);
        self
    }

    pub fn build(self) -> Result<EnrollmentToken, EnrollmentTokenError> {
        let token_id = self
            .token_id
            .ok_or(EnrollmentTokenError::MissingField("token_id"))?;
        let organization_id = self
            .organization_id
            .ok_or(EnrollmentTokenError::MissingField("organization_id"))?;
        let device_type = self
            .device_type
            .ok_or(EnrollmentTokenError::MissingField("device_type"))?;
        let not_before = self.not_before.unwrap_or(UnixTimestamp::from_secs(0));
        let expires_at = self
            .expires_at
            .ok_or(EnrollmentTokenError::MissingField("expires_at"))?;
        let max_uses = self.max_uses.unwrap_or(1);

        Ok(EnrollmentToken {
            token_id,
            organization_id,
            device_type,
            not_before,
            expires_at,
            max_uses,
            signature: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enrollment_token_sign_and_verify() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let org_id = TenantId::new("org-enterprise-1").unwrap();

        let mut token = EnrollmentToken::builder()
            .token_id("tok-enroll-12345")
            .organization_id(org_id)
            .device_type(DeviceType::Host)
            .not_before(UnixTimestamp::from_secs(100))
            .expires_at(UnixTimestamp::from_secs(500))
            .max_uses(1)
            .build()
            .unwrap();

        token.sign(&signing_key).unwrap();
        assert!(!token.signature.is_empty());

        // Valid timestamp
        assert!(token
            .verify(&verifying_key, UnixTimestamp::from_secs(200))
            .is_ok());

        // Expired
        let err = token
            .verify(&verifying_key, UnixTimestamp::from_secs(600))
            .unwrap_err();
        assert!(matches!(err, EnrollmentTokenError::Expired { .. }));

        // Not yet active
        let err = token
            .verify(&verifying_key, UnixTimestamp::from_secs(50))
            .unwrap_err();
        assert!(matches!(err, EnrollmentTokenError::NotYetActive { .. }));

        // Signature tampering
        let wrong_key = SigningKey::from_bytes(&[8u8; 32]).verifying_key();
        let err = token
            .verify(&wrong_key, UnixTimestamp::from_secs(200))
            .unwrap_err();
        assert!(matches!(err, EnrollmentTokenError::InvalidSignature(_)));
    }
}
