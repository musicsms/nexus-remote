# nexusd SQLite Persistence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the existing nexusd enrollment, device, session-authorization, and audit flows in SQLite with transactional token consumption and restart-safe data.

**Architecture:** Axum routes continue to call async domain operations on `AppState`; `AppState` delegates durable operations to a focused SQLx/SQLite storage module. Schema creation is migration-driven, privileged writes fail closed, and in-memory SQLite remains available only as a test fixture.

**Tech Stack:** Rust 2021, Tokio, Axum 0.7, SQLx 0.8 (`sqlite`, `runtime-tokio-rustls`, `migrate`, `macros`), Serde JSON, tempfile, existing Nexus domain crates.

**Spec:** `docs/superpowers/specs/2026-08-27-nexusd-sqlite-persistence-design.md`

## Global Constraints

- SQLite is the only runtime backend implemented in this slice; selecting `postgres` returns an explicit unsupported-driver error.
- Schema types and SQL must avoid backend-specific arrays, JSON types, generated columns, and SQLite dynamic-typing assumptions.
- Enrollment token consumption and device/credential insertion are one transaction.
- Session authorization is persisted before an authorization response is returned.
- Required audit persistence and the injected audit sink both fail closed.
- No blocking I/O may run on Tokio worker threads.
- Every storage-facing parser validates serialized rows instead of applying defaults.
- Update `docs/IMPLEMENTATION_STATUS.md`, `README.md`, and `CLAUDE.md` when implementation status changes.

## File Map

- Create `migrations/0001_control_plane_foundation.sql`: portable initial control-plane schema.
- Create `apps/nexusd/src/config.rs`: database driver/config validation.
- Create `apps/nexusd/src/storage/mod.rs`: storage facade, errors, connection and migrations.
- Create `apps/nexusd/src/storage/enrollment.rs`: enrollment-token and atomic enrollment operations.
- Create `apps/nexusd/src/storage/device.rs`: registered-device row conversion and queries.
- Create `apps/nexusd/src/storage/session.rs`: authorized-session persistence model and insert/query operations.
- Create `apps/nexusd/src/storage/audit.rs`: chained-audit persistence and queries.
- Create `apps/nexusd/tests/storage_sqlite.rs`: migration, storage, concurrency, corruption, and restart tests.
- Modify `Cargo.toml`: declare SQLx workspace dependency.
- Modify `apps/nexusd/Cargo.toml`: consume SQLx and tempfile.
- Modify `apps/nexusd/src/lib.rs`: export config/storage interfaces.
- Modify `apps/nexusd/src/state.rs`: replace in-memory registries with async storage operations.
- Modify `apps/nexusd/src/routes.rs`: await state operations and persist sessions/audits.
- Modify `apps/nexusd/src/main.rs`: initialize config, database, migrations, and state before binding.
- Modify `apps/nexusd/tests/control_plane_api_e2e.rs`: run the API flow against SQLite.
- Modify `docs/IMPLEMENTATION_STATUS.md`, `README.md`, `CLAUDE.md`: align status documentation.

---

### Task 1: Database Configuration, Migration, and Connection

**Files:**
- Create: `migrations/0001_control_plane_foundation.sql`
- Create: `apps/nexusd/src/config.rs`
- Create: `apps/nexusd/src/storage/mod.rs`
- Create: `apps/nexusd/tests/storage_sqlite.rs`
- Modify: `Cargo.toml`
- Modify: `apps/nexusd/Cargo.toml`
- Modify: `apps/nexusd/src/lib.rs`

**Interfaces:**
- Produces: `DatabaseDriver::{Sqlite, Postgres}`.
- Produces: `DatabaseConfig { driver, url, max_connections }` and `DatabaseConfig::sqlite(url)`.
- Produces: `SqliteStorage::connect(&DatabaseConfig) -> Result<SqliteStorage, StorageError>`.
- Produces: `SqliteStorage::pool(&self) -> &SqlitePool` for module-local queries and test inspection.

- [ ] **Step 1: Add the SQLx dependencies**

Add to root `[workspace.dependencies]`:

```toml
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio-rustls", "sqlite", "migrate", "macros"] }
```

Add `sqlx.workspace = true` to nexusd dependencies and `tempfile.workspace = true` to nexusd dev-dependencies.

