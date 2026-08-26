//! Local host identity and keypair management for nexus-agent.
//! Part of Nexus Remote Desktop Platform.

use std::fs;
use std::path::PathBuf;

use ed25519_dalek::{SigningKey, VerifyingKey};
use nexus_auth::credential::DeviceCredential;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors arising during local identity operations.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("I/O error during identity operation: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("Device credential not found (device not enrolled)")]
    NotEnrolled,
}

/// Persistent identity data stored on the host machine.
#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    device_private_key: Vec<u8>,
    credential: Option<DeviceCredential>,
}

/// Host device identity manager.
pub struct AgentIdentity {
    storage_path: PathBuf,
    signing_key: SigningKey,
    credential: Option<DeviceCredential>,
}

impl AgentIdentity {
    /// Loads an existing identity from storage, or creates a new keypair if not present.
    pub fn load_or_generate(storage_path: impl Into<PathBuf>) -> Result<Self, IdentityError> {
        let storage_path = storage_path.into();

        if storage_path.exists() {
            let data = fs::read(&storage_path)?;
            let stored: StoredIdentity = serde_json::from_slice(&data)?;

            let key_bytes: [u8; 32] = stored
                .device_private_key
                .as_slice()
                .try_into()
                .map_err(|_| IdentityError::InvalidKey("private key must be 32 bytes".into()))?;

            let signing_key = SigningKey::from_bytes(&key_bytes);

            Ok(Self {
                storage_path,
                signing_key,
                credential: stored.credential,
            })
        } else {
            // Generate deterministic or random 32-byte secret
            use std::time::SystemTime;
            let seed = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut key_bytes = [0u8; 32];
            key_bytes[..16].copy_from_slice(&seed.to_be_bytes());
            key_bytes[16..].copy_from_slice(&seed.to_le_bytes());

            let signing_key = SigningKey::from_bytes(&key_bytes);
            let identity = Self {
                storage_path,
                signing_key,
                credential: None,
            };
            identity.save()?;
            Ok(identity)
        }
    }

    /// Creates an in-memory identity with a specific signing key (useful for tests).
    pub fn from_signing_key(signing_key: SigningKey, storage_path: impl Into<PathBuf>) -> Self {
        Self {
            storage_path: storage_path.into(),
            signing_key,
            credential: None,
        }
    }

    /// Returns the local Ed25519 signing key.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    /// Returns the corresponding verifying (public) key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Returns the device credential if enrolled.
    pub fn credential(&self) -> Option<&DeviceCredential> {
        self.credential.as_ref()
    }

    /// Saves the enrolled credential and writes identity to disk.
    pub fn set_credential(&mut self, credential: DeviceCredential) -> Result<(), IdentityError> {
        self.credential = Some(credential);
        self.save()
    }

    /// Persists current identity state to disk.
    pub fn save(&self) -> Result<(), IdentityError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let stored = StoredIdentity {
            device_private_key: self.signing_key.to_bytes().to_vec(),
            credential: self.credential.clone(),
        };

        let data = serde_json::to_vec_pretty(&stored)?;
        fs::write(&self.storage_path, data)?;
        Ok(())
    }
}
