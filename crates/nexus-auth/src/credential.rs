//! Device credential models, registration requests, and validation.
//! Part of Nexus Remote Desktop Platform.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use nexus_common::id::{DeviceId, TenantId};
use nexus_common::time::UnixTimestamp;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::enrollment::{DeviceType, EnrollmentToken, EnrollmentTokenError};

/// Errors occurring during device credential issuance or validation.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DeviceCredentialError {
    /// Credential has expired.
    #[error("device credential expired: expired at {expired_at}, current time {current_time}")]
    Expired {
        expired_at: UnixTimestamp,
        current_time: UnixTimestamp,
    },

    /// Credential is not valid yet.
    #[error(
        "device credential not yet active: not before {not_before}, current time {current_time}"
    )]
    NotYetActive {
        not_before: UnixTimestamp,
        current_time: UnixTimestamp,
    },

    /// Invalid cryptographic signature.
    #[error("invalid credential signature: {0}")]
    InvalidSignature(String),

    /// Proof-of-possession verification failed for device registration request.
    #[error("invalid device proof-of-possession signature: {0}")]
    InvalidProofOfPossession(String),

    /// Enrollment token validation error.
    #[error("enrollment token error: {0}")]
    Enrollment(#[from] EnrollmentTokenError),

    /// Missing required fields during credential building.
    #[error("missing field when building device credential: {0}")]
    MissingField(&'static str),

    /// Serialization error during canonical bytes generation.
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Metadata sent by an installer/agent requesting device registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistrationRequest {
    /// Pre-signed one-time enrollment token.
    pub enrollment_token: EnrollmentToken,
    /// Device's locally generated Ed25519 public key (32 bytes).
    pub device_public_key: Vec<u8>,
    /// Operating system (e.g. "windows", "linux", "macos").
    pub os: String,
    /// Architecture (e.g. "x86_64", "aarch64").
    pub architecture: String,
    /// Machine hostname.
    pub hostname: String,
    /// Device agent version.
    pub agent_version: String,
    /// Registration request timestamp.
    pub requested_at: UnixTimestamp,
    /// Device's proof-of-possession signature over `(enrollment_token.token_id || requested_at)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proof_signature: Vec<u8>,
}

impl DeviceRegistrationRequest {
    /// Computes canonical signing bytes for proof-of-possession.
    pub fn proof_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.enrollment_token.token_id.len() + 8);
        bytes.extend_from_slice(self.enrollment_token.token_id.as_bytes());
        bytes.extend_from_slice(&self.requested_at.as_secs().to_be_bytes());
        bytes
    }

    /// Signs the request using the device's locally generated private key.
    pub fn sign_proof(&mut self, device_signing_key: &SigningKey) {
        let bytes = self.proof_bytes();
        let sig = device_signing_key.sign(&bytes);
        self.proof_signature = sig.to_bytes().to_vec();
    }

    /// Verifies the proof-of-possession signature against the device's public key.
    pub fn verify_proof(&self) -> Result<(), DeviceCredentialError> {
        let key_bytes: [u8; 32] = self.device_public_key.as_slice().try_into().map_err(|_| {
            DeviceCredentialError::InvalidProofOfPossession(
                "device public key must be 32 bytes".into(),
            )
        })?;
        let verifying_key = VerifyingKey::from_bytes(&key_bytes).map_err(|e| {
            DeviceCredentialError::InvalidProofOfPossession(format!(
                "invalid public key encoding: {e}"
            ))
        })?;

        let sig_bytes: [u8; 64] = self.proof_signature.as_slice().try_into().map_err(|_| {
            DeviceCredentialError::InvalidProofOfPossession(
                "proof signature must be 64 bytes".into(),
            )
        })?;
        let signature = Signature::from_bytes(&sig_bytes);

        verifying_key
            .verify(&self.proof_bytes(), &signature)
            .map_err(|e| DeviceCredentialError::InvalidProofOfPossession(e.to_string()))
    }
}