- [ ] **Step 2: Write failing configuration and migration tests**

Create tests that assert a fresh temporary SQLite file migrates, all six tables exist through `sqlite_master`, invalid `max_connections == 0` is rejected, and `DatabaseDriver::Postgres` returns `StorageError::UnsupportedDriver`:

```rust
#[tokio::test]
async fn fresh_database_runs_foundation_migration() {
    let temp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", temp.path().join("nexus.db").display());
    let storage = SqliteStorage::connect(&DatabaseConfig::sqlite(url)).await.unwrap();
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
    )
    .fetch_all(storage.pool()).await.unwrap();
    for expected in ["audit_events", "device_credentials", "devices",
        "enrollment_tokens", "organizations", "sessions"] {
        assert!(names.iter().any(|name| name == expected), "missing {expected}");
    }
}
```

- [ ] **Step 3: Run the focused test and verify failure**

Run: `cargo test -p nexusd --test storage_sqlite fresh_database_runs_foundation_migration`

Expected: compilation fails because `config` and `storage` do not exist.

- [ ] **Step 4: Implement configuration, storage connection, and migration**

Define exact public types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseDriver { Sqlite, Postgres }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub driver: DatabaseDriver,
    pub url: String,
    pub max_connections: u32,
}

impl DatabaseConfig {
    pub fn sqlite(url: impl Into<String>) -> Self;
    pub fn validate(&self) -> Result<(), StorageError>;
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("unsupported database driver: {0}")]
    UnsupportedDriver(&'static str),
    #[error("invalid database configuration: {0}")]
    InvalidConfig(String),
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("stored data is invalid: {0}")]
    CorruptRow(String),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

Use `SqlitePoolOptions::new().max_connections(config.max_connections)` and `sqlx::migrate!("../../migrations").run(&pool).await`. The migration uses `TEXT`, `BIGINT`, `INTEGER`, and `BLOB`, enables foreign keys, and defines explicit primary/foreign keys and indexes for organization/device/session lookup.

- [ ] **Step 5: Run migration tests and workspace formatting**

Run: `cargo test -p nexusd --test storage_sqlite fresh_database_runs_foundation_migration`

Expected: PASS.

Run: `cargo fmt --all -- --check`

Expected: PASS after applying `cargo fmt --all` if needed.

- [ ] **Step 6: Commit Task 1**

```bash
git add Cargo.toml Cargo.lock apps/nexusd/Cargo.toml apps/nexusd/src/lib.rs apps/nexusd/src/config.rs apps/nexusd/src/storage/mod.rs apps/nexusd/tests/storage_sqlite.rs migrations/0001_control_plane_foundation.sql
git commit -m "feat(control-plane): initialize SQLite storage"
```

### Task 2: Transactional Enrollment and Device Repository

**Files:**
- Create: `apps/nexusd/src/storage/enrollment.rs`
- Create: `apps/nexusd/src/storage/device.rs`
- Modify: `apps/nexusd/src/storage/mod.rs`
- Modify: `apps/nexusd/tests/storage_sqlite.rs`

**Interfaces:**
- Consumes: `SqliteStorage`, `StorageError`, `RegisteredDevice`, `EnrollmentToken`, `DeviceCredential`.
- Produces: `SqliteStorage::store_enrollment_token(&EnrollmentToken) -> Result<(), StorageError>`.
- Produces: `SqliteStorage::enroll_device(&str, UnixTimestamp, RegisteredDevice) -> Result<EnrollmentToken, EnrollmentError>`.
- Produces: async `get_device`, `list_devices`, and `count_devices` methods.
- Produces: `EnrollmentError::{NotFound, Expired, Exhausted, Storage}` with the same domain details as current `StateError`.

- [ ] **Step 1: Write failing repository tests**

Add tests for storing/loading a token, atomic device plus credential insertion, tenant filtering, and persistence after reconnect. Add a two-task race using `tokio::join!`:

```rust
let first = storage.clone();
let second = storage.clone();
let (a, b) = tokio::join!(
    first.enroll_device(token.token_id.as_str(), now, device_a),
    second.enroll_device(token.token_id.as_str(), now, device_b),
);
assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
assert_eq!(storage.count_devices().await.unwrap(), 1);
assert!(matches!(a.as_ref().err().or(b.as_ref().err()), Some(EnrollmentError::Exhausted { .. })));
```

