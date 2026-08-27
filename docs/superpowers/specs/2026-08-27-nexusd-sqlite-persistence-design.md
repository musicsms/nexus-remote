# nexusd SQLite Persistence Design

## Objective

Add durable SQLite persistence to the existing Phase 1 control-plane flows
without prematurely implementing the entire Section 34 domain model. Device
enrollment, device lookup/listing, session authorization, and audit records
must survive process restarts. Enrollment-token consumption must remain
one-shot under concurrent requests.

This is the first Phase 1 persistence slice. PostgreSQL remains the documented
upgrade path, but its runtime implementation is outside this change.

## Scope

The first migration creates the tables exercised by current control-plane
behavior:

- `organizations`
- `enrollment_tokens`
- `devices`
- `device_credentials`
- `sessions`
- `audit_events`

Users, user identities, device labels, roles, role bindings, session events,
access requests, and relay nodes remain out of scope until their domain flows
exist. The schema uses portable scalar types and avoids SQLite-only schema
features so later PostgreSQL support does not require a data-model rewrite.

## Architecture

`nexusd` gains a `storage` module backed by a small SQLx `SqlitePool`. The
module owns migration execution, persistence models, row conversion, and
transactions. HTTP routes continue to depend on `AppState`, while `AppState`
exposes async domain-oriented operations rather than SQL details.

The dependency flow is:

```text
Axum routes -> AppState domain operations -> SQLite storage -> SQLx
```

`AppState` retains the signing key, policy engine, and in-memory audit hash
chain. Its in-memory device and enrollment-token maps are removed. Tests may
still inject an in-memory SQLite database, which provides the same transaction
behavior as file-backed deployment.

## Configuration and Startup

Add a database configuration containing:

- `driver`: `sqlite` or `postgres`
- `url`: the SQLx connection URL
- `max_connections`: a small positive pool limit

The default executable configuration uses SQLite. This slice accepts the
`postgres` enum value so the public configuration is consistent with ADR-013,
but startup returns an explicit unsupported-driver error until the PostgreSQL
implementation is added. Migrations run before the HTTP listener starts; a
migration failure prevents startup.

SQLite uses a conservative pool size suitable for its single-writer model.
Tests use temporary file databases where restart behavior matters. A shared
in-memory database may be used only when its connection lifetime and pooling
semantics are controlled by the test.

## Schema

Identifiers and timestamps are stored as text and integer scalar values that
map consistently to SQLite and PostgreSQL. Structured capability, credential,
token, policy-snapshot, and audit metadata values are stored as canonical JSON
text. Binary public keys and signatures are stored as binary columns.

Foreign keys express organization, device, and session ownership. Enrollment
tokens store their signed payload plus `uses_count`, expiry, and maximum uses.
Devices and credentials are separate tables to preserve the Section 34 model;
the current credential is joined when loading `RegisteredDevice`.

Sessions record authorization-time state, subject, endpoints, relay, policy
snapshot, creation time, and optional lifecycle timestamps/reason. This slice
inserts the initial authorized session record but does not add session lifecycle
APIs.

Audit rows store the chained audit representation needed to verify event order
and integrity after loading. The existing `AuditSink` still receives chained
events, so callers that inject a memory or broadcast sink retain current
behavior.

## Transactions and Concurrency

Enrollment token consumption uses a database transaction and a conditional
update that increments `uses_count` only when the token is unexpired and has
remaining uses. Failure is classified by reading the token within the same
transaction, preserving the current not-found, expired, and exhausted domain
errors. Device and credential insertion occurs in the same transaction as the
successful final enrollment write so no partial device is visible.

Session authorization persists the session before returning signed capability
and relay tokens. If persistence fails, the endpoint returns an internal error
and does not claim authorization succeeded.

Audit persistence errors are not silently discarded. `record_audit` returns a
result, and privileged operations fail closed if their required audit record
cannot be stored. The injected secondary `AuditSink` is called after durable
storage; sink failure is surfaced as an internal error as well.

## Error Handling

Storage errors are wrapped in a `StorageError` that preserves migration,
database, serialization, and corrupt-row context without exposing SQL details
through HTTP responses. `StateError` retains domain failures such as device not
found and token exhaustion and gains a storage variant.

Routes map expected domain failures to their existing HTTP statuses. Unexpected
storage failures return HTTP 500 with a stable public message and are logged
with internal context. Hostile or corrupt serialized rows fail conversion
rather than being accepted with defaults.

## Testing

Tests are added in layers:

1. Migration smoke test verifies a fresh database reaches the current schema.
2. Storage tests cover token consumption, concurrent final-use attempts,
   atomic device/credential persistence, tenant-filtered device listing,
   session insertion, audit insertion, and malformed-row rejection where
   practical.
3. A file-backed restart test drops the first pool, creates a new pool for the
   same database, and verifies devices, token usage, sessions, and audits remain.
4. The existing control-plane API E2E test is converted to SQLite-backed state
   and must retain its enrollment, duplicate-token rejection, authorization,
   relay-token, capability, and audit assertions.
5. Workspace formatting, tests, and clippy must pass.

## Documentation Impact

`docs/IMPLEMENTATION_STATUS.md` will record migrations and SQLite-backed
control-plane persistence as in progress. The stale pre-Phase-0 wording in
`README.md` and `CLAUDE.md` will be aligned with the completed OS-independent
Phase 0 foundation. No target-architecture statements in the main spec change.

## Deferred Work

- PostgreSQL pool and migration verification
- User/password/TOTP persistence
- Database-backed roles and bindings
- Device labels and revocation APIs
- Session lifecycle and session-event persistence
- Access requests and relay-node registry
- Rebuilding/verifying the audit chain from stored rows during startup
- Production secret management for the control-plane signing key
