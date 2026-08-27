use nexus_audit::{verify_chain, AuditChain, AuditEvent, AuditEventType};
use nexus_auth::{DeviceCredential, DeviceType, EnrollmentToken};
use nexus_common::id::{DeviceId, SessionId, TenantId, UserId};
use nexus_common::time::UnixTimestamp;
use nexusd::state::RegisteredDevice;
use nexusd::storage::EnrollmentError;
use nexusd::{DatabaseConfig, DatabaseDriver, SqliteStorage, StorageError};

async fn storage() -> SqliteStorage {
    let temp = tempfile::tempdir().unwrap();
    // Keep the directory alive for the duration of the process via a leaked path.
    let path = temp.path().to_owned();
    std::mem::forget(temp);
    SqliteStorage::connect(&DatabaseConfig::sqlite(format!(
        "sqlite://{}?mode=rwc",
        path.join("nexus.db").display()
    )))
    .await
    .unwrap()
}

fn token(org: &TenantId) -> EnrollmentToken {
    EnrollmentToken::builder()
        .token_id("tok-1")
        .organization_id(org.clone())
        .device_type(DeviceType::Host)
        .not_before(UnixTimestamp::from_secs(0))
        .expires_at(UnixTimestamp::from_secs(1000))
        .max_uses(1)
        .build()
        .unwrap()
}

fn device(org: &TenantId, id: &str) -> RegisteredDevice {
    let cred = DeviceCredential::builder()
        .device_id(DeviceId::new(id).unwrap())
        .organization_id(org.clone())
        .public_key(vec![1, 2, 3])
        .device_type(DeviceType::Host)
        .os("linux")
        .architecture("x86_64")
        .issued_at(UnixTimestamp::from_secs(1))
        .expires_at(UnixTimestamp::from_secs(1000))
        .build()
        .unwrap();
    RegisteredDevice {
        credential: cred,
        hostname: id.into(),
        agent_version: "1.0".into(),
        enrolled_at: UnixTimestamp::from_secs(1),
        is_active: true,
    }
}

