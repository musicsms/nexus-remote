//! Small, OS-independent cryptographic primitives used by capabilities.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("invalid signing key length")]
    InvalidSigningKey,
    #[error("invalid verifying key length")]
    InvalidVerifyingKey,
    #[error("invalid signature length")]
    InvalidSignature,
    #[error("signature verification failed")]
    VerificationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedPayload {
    pub payload: Vec<u8>,
    pub signature: [u8; 64],
}

/// An Ed25519 device identity backed by provisioned 32-byte seed material.
/// The seed is intentionally private; persistence and rotation belong to the
/// platform/control-plane layer rather than the wire protocol.
#[derive(Clone)]
pub struct DeviceKeypair {
    signing_key: SigningKey,
}

impl DeviceKeypair {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn sign(&self, payload: &[u8]) -> [u8; 64] {
        self.signing_key.sign(payload).to_bytes()
    }

    pub fn verify(&self, payload: &[u8], signature: &[u8; 64]) -> Result<(), SignatureError> {
        verify_ed25519(&self.public_key(), payload, signature)
    }
}

impl SignedPayload {
    pub fn sign(secret_key: &[u8; 32], payload: impl Into<Vec<u8>>) -> Self {
        let payload = payload.into();
        Self {
            signature: sign_ed25519(secret_key, &payload),
            payload,
        }
    }

    pub fn verify(&self, public_key: &[u8; 32]) -> Result<&[u8], SignatureError> {
        verify_ed25519(public_key, &self.payload, &self.signature)?;
        Ok(&self.payload)
    }
}

pub fn sign_ed25519(secret_key: &[u8; 32], payload: &[u8]) -> [u8; 64] {
    let key = SigningKey::from_bytes(secret_key);
    key.sign(payload).to_bytes()
}

pub fn verify_ed25519(
    public_key: &[u8; 32],
    payload: &[u8],
    signature: &[u8; 64],
) -> Result<(), SignatureError> {
    let key =
        VerifyingKey::from_bytes(public_key).map_err(|_| SignatureError::InvalidVerifyingKey)?;
    key.verify(payload, &Signature::from_bytes(signature))
        .map_err(|_| SignatureError::VerificationFailed)
}

pub fn init() {
    // Reserved for crypto provider initialization.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_verifies_capability_payload() {
        let secret = [7u8; 32];
        let signing = SigningKey::from_bytes(&secret);
        let public = signing.verifying_key().to_bytes();
        let payload = b"session capability";
        let signature = sign_ed25519(&secret, payload);
        assert!(verify_ed25519(&public, payload, &signature).is_ok());
        assert_eq!(
            verify_ed25519(&public, b"tampered", &signature),
            Err(SignatureError::VerificationFailed)
        );
    }

    #[test]
    fn signed_payload_envelope_detects_mutation() {
        let secret = [9u8; 32];
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let mut signed = SignedPayload::sign(&secret, b"capability".to_vec());
        assert_eq!(signed.verify(&public).unwrap(), b"capability");
        signed.payload[0] ^= 1;
        assert_eq!(
            signed.verify(&public),
            Err(SignatureError::VerificationFailed)
        );
    }

    #[test]
    fn device_keypair_round_trips_without_exposing_seed() {
        let identity = DeviceKeypair::from_seed([11; 32]);
        let signature = identity.sign(b"device identity");
        assert!(identity.verify(b"device identity", &signature).is_ok());
        assert_eq!(
            identity.public_key(),
            SigningKey::from_bytes(&[11; 32]).verifying_key().to_bytes()
        );
        assert_eq!(
            identity.verify(b"tampered", &signature),
            Err(SignatureError::VerificationFailed)
        );
    }

    #[test]
    fn device_keypair_rejects_signature_from_another_identity() {
        let first = DeviceKeypair::from_seed([1; 32]);
        let second = DeviceKeypair::from_seed([2; 32]);
        let signature = first.sign(b"payload");
        assert_eq!(
            second.verify(b"payload", &signature),
            Err(SignatureError::VerificationFailed)
        );
    }
}
