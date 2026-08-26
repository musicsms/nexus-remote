//! End-to-end integration test for device enrollment, credential issuance, and session capability authorization.
//! Part of Nexus Remote Desktop Platform.

use ed25519_dalek::{Signer, SigningKey};
use nexus_auth::credential::{DeviceCredential, DeviceRegistrationRequest};
use nexus_auth::enrollment::{DeviceType, EnrollmentToken};
use nexus_auth::verifier::CapabilityVerifier;
use nexus_common::id::{DeviceId, TenantId};
use nexus_common::time::UnixTimestamp;
use nexus_protocol::SessionCapability;
use prost::Message;
use std::time::{Duration, Instant};

#[test]
fn test_full_device_enrollment_and_capability_flow() {
    // 1. Control Plane generates its master signing key
    let cp_signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let cp_verifying_key = cp_signing_key.verifying_key();
    let org_id = TenantId::new("tenant-nexus-alpha").unwrap();

    // 2. Control Plane issues one-time EnrollmentToken for a new Windows Host
    let mut enrollment_token = EnrollmentToken::builder()
        .token_id("enroll-token-host-001")
        .organization_id(org_id.clone())
        .device_type(DeviceType::Host)
        .not_before(UnixTimestamp::from_secs(1_000))
        .expires_at(UnixTimestamp::from_secs(2_000))
        .max_uses(1)
        .build()
        .unwrap();
    enrollment_token.sign(&cp_signing_key).unwrap();

    // 3. Host Agent generates its local private key during installation
    let host_device_key = SigningKey::from_bytes(&[88u8; 32]);
    let host_device_pub = host_device_key.verifying_key().to_bytes().to_vec();

    // 4. Host Agent creates registration request with proof-of-possession
    let mut reg_req = DeviceRegistrationRequest {
        enrollment_token: enrollment_token.clone(),
        device_public_key: host_device_pub.clone(),
        os: "windows".into(),
        architecture: "x86_64".into(),
        hostname: "HOST-WIN-DESKTOP".into(),
        agent_version: "0.1.0".into(),
        requested_at: UnixTimestamp::from_secs(1_500),
        proof_signature: Vec::new(),
    };
    reg_req.sign_proof(&host_device_key);

    // 5. Control Plane validates enrollment token and proof of possession
    reg_req
        .enrollment_token
        .verify(&cp_verifying_key, UnixTimestamp::from_secs(1_500))
        .unwrap();
    reg_req.verify_proof().unwrap();

    // 6. Control Plane issues signed DeviceCredential to the Host
    let assigned_device_id = DeviceId::new("dev-host-assigned-001").unwrap();
    let mut host_cred = DeviceCredential::builder()
        .device_id(assigned_device_id.clone())
        .organization_id(org_id)
        .public_key(host_device_pub)
        .device_type(DeviceType::Host)
        .os("windows")
        .architecture("x86_64")
        .capabilities(vec!["desktop.view".into(), "desktop.control".into()])
        .issued_at(UnixTimestamp::from_secs(1_500))
        .expires_at(UnixTimestamp::from_secs(100_000))
        .build()
        .unwrap();
    host_cred.sign(&cp_signing_key).unwrap();

    // Host validates credential from Control Plane
    host_cred
        .verify(&cp_verifying_key, UnixTimestamp::from_secs(1_500))
        .unwrap();

    // 7. Client requests session; Control Plane generates signed SessionCapability
    let mut session_cap = SessionCapability {
        version: 1,
        issuer: "control-plane".into(),
        session_id: "sess-live-001".into(),
        subject_user_id: "user-admin".into(),
        client_device_id: "dev-client-laptop".into(),
        target_device_id: assigned_device_id.to_string(),
        permissions: vec!["desktop.view".into(), "desktop.control".into()],
        restrictions: vec![],
        not_before: 1_500,
        expires_at: 1_600,
        nonce: vec![1, 2, 3, 4, 5, 6, 7, 8],
        agent_min_protocol: 1,
        agent_max_protocol: 1,
        client_ephemeral_public_key: vec![9u8; 32],
        signature: Vec::new(),
    };

    let mut unsigned_cap_bytes = Vec::new();
    session_cap.encode(&mut unsigned_cap_bytes).unwrap();
    let cap_sig = cp_signing_key.sign(&unsigned_cap_bytes);
    session_cap.signature = cap_sig.to_bytes().to_vec();

    // 8. Host Agent verifies capability upon receiving SessionHello
    let mut verifier = CapabilityVerifier::new(cp_verifying_key, Duration::from_secs(60), 100)
        .with_target_device_id(assigned_device_id.to_string());

    let now_ts = UnixTimestamp::from_secs(1_550);
    let inst_now = Instant::now();

    assert!(verifier.verify(&session_cap, 1, now_ts, inst_now).is_ok());
}
