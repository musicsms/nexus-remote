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
}
