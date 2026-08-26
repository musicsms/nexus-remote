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
    #[error("relay token is missing required field: {0}")]
    MissingField(&'static str),
    #[error("relay token field is too long: {0}")]
    FieldTooLong(&'static str),
}

impl RelayToken {
    pub fn builder() -> RelayTokenBuilder {
        RelayTokenBuilder::default()
    }
    pub fn signing_bytes(&self) -> Vec<u8> {
        // Explicit, versioned wire encoding prevents signatures depending on
        // serializer implementation details or field ordering.
        let mut out = b"nexus-relay-token/v1\0".to_vec();
        fn put(out: &mut Vec<u8>, s: &str) {
            out.extend_from_slice(&(s.len() as u16).to_be_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        put(&mut out, self.session_id.as_str());
        put(&mut out, &self.relay_id);
        put(&mut out, self.client_device_id.as_str());
        put(&mut out, self.target_device_id.as_str());
        out.push(match self.role {
            EndpointRole::Client => 0,
            EndpointRole::Host => 1,
        });
        out.extend_from_slice(&self.expires_at.as_secs().to_be_bytes());
        out
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
            session_id: self
                .session_id
                .ok_or(RelayTokenError::MissingField("session_id"))?,
            relay_id: {
                let id = self
                    .relay_id
                    .ok_or(RelayTokenError::MissingField("relay_id"))?;
                if id.is_empty() {
                    return Err(RelayTokenError::InvalidRelayId);
                }
                if id.len() > u16::MAX as usize {
                    return Err(RelayTokenError::FieldTooLong("relay_id"));
                }
                id
            },
            client_device_id: self
                .client_device_id
                .ok_or(RelayTokenError::MissingField("client_device_id"))?,
            target_device_id: self
                .target_device_id
                .ok_or(RelayTokenError::MissingField("target_device_id"))?,
            role: self.role.ok_or(RelayTokenError::MissingField("role"))?,
            expires_at: self
                .expires_at
                .ok_or(RelayTokenError::MissingField("expires_at"))?,
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
