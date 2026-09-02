use nexus_client::session::{ClientError, ClientSession, ClientState, RelayTokenMetadata};
use nexus_common::{MockClock, UnixTimestamp};
use nexus_protocol::SessionCapability;

fn capability() -> SessionCapability {
    SessionCapability {
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
    }
}

fn token() -> RelayTokenMetadata {
    RelayTokenMetadata {
        session_id: "session-1".into(),
        client_device_id: "client-1".into(),
        target_device_id: "host-1".into(),
        expires_at: UnixTimestamp::from_secs(200),
    }
}

#[test]
fn follows_lifecycle_and_expires() {
    let mut client = ClientSession::new(capability(), token(), MockClock::from_secs(100));
    assert_eq!(client.state(), ClientState::Disconnected);
    client.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    client.connected(UnixTimestamp::from_secs(101)).unwrap();
    client
        .transport_lost(UnixTimestamp::from_secs(102))
        .unwrap();
    assert_eq!(client.state(), ClientState::Reconnecting);
    assert!(client.reconnect_deadline().is_some());
    client.expire().unwrap();
    assert_eq!(client.state(), ClientState::Expired);
}

#[test]
fn rejects_skipped_and_expired_transitions() {
    let mut client = ClientSession::new(capability(), token(), MockClock::from_secs(100));
    assert!(matches!(
        client.connected(UnixTimestamp::from_secs(100)),
        Err(ClientError::InvalidTransition { .. })
    ));
    client.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    client.connected(UnixTimestamp::from_secs(101)).unwrap();
    client.expire().unwrap();
    assert!(matches!(
        client.transport_lost(UnixTimestamp::from_secs(102)),
        Err(ClientError::Expired)
    ));
}

#[test]
fn validates_identity_and_capability_window() {
    let mut bad_token = token();
    bad_token.session_id = "other".into();
    let mut client = ClientSession::new(capability(), bad_token, MockClock::from_secs(100));
    assert!(matches!(
        client.begin_connect(UnixTimestamp::from_secs(100)),
        Err(ClientError::IdentityMismatch)
    ));

    let mut expired = capability();
    expired.not_before = 150;
    let mut client = ClientSession::new(expired, token(), MockClock::from_secs(100));
    assert!(matches!(
        client.begin_connect(UnixTimestamp::from_secs(100)),
        Err(ClientError::CapabilityNotActive)
    ));
}

#[test]
fn reconnect_deadline_is_inclusive_and_established_duration_does_not_reset() {
    let mut client = ClientSession::new(capability(), token(), MockClock::from_secs(100));
    client.begin_connect(UnixTimestamp::from_secs(100)).unwrap();
    client.connected(UnixTimestamp::from_secs(101)).unwrap();
    client
        .transport_lost(UnixTimestamp::from_secs(102))
        .unwrap();
    assert!(client.can_reconnect(UnixTimestamp::from_secs(162)));
    assert!(!client.can_reconnect(UnixTimestamp::from_secs(163)));
    client.begin_connect(UnixTimestamp::from_secs(110)).unwrap();
    client.connected(UnixTimestamp::from_secs(111)).unwrap();
    assert!(client.session_duration_expired(UnixTimestamp::from_secs(1901)));
}
