//! Small, OS-independent cryptographic primitives used by capabilities.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Key, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyDerivationError {
    #[error("x25519 produced an invalid all-zero shared secret")]
    InvalidSharedSecret,
    #[error("HKDF expansion failed")]
    ExpansionFailed,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AeadError {
    #[error("AEAD encryption failed")]
    EncryptionFailed,
    #[error("AEAD authentication failed")]
    AuthenticationFailed,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NonceSequenceError {
    #[error("AEAD nonce sequence exhausted; session must be rekeyed")]
    Exhausted,
}

/// Monotonic per-channel nonce allocator matching ADR-025's 32+64 layout.
#[derive(Debug, PartialEq, Eq)]
pub struct NonceSequence {
    domain: u32,
    next: u64,
    exhausted: bool,
}

impl NonceSequence {
    pub const fn new(domain: u32) -> Self {
        Self {
            domain,
            next: 0,
            exhausted: false,
        }
    }

    pub const fn domain(self) -> u32 {
        self.domain
    }

    pub fn next_nonce(&mut self) -> Result<[u8; 12], NonceSequenceError> {
        if self.exhausted {
            return Err(NonceSequenceError::Exhausted);
        }
        let sequence = self.next;
        if sequence == u64::MAX {
            self.exhausted = true;
        } else {
            self.next += 1;
        }
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.domain.to_be_bytes());
        nonce[4..].copy_from_slice(&sequence.to_be_bytes());
        Ok(nonce)
    }
}

pub fn seal_session_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    associated_data: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AeadError> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| AeadError::EncryptionFailed)
}

pub fn open_session_payload(
    key: &[u8; 32],
    nonce: &[u8; 12],
    associated_data: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, AeadError> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| AeadError::AuthenticationFailed)
}

/// Derive a domain-separated 32-byte session root key from X25519 material.
pub fn derive_session_key(
    local_secret: &[u8; 32],
    peer_public: &[u8; 32],
    transcript_context: &[u8],
) -> Result<[u8; 32], KeyDerivationError> {
    let shared =
        StaticSecret::from(*local_secret).diffie_hellman(&X25519PublicKey::from(*peer_public));
    if shared.as_bytes().iter().all(|byte| *byte == 0) {
        return Err(KeyDerivationError::InvalidSharedSecret);
    }
    let hkdf = Hkdf::<Sha256>::new(Some(b"nexus/session-key/v1"), shared.as_bytes());
    let mut output = [0u8; 32];
    hkdf.expand(transcript_context, &mut output)
        .map_err(|_| KeyDerivationError::ExpansionFailed)?;
    Ok(output)
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

    #[test]
    fn session_key_derivation_is_symmetric_and_context_bound() {
        let alice_secret = StaticSecret::from([3u8; 32]);
        let bob_secret = StaticSecret::from([4u8; 32]);
        let alice_public = X25519PublicKey::from(&alice_secret);
        let bob_public = X25519PublicKey::from(&bob_secret);
        let first =
            derive_session_key(alice_secret.as_bytes(), bob_public.as_bytes(), b"session-1")
                .unwrap();
        let second =
            derive_session_key(bob_secret.as_bytes(), alice_public.as_bytes(), b"session-1")
                .unwrap();
        assert_eq!(first, second);
        assert_ne!(
            first,
            derive_session_key(alice_secret.as_bytes(), bob_public.as_bytes(), b"session-2")
                .unwrap()
        );
    }

    #[test]
    fn session_key_derivation_rejects_low_order_peer_key() {
        assert_eq!(
            derive_session_key(&[9; 32], &[0; 32], b"ctx"),
            Err(KeyDerivationError::InvalidSharedSecret)
        );
    }

    #[test]
    fn aead_round_trip_authenticates_associated_data() {
        let key = [5u8; 32];
        let nonce = [6u8; 12];
        let ciphertext = seal_session_payload(&key, &nonce, b"header", b"frame").unwrap();
        assert_eq!(
            open_session_payload(&key, &nonce, b"header", &ciphertext).unwrap(),
            b"frame"
        );
        assert_eq!(
            open_session_payload(&key, &nonce, b"tampered", &ciphertext),
            Err(AeadError::AuthenticationFailed)
        );
    }

    #[test]
    fn aead_rejects_ciphertext_mutation_and_wrong_key() {
        let nonce = [8u8; 12];
        let mut ciphertext = seal_session_payload(&[1; 32], &nonce, b"aad", b"secret").unwrap();
        ciphertext[0] ^= 1;
        assert_eq!(
            open_session_payload(&[1; 32], &nonce, b"aad", &ciphertext),
            Err(AeadError::AuthenticationFailed)
        );
        let ciphertext = seal_session_payload(&[1; 32], &nonce, b"aad", b"secret").unwrap();
        assert_eq!(
            open_session_payload(&[2; 32], &nonce, b"aad", &ciphertext),
            Err(AeadError::AuthenticationFailed)
        );
    }

    #[test]
    fn nonce_sequence_encodes_domain_and_monotonic_counter() {
        let mut sequence = NonceSequence::new(0x0102_0304);
        assert_eq!(
            sequence.next_nonce().unwrap(),
            [1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(sequence.next_nonce().unwrap()[11], 1);
    }

    #[test]
    fn nonce_sequence_fails_closed_before_wrap() {
        let mut sequence = NonceSequence {
            domain: 7,
            next: u64::MAX - 1,
            exhausted: false,
        };
        assert!(sequence.next_nonce().is_ok());
        assert!(sequence.next_nonce().is_ok());
        assert_eq!(sequence.next_nonce(), Err(NonceSequenceError::Exhausted));
    }
}
