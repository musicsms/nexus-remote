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
}
