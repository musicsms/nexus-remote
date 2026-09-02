use ed25519_dalek::{Signer, SigningKey};
use nexus_client::session::{
    ClientError, ClientSession, ClientState, ClientVerification, RelayTokenMetadata, SessionPolicy,
};
use nexus_common::{MockClock, UnixTimestamp};
use nexus_protocol::SessionCapability;
use std::time::Duration;

fn capability() -> SessionCapability {
    let mut c = SessionCapability {
        version: 1,
        issuer: "control-plane".into(),
        session_id: "session-1".into(),
        subject_user_id: "user-1".into(),
        client_device_id: "client-1".into(),
        target_device_id: "host-1".into(),
        permissions: vec!["desktop.view".into()],
        restrictions: vec![],
        not_before: 100,
        expires_at: 200,
        nonce: vec![1],
        agent_min_protocol: 1,
        agent_max_protocol: 1,
        client_ephemeral_public_key: vec![],
        signature: vec![],
    };
    c.signature = SigningKey::from_bytes(&[1; 32])
        .sign(&c.signing_bytes())
        .to_bytes()
        .to_vec();
    c
}
fn token() -> RelayTokenMetadata {
    let mut t = RelayTokenMetadata {
        relay_id: "relay".into(),
        session_id: "session-1".into(),
        client_device_id: "client-1".into(),
        target_device_id: "host-1".into(),
        expires_at: UnixTimestamp::from_secs(200),
        signature: vec![],
    };
    t.signature = SigningKey::from_bytes(&[2; 32])
        .sign(&t.signing_bytes())
        .to_bytes()
        .to_vec();
    t
}
fn client(c: SessionCapability, t: RelayTokenMetadata, now: u64) -> ClientSession {
    ClientSession::new(
        c,
        t,
        MockClock::from_secs(now),
        SessionPolicy::new(Duration::from_secs(1800), Duration::from_secs(60)).unwrap(),
        ClientVerification {
            capability_key: SigningKey::from_bytes(&[1; 32]).verifying_key(),
            relay_key: SigningKey::from_bytes(&[2; 32]).verifying_key(),
            relay_id: "relay".into(),
        },
    )
}

#[test]
fn exposes_signed_capability_permissions_for_each_boundary() {
    let c = client(capability(), token(), 100);
    assert!(c.has_permission("desktop.view"));
    assert!(c.can_view());
    assert!(!c.can_control());
    assert!(!c.has_permission("desktop.control"));
    assert!(c.require_view().is_ok());
    assert_eq!(
        c.require_control(),
        Err(ClientError::PermissionDenied("desktop.control"))
    );
}
#[test]
fn follows_lifecycle_and_expires() {
    let mut cap = capability();
    cap.expires_at = 120;
    cap.signature = SigningKey::from_bytes(&[1; 32])
        .sign(&cap.signing_bytes())
        .to_bytes()
        .to_vec();
    let mut c = client(cap, token(), 100);
    assert_eq!(c.state(), ClientState::Disconnected);
    c.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    c.connected(UnixTimestamp::from_secs(101)).unwrap();
    c.transport_lost(UnixTimestamp::from_secs(102)).unwrap();
    assert_eq!(c.state(), ClientState::Reconnecting);
    assert!(c.reconnect_deadline().is_some());
    c.expire().unwrap();
    assert_eq!(c.state(), ClientState::Expired);
}
#[test]
fn rejects_skipped_and_expired_transitions() {
    let mut cap = capability();
    cap.expires_at = 120;
    cap.signature = SigningKey::from_bytes(&[1; 32])
        .sign(&cap.signing_bytes())
        .to_bytes()
        .to_vec();
    let mut c = client(cap, token(), 100);
    assert!(matches!(
        c.connected(UnixTimestamp::from_secs(100)),
        Err(ClientError::InvalidTransition { .. })
    ));
    c.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    c.connected(UnixTimestamp::from_secs(101)).unwrap();
    c.expire().unwrap();
    assert!(matches!(
        c.transport_lost(UnixTimestamp::from_secs(102)),
        Err(ClientError::Expired)
    ));
}
#[test]
fn validates_identity_and_capability_window() {
    let mut t = token();
    t.session_id = "other".into();
    let mut c = client(capability(), t, 100);
    assert!(matches!(
        c.begin_connect(UnixTimestamp::from_secs(100)),
        Err(ClientError::IdentityMismatch)
    ));
    let mut x = capability();
    x.not_before = 150;
    let mut c = client(x, token(), 100);
    assert!(matches!(
        c.begin_connect(UnixTimestamp::from_secs(100)),
        Err(ClientError::CapabilityNotActive)
    ));
}
#[test]
fn reconnect_deadline_and_duration_are_bounded() {
    let mut c = client(capability(), token(), 100);
    c.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    c.connected(UnixTimestamp::from_secs(101)).unwrap();
    c.transport_lost(UnixTimestamp::from_secs(102)).unwrap();
    assert!(c.can_reconnect(UnixTimestamp::from_secs(162)));
    assert!(!c.can_reconnect(UnixTimestamp::from_secs(163)));
    c.begin_connect(UnixTimestamp::from_secs(110)).unwrap();
    c.connected(UnixTimestamp::from_secs(111)).unwrap();
    assert!(c.session_duration_expired(UnixTimestamp::from_secs(1901)));
}