#[tokio::test]
async fn enrollment_persists_and_consumes_atomically() {
    let storage = storage().await;
    let org = TenantId::new("org-test").unwrap();
    let t = token(&org);
    storage.store_enrollment_token(&t).await.unwrap();
    let enrolled = storage
        .enroll_device("tok-1", UnixTimestamp::from_secs(1), device(&org, "dev-1"))
        .await
        .unwrap();
    assert_eq!(enrolled.token_id, "tok-1");
    assert_eq!(storage.count_devices().await.unwrap(), 1);
    assert!(matches!(
        storage
            .enroll_device("tok-1", UnixTimestamp::from_secs(1), device(&org, "dev-2"))
            .await,
        Err(EnrollmentError::Exhausted { .. })
    ));
    assert!(storage
        .get_device(&DeviceId::new("dev-1").unwrap())
        .await
        .unwrap()
        .is_some());
    assert_eq!(storage.list_devices(&org).await.unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_final_use_allows_exactly_one_device() {
    let storage = storage().await;
    let org = TenantId::new("org-race").unwrap();
    storage.store_enrollment_token(&token(&org)).await.unwrap();
    let first = storage.clone();
    let second = storage.clone();
    let (a, b) = tokio::join!(
        first.enroll_device("tok-1", UnixTimestamp::from_secs(1), device(&org, "dev-a")),
        second.enroll_device("tok-1", UnixTimestamp::from_secs(1), device(&org, "dev-b")),
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(storage.count_devices().await.unwrap(), 1);
    assert!(matches!(
        a.as_ref().err().or(b.as_ref().err()),
        Some(EnrollmentError::Exhausted { .. })
    ));
}

#[tokio::test]
async fn enrollment_rejects_not_before_and_tenant_mismatch_without_consuming() {
    let storage = storage().await;
    let org = TenantId::new("org-window").unwrap();
    let mut t = token(&org);
    t.not_before = UnixTimestamp::from_secs(100);
    storage.store_enrollment_token(&t).await.unwrap();
    assert!(matches!(
        storage
            .enroll_device("tok-1", UnixTimestamp::from_secs(1), device(&org, "dev-a"))
            .await,
        Err(EnrollmentError::NotYetActive { .. })
    ));
    let other = TenantId::new("org-other").unwrap();
    assert!(matches!(
        storage
            .enroll_device(
                "tok-1",
                UnixTimestamp::from_secs(100),
                device(&other, "dev-b")
            )
            .await,
        Err(EnrollmentError::Storage(_))
    ));
    assert_eq!(storage.count_devices().await.unwrap(), 0);
}

#[tokio::test]
async fn fresh_database_runs_foundation_migration() {
    let temp = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        temp.path().join("nexus.db").display()
    );
    let storage = SqliteStorage::connect(&DatabaseConfig::sqlite(url))
        .await
        .unwrap();
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(storage.pool())
            .await
            .unwrap();
    for expected in [
        "audit_events",
        "device_credentials",
        "devices",
        "enrollment_tokens",
        "organizations",
        "sessions",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
}

#[test]
fn zero_max_connections_is_rejected() {
    let config = DatabaseConfig {
        driver: DatabaseDriver::Sqlite,
        url: "sqlite::memory:".into(),
        max_connections: 0,
    };
    assert!(matches!(
        config.validate(),
        Err(StorageError::InvalidConfig(_))
    ));
}

#[tokio::test]
async fn postgres_is_explicitly_unsupported() {
    let config = DatabaseConfig {
        driver: DatabaseDriver::Postgres,
        url: "postgres://localhost/nexus".into(),
        max_connections: 1,
    };
    assert!(matches!(
        SqliteStorage::connect(&config).await,
        Err(StorageError::UnsupportedDriver("postgres"))
    ));
}

#[tokio::test]
async fn session_and_audit_round_trip() {
    let storage = storage().await;
    let org = TenantId::new("org-session").unwrap();
    sqlx::query("INSERT INTO organizations (id, created_at) VALUES (?, ?)")
        .bind(org.as_str())
        .bind(1i64)
        .execute(storage.pool())
        .await
        .unwrap();
    for device_id in ["client-1", "target-1"] {
        sqlx::query("INSERT INTO devices (id, organization_id, hostname, os, os_version, architecture, agent_version, public_key, status, capabilities_json, created_at) VALUES (?, ?, '', '', '', '', '', X'', 'active', '[]', 1)")
            .bind(device_id).bind(org.as_str()).execute(storage.pool()).await.unwrap();
    }
    let record = nexusd::storage::AuthorizedSessionRecord {
        session_id: SessionId::new("sess-1").unwrap(),
        organization_id: org.clone(),
        user_id: UserId::new("user-1").unwrap(),
        client_device_id: DeviceId::new("client-1").unwrap(),
        target_device_id: DeviceId::new("target-1").unwrap(),
        relay_id: "relay-1".into(),
        permissions: vec!["desktop.view".into(), "desktop.control".into()],
        created_at: UnixTimestamp::from_secs(42),
        status: "authorized".into(),
    };
    storage.insert_authorized_session(&record).await.unwrap();
    assert_eq!(
        storage.get_session(&record.session_id).await.unwrap(),
        Some(record)
    );

    let mut chain = AuditChain::new(None);
    let first = chain
        .append(AuditEvent::new(
            "evt-1",
            UnixTimestamp::from_secs(43),
            org.clone(),
            AuditEventType::SessionAuthorize,
        ))
        .unwrap();
    let second = chain
        .append(AuditEvent::new(
            "evt-2",
            UnixTimestamp::from_secs(44),
            org.clone(),
            AuditEventType::SessionStart,
        ))
        .unwrap();
    storage.insert_audit_event(&first).await.unwrap();
    storage.insert_audit_event(&second).await.unwrap();
    let loaded = storage.list_audit_events(&org).await.unwrap();
    assert_eq!(loaded, vec![first, second]);
    assert!(verify_chain(&loaded, None).is_ok());
}
