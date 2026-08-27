use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use nexus_audit::sink::MemoryAuditSink;
use nexus_auth::credential::DeviceRegistrationRequest;
use nexus_auth::enrollment::{DeviceType, EnrollmentToken};
use nexus_auth::verifier::CapabilityVerifier;
use nexus_common::id::{DeviceId, TenantId, UserId};
use nexus_common::time::{Clock, SystemClock, UnixTimestamp};
use nexus_protocol::SessionCapability;
use nexus_relay::token::RelayTokenVerifier;
use nexusd::routes::{HealthResponse, SessionAuthorizationResponse, SessionRequestPayload};
use nexusd::server::ControlPlaneServer;
use nexusd::state::AppState;
use nexusd::{DatabaseConfig, SqliteStorage};
use prost::Message;

#[tokio::test]
async fn test_control_plane_api_full_e2e_flow() {
    // 1. Setup server state with signing key and audit sink
    let cp_signing_key = SigningKey::from_bytes(&[55u8; 32]);
    let cp_verifying_key = cp_signing_key.verifying_key();
    let audit_sink = Arc::new(MemoryAuditSink::new());
    let database_dir = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database_dir.path().join("control-plane.db").display()
    );
    let storage = SqliteStorage::connect(&DatabaseConfig::sqlite(database_url.clone()))
        .await
        .unwrap();

    let state = AppState::new(cp_signing_key.clone(), "nexus-cp-e2e-cluster", storage)
        .with_default_relay_id("relay-e2e-01")
        .with_audit_sink(audit_sink.clone());

    // 2. Issue and store enrollment token for Host
    let org_id = TenantId::new("org-tech-corp").unwrap();
    let mut enroll_token = EnrollmentToken::builder()
        .token_id("tok-enroll-win-host-1")
        .organization_id(org_id.clone())
        .device_type(DeviceType::Host)
        .expires_at(UnixTimestamp::from_secs(2_000_000_000))
        .max_uses(1)
        .build()
        .unwrap();
    enroll_token.sign(&cp_signing_key).unwrap();
    state
        .store_enrollment_token(enroll_token.clone())
        .await
        .unwrap();

    // 3. Bind and run ControlPlaneServer
    let server = ControlPlaneServer::bind("127.0.0.1:0".parse().unwrap(), state.clone())
        .await
        .unwrap();
    let server_addr = server.local_addr().unwrap();

    let _server_handle = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://{}", server_addr);

    // 4. Test GET /healthz
    let res = client
        .get(format!("{}/healthz", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let health: HealthResponse = res.json().await.unwrap();
    assert_eq!(health.status, "ok");
    assert_eq!(health.control_plane_id, "nexus-cp-e2e-cluster");

    // 5. Host prepares registration request with proof-of-possession
    let host_signing_key = SigningKey::from_bytes(&[77u8; 32]);
    let host_pub_key = host_signing_key.verifying_key().to_bytes().to_vec();

    let mut reg_req = DeviceRegistrationRequest {
        enrollment_token: enroll_token,
        device_public_key: host_pub_key,
        os: "windows".into(),
        architecture: "x86_64".into(),
        hostname: "WIN-WORKSTATION-01".into(),
        agent_version: "0.1.0".into(),
        requested_at: SystemClock.now(),
        proof_signature: Vec::new(),
    };
    reg_req.sign_proof(&host_signing_key);

    // 6. Test POST /api/v1/devices/enroll
    let res = client
        .post(format!("{}/api/v1/devices/enroll", base_url))
        .json(&reg_req)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let cred: nexus_auth::credential::DeviceCredential = res.json().await.unwrap();
    assert_eq!(cred.organization_id, org_id);
    assert_eq!(cred.device_type, DeviceType::Host);
    cred.verify(&cp_verifying_key, SystemClock.now()).unwrap();

    let host_device_id = cred.device_id;

    // Second enrollment with same token must fail (max_uses exceeded)
    let res_duplicate = client
        .post(format!("{}/api/v1/devices/enroll", base_url))
        .json(&reg_req)
        .send()
        .await
        .unwrap();
    assert_eq!(res_duplicate.status(), reqwest::StatusCode::FORBIDDEN);

    // 7. Test POST /api/v1/sessions/request
    let client_device_id = DeviceId::new("dev-client-laptop-01").unwrap();
    let user_id = UserId::new("user-engineer-alice").unwrap();

    let session_req = SessionRequestPayload {
        organization_id: org_id.clone(),
        subject_user_id: user_id,
        client_device_id: client_device_id.clone(),
        target_device_id: host_device_id.clone(),
        requested_actions: vec!["desktop.view".into(), "desktop.control".into()],
        client_ephemeral_public_key: vec![33u8; 32],
    };

    let res = client
        .post(format!("{}/api/v1/sessions/request", base_url))
        .json(&session_req)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let auth_resp: SessionAuthorizationResponse = res.json().await.unwrap();

    assert_eq!(auth_resp.relay_id, "relay-e2e-01");

    let capability = SessionCapability::decode(auth_resp.capability_bytes.as_slice()).unwrap();
    assert_eq!(capability.target_device_id, host_device_id.as_str());
    assert_eq!(capability.client_device_id, client_device_id.as_str());

    // 8. Verify capability using CapabilityVerifier
    let mut verifier = CapabilityVerifier::new(cp_verifying_key, Duration::from_secs(60), 100)
        .with_target_device_id(host_device_id.as_str());
    assert!(verifier
        .verify(&capability, 1, SystemClock.now(), std::time::Instant::now())
        .is_ok());

    // 9. Verify Relay Tokens using RelayTokenVerifier
    let relay_verifier = RelayTokenVerifier::new(cp_verifying_key, "relay-e2e-01");
    assert!(relay_verifier
        .verify(&auth_resp.client_relay_token, SystemClock.now())
        .is_ok());
    assert!(relay_verifier
        .verify(&auth_resp.host_relay_token, SystemClock.now())
        .is_ok());

    // 10. Verify audit records generated and durable across a new connection.
    let audit_events = audit_sink.events();
    assert_eq!(audit_events.len(), 2);

    let restarted_storage = SqliteStorage::connect(&DatabaseConfig::sqlite(database_url))
        .await
        .unwrap();
    assert!(restarted_storage
        .get_device(&host_device_id)
        .await
        .unwrap()
        .is_some());
    assert!(restarted_storage
        .get_session(&auth_resp.session_id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        restarted_storage
            .list_audit_events(&org_id)
            .await
            .unwrap()
            .len(),
        2
    );
}