Insert a malformed credential JSON row with raw SQL and assert `get_device` returns `StorageError::CorruptRow`.

- [ ] **Step 2: Run repository tests and verify failure**

Run: `cargo test -p nexusd --test storage_sqlite enrollment`

Expected: compilation fails because enrollment/device repository methods do not exist.

- [ ] **Step 3: Implement row conversion and token storage**

Serialize the signed token and credential with `serde_json::to_string`. Convert database `BIGINT` timestamps with checked `u64::try_from`; validate stored IDs with `TenantId::new` and `DeviceId::new`. Map every failure to `StorageError::CorruptRow` with the offending field name.

- [ ] **Step 4: Implement atomic enrollment transaction**

Within one SQLx transaction:

1. Fetch and deserialize the token.
2. Return `NotFound`, `Expired`, or `Exhausted` before mutation.
3. Execute `UPDATE enrollment_tokens SET uses_count = uses_count + 1 WHERE token_id = ? AND uses_count < max_uses AND expires_at >= ?` and require one affected row.
4. Insert the organization with conflict-ignore semantics.
5. Insert `devices` and `device_credentials`.
6. Commit and return the token.

Keep transaction SQL in `storage/enrollment.rs`; keep row-to-domain conversion in `storage/device.rs`.

- [ ] **Step 5: Run all storage tests**

Run: `cargo test -p nexusd --test storage_sqlite`

Expected: PASS, including exactly one successful concurrent final-use enrollment.

- [ ] **Step 6: Commit Task 2**

```bash
git add apps/nexusd/src/storage apps/nexusd/tests/storage_sqlite.rs
git commit -m "feat(control-plane): persist enrollment and devices"
```

### Task 3: Session and Audit Persistence

**Files:**
- Create: `apps/nexusd/src/storage/session.rs`
- Create: `apps/nexusd/src/storage/audit.rs`
- Modify: `apps/nexusd/src/storage/mod.rs`
- Modify: `apps/nexusd/tests/storage_sqlite.rs`

**Interfaces:**
- Produces: `AuthorizedSessionRecord` containing session/tenant/user/client/target IDs, relay ID, permissions, creation time, and status `authorized`.
- Produces: `SqliteStorage::insert_authorized_session(&AuthorizedSessionRecord) -> Result<(), StorageError>`.
- Produces: `SqliteStorage::get_session(&SessionId) -> Result<Option<AuthorizedSessionRecord>, StorageError>`.
- Produces: `SqliteStorage::insert_audit_event(&ChainedAuditEvent) -> Result<(), StorageError>`.
- Produces: `SqliteStorage::list_audit_events(&TenantId) -> Result<Vec<ChainedAuditEvent>, StorageError>`.

- [ ] **Step 1: Write failing session/audit tests**

Create an `AuthorizedSessionRecord`, insert/load it, and assert every ID, relay, permission, and timestamp round-trips. Append two events using `AuditChain`, persist them, reload by organization, and assert equality plus `verify_chain(&loaded, None).is_ok()`.

- [ ] **Step 2: Run focused tests and verify failure**

Run: `cargo test -p nexusd --test storage_sqlite session_and_audit`

Expected: compilation fails because the record and repository methods are missing.

- [ ] **Step 3: Implement session persistence**

Store permissions as canonical JSON in `policy_snapshot_json`, set `status = 'authorized'`, `connection_mode = 'relay'`, and leave lifecycle timestamps null. On load, validate IDs/timestamps and require the permissions JSON to be an array of strings.

- [ ] **Step 4: Implement audit persistence**

Persist `event_id`, sequence, organization/user/device/session fields, event type, canonical event JSON, previous hash, and hash. Order reads by sequence. Deserialize full `ChainedAuditEvent` values and reject inconsistent duplicated columns as corrupt rows.

- [ ] **Step 5: Run storage tests and clippy for nexusd**

Run: `cargo test -p nexusd --test storage_sqlite`

Expected: PASS.

Run: `cargo clippy -p nexusd --all-targets -- -D warnings`

Expected: PASS.

- [ ] **Step 6: Commit Task 3**

```bash
git add apps/nexusd/src/storage apps/nexusd/tests/storage_sqlite.rs
git commit -m "feat(control-plane): persist sessions and audit events"
```