#[test]
fn transient_reconnect_failure_returns_to_reconnecting_until_window_expires() {
    let mut client = client(capability(), token(), 100);
    client.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    client.connected(UnixTimestamp::from_secs(101)).unwrap();
    client
        .transport_lost(UnixTimestamp::from_secs(102))
        .unwrap();
    client.begin_connect(UnixTimestamp::from_secs(110)).unwrap();
    client
        .reconnect_attempt_failed(UnixTimestamp::from_secs(111))
        .unwrap();
    assert_eq!(client.state(), ClientState::Reconnecting);
    client
        .begin_connect(UnixTimestamp::from_secs(163))
        .unwrap_err();
    assert_eq!(client.state(), ClientState::Expired);
}

#[test]
fn rejects_invalid_signatures_and_expiry_on_reconnect() {
    let mut c = capability();
    c.signature[0] ^= 1;
    assert!(matches!(
        client(c, token(), 100).begin_connect(UnixTimestamp::from_secs(100)),
        Err(ClientError::InvalidCapabilitySignature)
    ));
    let mut t = token();
    t.signature[0] ^= 1;
    assert!(matches!(
        client(capability(), t, 100).begin_connect(UnixTimestamp::from_secs(100)),
        Err(ClientError::InvalidRelaySignature)
    ));
    let mut cap = capability();
    cap.expires_at = 120;
    cap.signature = SigningKey::from_bytes(&[1; 32])
        .sign(&cap.signing_bytes())
        .to_bytes()
        .to_vec();
    let mut c = client(cap, token(), 100);
    c.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    c.connected(UnixTimestamp::from_secs(101)).unwrap();
    c.transport_lost(UnixTimestamp::from_secs(102)).unwrap();
    assert!(matches!(
        c.begin_connect(UnixTimestamp::from_secs(150)),
        Err(ClientError::CapabilityExpired)
    ));
}

#[test]
fn classifies_relay_expiry() {
    let mut t = token();
    t.expires_at = UnixTimestamp::from_secs(102);
    t.signature = SigningKey::from_bytes(&[2; 32])
        .sign(&t.signing_bytes())
        .to_bytes()
        .to_vec();
    let mut c = client(capability(), t, 100);
    c.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    c.connected(UnixTimestamp::from_secs(101)).unwrap();
    c.transport_lost(UnixTimestamp::from_secs(102)).unwrap();
    assert!(matches!(
        c.begin_connect(UnixTimestamp::from_secs(105)),
        Err(ClientError::RelayTokenExpired)
    ));
}

#[test]
fn rejects_deadline_overrun_at_connected() {
    let mut c = client(capability(), token(), 100);
    c.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    c.connected(UnixTimestamp::from_secs(101)).unwrap();
    c.transport_lost(UnixTimestamp::from_secs(102)).unwrap();
    c.begin_connect(UnixTimestamp::from_secs(110)).unwrap();
    assert!(matches!(
        c.connected(UnixTimestamp::from_secs(163)),
        Err(ClientError::ReconnectWindowElapsed)
    ));
    assert_eq!(c.state(), ClientState::Expired);
}
