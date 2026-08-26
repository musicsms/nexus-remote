//! Authentication and capability replay-protection primitives.

mod replay;

pub use replay::NonceReplayCache;

pub fn init() {
    // Initializer stub for nexus-auth
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use nexus_crypto::SignedPayload;
    use nexus_protocol::SessionCapability;
    use prost::Message;
    use std::time::{Duration, Instant};

    #[test]
    fn capability_serialization_signature_and_replay_work_together() {
        let capability = SessionCapability {
            version: 1,
            issuer: "cp".into(),
            session_id: "ses".into(),
            subject_user_id: "user".into(),
            client_device_id: "client".into(),
            target_device_id: "target".into(),
            permissions: vec!["desktop.control".into()],
            restrictions: vec![],
            not_before: 1,
            expires_at: 2,
            nonce: vec![7; 16],
            agent_min_protocol: 1,
            agent_max_protocol: 1,
            client_ephemeral_public_key: vec![],
            signature: vec![],
        };
        capability.validate().unwrap();
        let mut bytes = Vec::new();
        capability.encode(&mut bytes).unwrap();
        let secret = [4u8; 32];
        let public = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
        let signed = SignedPayload::sign(&secret, bytes);
        let verified = signed.verify(&public).unwrap();
        assert_eq!(SessionCapability::decode(verified).unwrap(), capability);
        let mut cache = NonceReplayCache::new(Duration::from_secs(30), 4);
        let now = Instant::now();
        assert!(cache.accept(&capability.nonce, now));
        assert!(!cache.accept(&capability.nonce, now + Duration::from_secs(1)));
    }
}
