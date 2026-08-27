use nexusd::{DatabaseConfig, DatabaseDriver, SqliteStorage, StorageError};

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