/// Signed device identity credential issued by the control plane upon successful enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCredential {
    /// Unique identifier assigned to the device.
    pub device_id: DeviceId,
    /// Tenant / organization owning this device.
    pub organization_id: TenantId,
    /// Device's registered Ed25519 public key.
    pub public_key: Vec<u8>,
    /// Role / device type.
    pub device_type: DeviceType,
    /// OS metadata.
    pub os: String,
    /// Architecture metadata.
    pub architecture: String,
    /// Capabilities granted to this device.
    pub capabilities: Vec<String>,
    /// Credential activation timestamp.
    pub issued_at: UnixTimestamp,
    /// Credential expiration timestamp.
    pub expires_at: UnixTimestamp,
    /// Control plane signature over canonical credential bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<u8>,
}

impl DeviceCredential {
    /// Creates a new builder for constructing [`DeviceCredential`] instances.
    pub fn builder() -> DeviceCredentialBuilder {
        DeviceCredentialBuilder::default()
    }

    /// Generates canonical deterministic bytes for signing and verification.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, DeviceCredentialError> {
        #[derive(Serialize)]
        struct CanonicalCredentialPayload<'a> {
            device_id: &'a str,
            organization_id: &'a str,
            public_key: &'a [u8],
            device_type: &'a str,
            os: &'a str,
            architecture: &'a str,
            capabilities: &'a [String],
            issued_at: u64,
            expires_at: u64,
        }

        let payload = CanonicalCredentialPayload {
            device_id: self.device_id.as_str(),
            organization_id: self.organization_id.as_str(),
            public_key: &self.public_key,
            device_type: self.device_type.as_str(),
            os: &self.os,
            architecture: &self.architecture,
            capabilities: &self.capabilities,
            issued_at: self.issued_at.as_secs(),
            expires_at: self.expires_at.as_secs(),
        };

        serde_json::to_vec(&payload)
            .map_err(|e| DeviceCredentialError::Serialization(e.to_string()))
    }

    /// Signs the credential using the Control Plane Ed25519 private key.
    pub fn sign(&mut self, cp_signing_key: &SigningKey) -> Result<(), DeviceCredentialError> {
        let bytes = self.signing_bytes()?;
        let sig = cp_signing_key.sign(&bytes);
        self.signature = sig.to_bytes().to_vec();
        Ok(())
    }

    /// Verifies credential validity and cryptographic signature from the Control Plane.
    pub fn verify(
        &self,
        cp_verifying_key: &VerifyingKey,
        now: UnixTimestamp,
    ) -> Result<(), DeviceCredentialError> {
        if now < self.issued_at {
            return Err(DeviceCredentialError::NotYetActive {
                not_before: self.issued_at,
                current_time: now,
            });
        }
        if now > self.expires_at {
            return Err(DeviceCredentialError::Expired {
                expired_at: self.expires_at,
                current_time: now,
            });
        }

        let sig_bytes: [u8; 64] = self.signature.as_slice().try_into().map_err(|_| {
            DeviceCredentialError::InvalidSignature("signature length must be 64 bytes".into())
        })?;
        let signature = Signature::from_bytes(&sig_bytes);
        let signing_bytes = self.signing_bytes()?;

        cp_verifying_key
            .verify(&signing_bytes, &signature)
            .map_err(|e| DeviceCredentialError::InvalidSignature(e.to_string()))
    }
}

/// Builder for constructing [`DeviceCredential`] instances.
#[derive(Debug, Default)]
pub struct DeviceCredentialBuilder {
    device_id: Option<DeviceId>,
    organization_id: Option<TenantId>,
    public_key: Option<Vec<u8>>,
    device_type: Option<DeviceType>,
    os: Option<String>,
    architecture: Option<String>,
    capabilities: Vec<String>,
    issued_at: Option<UnixTimestamp>,
    expires_at: Option<UnixTimestamp>,
}

impl DeviceCredentialBuilder {
    pub fn device_id(mut self, device_id: DeviceId) -> Self {
        self.device_id = Some(device_id);
        self
    }

    pub fn organization_id(mut self, organization_id: TenantId) -> Self {
        self.organization_id = Some(organization_id);
        self
    }

