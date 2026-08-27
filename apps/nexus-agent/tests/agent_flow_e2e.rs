//! Integration test verifying host agent auto-enrollment against nexusd and session verification.
//! Part of Nexus Remote Desktop Platform.

use ed25519_dalek::SigningKey;
use nexus_agent::enroll::EnrollmentClient;
use nexus_agent::identity::AgentIdentity;
use nexus_agent::session_manager::AgentSessionManager;
use nexus_auth::enrollment::{DeviceType, EnrollmentToken};
use nexus_common::id::TenantId;
use nexus_common::time::UnixTimestamp;
use nexusd::routes::{SessionAuthorizationResponse, SessionRequestPayload};
use nexusd::server::ControlPlaneServer;
use nexusd::state::AppState;
use nexusd::{DatabaseConfig, SqliteStorage};
use prost::Message;
use tempfile::tempdir;

#[tokio::test]
async fn test_agent_auto_enroll_and_session_acceptance_flow() {
    // 1. Setup Control Plane (nexusd) in memory
    let cp_signing_key = SigningKey::from_bytes(&[89u8; 32]);
    let cp_verifying_key = cp_signing_key.verifying_key();
    let org_id = TenantId::new("org-agent-corp").unwrap();
    let database_dir = tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("control-plane.db").display()
    );
    let storage = SqliteStorage::connect(&DatabaseConfig::sqlite(database_url))
        .await
        .unwrap();

    let cp_state = AppState::new(cp_signing_key.clone(), "nexus-cp-host-testing", storage)
        .with_default_relay_id("relay-agent-test");

    // 2. Issue EnrollmentToken in Control Plane
    let mut enroll_token = EnrollmentToken::builder()
        .token_id("tok-enroll-agent-01")
        .organization_id(org_id.clone())
        .device_type(DeviceType::Host)
        .expires_at(UnixTimestamp::from_secs(2_500_000_000))
        .max_uses(1)
        .build()
        .unwrap();
    enroll_token.sign(&cp_signing_key).unwrap();
    cp_state
        .store_enrollment_token(enroll_token.clone())
        .await
        .unwrap();

    // 3. Start Control Plane HTTP Server
    let cp_server = ControlPlaneServer::bind("127.0.0.1:0".parse().unwrap(), cp_state.clone())
        .await
        .unwrap();
    let cp_addr = cp_server.local_addr().unwrap();
    let _cp_handle = tokio::spawn(async move {
        cp_server.run().await.unwrap();
    });

    let cp_base_url = format!("http://{}", cp_addr);

    // 4. Host Agent initializes identity and performs remote enrollment
    let temp_dir = tempdir().unwrap();
    let identity_path = temp_dir.path().join("agent_identity.json");
    let mut agent_identity = AgentIdentity::load_or_generate(&identity_path).unwrap();

    assert!(agent_identity.credential().is_none());

    let enroll_client = EnrollmentClient::new(&cp_base_url);
    let credential = enroll_client
        .enroll(
            &mut agent_identity,
            enroll_token,
            "AGENT-WIN-WORKSTATION",
            "windows",
            "x86_64",
            "0.1.0",
        )
        .await
        .unwrap();

    assert_eq!(credential.organization_id, org_id);
    assert!(agent_identity.credential().is_some());

    // 5. Host Agent initializes its SessionManager
    let mut session_manager =
        AgentSessionManager::new(cp_verifying_key, credential.device_id.to_string());

    // 6. Client requests session from Control Plane
    let client_http = reqwest::Client::new();
    let session_req = SessionRequestPayload {
        organization_id: org_id,
        subject_user_id: nexus_common::id::UserId::new("user-dev-bob").unwrap(),
        client_device_id: nexus_common::id::DeviceId::new("dev-client-laptop-bob").unwrap(),
        target_device_id: credential.device_id.clone(),
        requested_actions: vec!["desktop.view".into(), "desktop.control".into()],
        client_ephemeral_public_key: vec![44u8; 32],
    };

    let resp = client_http
        .post(format!("{}/api/v1/sessions/request", cp_base_url))
        .json(&session_req)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let auth_resp: SessionAuthorizationResponse = resp.json().await.unwrap();

    // 7. Decode capability and present to AgentSessionManager
    let capability =
        nexus_protocol::SessionCapability::decode(auth_resp.capability_bytes.as_slice()).unwrap();

    let accepted_session = session_manager
        .verify_and_accept_session(&capability, 1)
        .unwrap();

    assert_eq!(accepted_session.session_id, auth_resp.session_id);
    assert_eq!(session_manager.active_session_count(), 1);

    // 8. Replaying the same capability must be rejected by replay defense
    let replay_err = session_manager.verify_and_accept_session(&capability, 1);
    assert!(replay_err.is_err());

    // 9. Terminate session
    let terminated = session_manager
        .terminate_session(&accepted_session.session_id)
        .unwrap();
    assert_eq!(terminated.session_id, accepted_session.session_id);
    assert_eq!(session_manager.active_session_count(), 0);
}
