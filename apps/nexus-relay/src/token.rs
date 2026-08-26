use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use nexus_common::{
    id::{DeviceId, SessionId},
    time::UnixTimestamp,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndpointRole {
    Client,
    Host,
}
impl EndpointRole {
    pub const fn peer(self) -> Self {
        match self {
            Self::Client => Self::Host,
            Self::Host => Self::Client,
        }
    }
}
impl std::fmt::Display for EndpointRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayToken {
    pub session_id: SessionId,
    pub relay_id: String,
    pub client_device_id: DeviceId,
    pub target_device_id: DeviceId,
    pub role: EndpointRole,
    pub expires_at: UnixTimestamp,
    pub signature: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RelayTokenError {
    #[error("invalid relay token signature")]
    InvalidSignature,
    #[error("relay token is expired")]
    Expired,
    #[error("relay token targets a different relay")]
    WrongRelay,
    #[error("relay token signature has invalid length")]
    SignatureLength,
    #[error("relay token has an empty relay id")]
    InvalidRelayId,
}

impl RelayToken {
    pub fn builder() -> RelayTokenBuilder {
        RelayTokenBuilder::default()
    }
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut clone = self.clone();
        clone.signature.clear();
        serde_json::to_vec(&clone).expect("relay token fields are serializable")
    }
    pub fn sign(&mut self, key: &SigningKey) {
        self.signature = key.sign(&self.signing_bytes()).to_bytes().to_vec();
    }
}

#[derive(Default)]
pub struct RelayTokenBuilder {
    session_id: Option<SessionId>,
    relay_id: Option<String>,
    client_device_id: Option<DeviceId>,
    target_device_id: Option<DeviceId>,
    role: Option<EndpointRole>,
    expires_at: Option<UnixTimestamp>,
}
impl RelayTokenBuilder {
    pub fn session_id(mut self, v: SessionId) -> Self {
        self.session_id = Some(v);
        self
    }
    pub fn relay_id(mut self, v: impl Into<String>) -> Self {
        self.relay_id = Some(v.into());
        self
    }
    pub fn client_device_id(mut self, v: DeviceId) -> Self {
        self.client_device_id = Some(v);
        self
    }
    pub fn target_device_id(mut self, v: DeviceId) -> Self {
        self.target_device_id = Some(v);
        self
    }
    pub fn role(mut self, v: EndpointRole) -> Self {
        self.role = Some(v);
        self
    }
    pub fn expires_at(mut self, v: UnixTimestamp) -> Self {
        self.expires_at = Some(v);
        self
    }
    pub fn build(self) -> Result<RelayToken, RelayTokenError> {
        Ok(RelayToken {
            session_id: self.session_id.ok_or(RelayTokenError::InvalidRelayId)?,
            relay_id: self.relay_id.ok_or(RelayTokenError::InvalidRelayId)?,
            client_device_id: self
                .client_device_id
                .ok_or(RelayTokenError::InvalidRelayId)?,
            target_device_id: self
                .target_device_id
                .ok_or(RelayTokenError::InvalidRelayId)?,
            role: self.role.ok_or(RelayTokenError::InvalidRelayId)?,
            expires_at: self.expires_at.ok_or(RelayTokenError::InvalidRelayId)?,
            signature: Vec::new(),
        })
    }
}

#[derive(Clone)]
pub struct RelayTokenVerifier {
    key: VerifyingKey,
    relay_id: String,
}
impl RelayTokenVerifier {
    pub fn new(key: VerifyingKey, relay_id: impl Into<String>) -> Self {
        Self {
            key,
            relay_id: relay_id.into(),
        }
    }
    pub fn relay_id(&self) -> &str {
        &self.relay_id
    }
    pub fn verify(&self, token: &RelayToken, now: UnixTimestamp) -> Result<(), RelayTokenError> {
        if token.relay_id != self.relay_id {
            return Err(RelayTokenError::WrongRelay);
        }
        if token.expires_at <= now {
            return Err(RelayTokenError::Expired);
        }
        if token.signature.len() != 64 {
            return Err(RelayTokenError::SignatureLength);
        }
        let sig = Signature::from_slice(&token.signature)
            .map_err(|_| RelayTokenError::SignatureLength)?;
        self.key
            .verify(&token.signing_bytes(), &sig)
            .map_err(|_| RelayTokenError::InvalidSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signs_and_verifies() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let mut t = RelayToken::builder()
            .session_id(SessionId::new("s").unwrap())
            .relay_id("r")
            .client_device_id(DeviceId::new("c").unwrap())
            .target_device_id(DeviceId::new("h").unwrap())
            .role(EndpointRole::Client)
            .expires_at(UnixTimestamp::from_secs(20))
            .build()
            .unwrap();
        t.sign(&key);
        assert!(RelayTokenVerifier::new(key.verifying_key(), "r")
            .verify(&t, UnixTimestamp::from_secs(10))
            .is_ok());
    }
}