    pub fn public_key(mut self, public_key: impl Into<Vec<u8>>) -> Self {
        self.public_key = Some(public_key.into());
        self
    }

    pub fn device_type(mut self, device_type: DeviceType) -> Self {
        self.device_type = Some(device_type);
        self
    }

    pub fn os(mut self, os: impl Into<String>) -> Self {
        self.os = Some(os.into());
        self
    }

    pub fn architecture(mut self, architecture: impl Into<String>) -> Self {
        self.architecture = Some(architecture.into());
        self
    }

    pub fn capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn issued_at(mut self, issued_at: UnixTimestamp) -> Self {
        self.issued_at = Some(issued_at);
        self
    }

    pub fn expires_at(mut self, expires_at: UnixTimestamp) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    pub fn build(self) -> Result<DeviceCredential, DeviceCredentialError> {
        let device_id = self
            .device_id
            .ok_or(DeviceCredentialError::MissingField("device_id"))?;
        let organization_id = self
            .organization_id
            .ok_or(DeviceCredentialError::MissingField("organization_id"))?;
        let public_key = self
            .public_key
            .ok_or(DeviceCredentialError::MissingField("public_key"))?;
        let device_type = self
            .device_type
            .ok_or(DeviceCredentialError::MissingField("device_type"))?;
        let os = self.os.unwrap_or_else(|| "windows".to_string());
        let architecture = self.architecture.unwrap_or_else(|| "x86_64".to_string());
        let issued_at = self.issued_at.unwrap_or(UnixTimestamp::from_secs(0));
        let expires_at = self
            .expires_at
            .ok_or(DeviceCredentialError::MissingField("expires_at"))?;

        Ok(DeviceCredential {
            device_id,
            organization_id,
            public_key,
            device_type,
            os,
            architecture,
            capabilities: self.capabilities,
            issued_at,
            expires_at,
            signature: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_registration_request_proof_of_possession() {
        let cp_signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let device_signing_key = SigningKey::from_bytes(&[2u8; 32]);
        let org_id = TenantId::new("org-corp-1").unwrap();

        let mut token = EnrollmentToken::builder()
            .token_id("tok-reg-99")
            .organization_id(org_id.clone())
            .device_type(DeviceType::Host)
            .expires_at(UnixTimestamp::from_secs(1000))
            .build()
            .unwrap();
        token.sign(&cp_signing_key).unwrap();

        let mut req = DeviceRegistrationRequest {
            enrollment_token: token,
            device_public_key: device_signing_key.verifying_key().to_bytes().to_vec(),
            os: "windows".into(),
            architecture: "x86_64".into(),
            hostname: "workstation-01".into(),
            agent_version: "0.1.0".into(),
            requested_at: UnixTimestamp::from_secs(100),
            proof_signature: Vec::new(),
        };

        req.sign_proof(&device_signing_key);
        assert!(req.verify_proof().is_ok());

        // Tamper with request timestamp
        req.requested_at = UnixTimestamp::from_secs(101);
        assert!(req.verify_proof().is_err());
    }

    #[test]
    fn test_device_credential_sign_and_verify() {
        let cp_signing_key = SigningKey::from_bytes(&[3u8; 32]);
        let cp_verifying_key = cp_signing_key.verifying_key();
        let device_id = DeviceId::new("dev-host-01").unwrap();
        let org_id = TenantId::new("org-corp-1").unwrap();

        let mut cred = DeviceCredential::builder()
            .device_id(device_id)
            .organization_id(org_id)
            .public_key([9u8; 32])
            .device_type(DeviceType::Host)
            .issued_at(UnixTimestamp::from_secs(100))
            .expires_at(UnixTimestamp::from_secs(500))
            .capabilities(vec!["capture".into(), "input".into()])
            .build()
            .unwrap();

        cred.sign(&cp_signing_key).unwrap();
        assert!(cred
            .verify(&cp_verifying_key, UnixTimestamp::from_secs(200))
            .is_ok());

        // Expired
        assert!(cred
            .verify(&cp_verifying_key, UnixTimestamp::from_secs(600))
            .is_err());
    }
}