### Task 4: Integrate Durable State with Existing HTTP Flows

**Files:**
- Modify: `apps/nexusd/src/state.rs`
- Modify: `apps/nexusd/src/routes.rs`
- Modify: `apps/nexusd/src/main.rs`
- Modify: `apps/nexusd/src/lib.rs`
- Modify: `apps/nexusd/tests/control_plane_api_e2e.rs`

**Interfaces:**
- Consumes: all `SqliteStorage` repository methods from Tasks 1–3.
- Produces: `AppState::new(signing_key, control_plane_id, storage)`.
- Produces: async `store_enrollment_token`, `enroll_device`, `get_device`, `list_devices`, `record_audit`, and `persist_authorized_session` state methods.

- [ ] **Step 1: Convert the API E2E fixture to durable state and add restart assertion**

Build `SqliteStorage` from a temporary file, pass it into `AppState::new`, and await token storage. After enrollment and authorization, connect a second storage instance to the same URL and assert the host device, authorized session, and two audit events exist.

- [ ] **Step 2: Run the E2E test and verify failure**

Run: `cargo test -p nexusd --test control_plane_api_e2e`

Expected: compilation fails because `AppState` and its operations are still synchronous/in-memory.

- [ ] **Step 3: Replace in-memory registries in AppState**

Remove `devices` and `enrollment_tokens`. Add `storage: SqliteStorage`. Preserve injected `AuditSink`, policy engine, signing key, relay ID, and audit chain. Make `record_audit` append, durably insert, then call the injected sink, returning `Result<(), StateError>`.

Do not hold the synchronous audit-chain mutex across either `.await`; clone the appended event before database and sink calls.

- [ ] **Step 4: Convert routes to async persistence and fail-closed audit behavior**

Await token storage/consumption, device lookup/list, session insertion, and audit writes. Build `AuthorizedSessionRecord` from the same granted permissions used in `SessionCapability`. Map expected enrollment failures to existing 403 responses; log `StateError::Storage` and return a stable `500 {"error":"control-plane storage failure"}`.

- [ ] **Step 5: Initialize SQLite before serving**

In `main`, read `NEXUS_DATABASE_URL` with default `sqlite://nexus.db?mode=rwc`, create `DatabaseConfig::sqlite`, connect/migrate, and pass storage into `AppState`. Keep bind address behavior unchanged.

- [ ] **Step 6: Run nexusd and workspace tests**

Run: `cargo test -p nexusd`

Expected: PASS.

Run: `cargo test --workspace`

Expected: PASS.

- [ ] **Step 7: Commit Task 4**

```bash
git add apps/nexusd/src apps/nexusd/tests/control_plane_api_e2e.rs
git commit -m "feat(control-plane): use durable SQLite state"
```

### Task 5: Documentation and Full Verification

**Files:**
- Modify: `docs/IMPLEMENTATION_STATUS.md`
- Modify: `README.md`
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: verified behavior from Tasks 1–4.
- Produces: accurate repository/phase status for future contributors.

- [ ] **Step 1: Update implementation status**

Mark `migrations/` as in progress with the foundation migration. Extend the nexusd row with SQLite-backed enrollment/device/session/audit persistence. Do not mark Phase 1 done.

- [ ] **Step 2: Remove stale pre-Phase-0 wording**

Update README and CLAUDE to say the OS-independent Phase 0 foundation is complete and Phase 1 platform/persistence work is underway. Change the ADR count from 24 to 25 wherever stale.

- [ ] **Step 3: Run documentation consistency and formatting checks**

Run: `rg -n "Pre-Phase-0|pre-Phase-0|24/24|all 24" README.md CLAUDE.md docs/IMPLEMENTATION_STATUS.md`

Expected: no stale status/count matches.

Run: `cargo fmt --all -- --check`

Expected: PASS.

- [ ] **Step 4: Run complete verification**

Run: `cargo test --workspace`

Expected: all unit, integration, E2E, and doc tests PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS with no warnings.

Run: `git diff --check`

Expected: no output.

- [ ] **Step 5: Commit documentation**

```bash
git add docs/IMPLEMENTATION_STATUS.md README.md CLAUDE.md
git commit -m "docs: mark SQLite persistence in progress"
```
