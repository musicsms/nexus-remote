# Audit Event & Tamper-Evident Logging System (`nexus-audit`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `nexus-audit` event model, initial event types, tamper-evident cryptographic hash chaining (Spec Section 35 `hash_n = HASH(event_n || hash_n-1)`), verification engine, and audit sink abstractions (`AuditSink`, `MemoryAuditSink`).

**Architecture:** 
- `event.rs`: `AuditEventType` (16 standard types per Spec Section 35), `AuditEvent` structure with canonical serialization, strongly typed IDs (`UserId`, `DeviceId`, `SessionId`, `TenantId`, `UnixTimestamp`), and structured metadata.
- `chain.rs`: `AuditChain` and `ChainedAuditEvent` maintaining a cryptographic hash chain (using BLAKE3 / SHA-256) with chain integrity verification (`verify_chain`).
- `sink.rs`: `AuditSink` trait for asynchronous audit recording, `MemoryAuditSink` for deterministic testing/querying, and multi-sink dispatchers.

**Tech Stack:** Rust, `nexus-common` (for `UserId`, `DeviceId`, `SessionId`, `TenantId`, `UnixTimestamp`), `blake3`, `serde`, `serde_json`, `thiserror`, `async-trait`.

**Spec:** `docs/Nexus Remote Desktop Platform - Spec.md` (Section 35 - Audit Model, Section 34 - Database Model).

## Global Constraints

- OS-independent by construction (pure Rust).
- Canonical serialization ensures deterministic hash computation across platforms.
- Tamper-evident hash chain detects any modification to event content, ordering, or sequence number.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` must pass.

---

### Task 1: Audit Event Types and Schema (`nexus-audit::event`)

**Files:**
- Create: `crates/nexus-audit/src/event.rs`
- Modify: `crates/nexus-audit/src/lib.rs`
- Modify: `crates/nexus-audit/Cargo.toml` (add `serde_json`)
- Test: `crates/nexus-audit/src/event.rs`

**Interfaces:**
- Produces: `AuditEventType`, `AuditEvent`, `EventParseError`.
- Event types: `UserLogin`, `UserMfa`, `DeviceEnroll`, `DeviceRevoke`, `SessionRequest`, `SessionAuthorize`, `SessionDeny`, `SessionStart`, `SessionDisconnect`, `SessionEnd`, `ClipboardRead`, `ClipboardWrite`, `FileUpload`, `FileDownload`, `AccessRequest`, `AccessApprove`, `PolicyUpdate`.
- Methods: `AuditEvent::new(...)`, `AuditEvent::canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error>`, `Display`, `FromStr`.

- [ ] **Step 1: Write failing unit tests for `AuditEventType` and `AuditEvent` serialization**
- [ ] **Step 2: Implement `AuditEventType` and `AuditEvent`**
- [ ] **Step 3: Run `cargo test -p nexus-audit` and verify PASS**
- [ ] **Step 4: Commit**

---

### Task 2: Tamper-Evident Cryptographic Hash Chain (`nexus-audit::chain`)

**Files:**
- Create: `crates/nexus-audit/src/chain.rs`
- Modify: `crates/nexus-audit/src/lib.rs`
- Modify: `crates/nexus-audit/Cargo.toml` (add `blake3`)
- Test: `crates/nexus-audit/src/chain.rs`

**Interfaces:**
- Produces: `ChainedAuditEvent`, `AuditChain`, `ChainVerificationError`.
- Methods: `AuditChain::new(initial_seed: Option<&str>) -> Self`, `AuditChain::append(&mut self, event: AuditEvent) -> Result<ChainedAuditEvent, ChainError>`, `verify_chain(events: &[ChainedAuditEvent], initial_seed: Option<&str>) -> Result<(), ChainVerificationError>`.

- [x] **Step 1: Write failing unit tests for hash chaining and tamper detection (modified content, reordered events, sequence gaps)**
- [x] **Step 2: Implement `AuditChain` hash generation and `verify_chain` validation**
- [x] **Step 3: Run `cargo test -p nexus-audit` and verify PASS**
- [x] **Step 4: Commit**

---

### Task 3: Audit Sink Abstractions (`nexus-audit::sink`)

**Files:**
- Create: `crates/nexus-audit/src/sink.rs`
- Modify: `crates/nexus-audit/src/lib.rs`
- Modify: `crates/nexus-audit/Cargo.toml` (add `async-trait`, `tokio` for mutex)
- Test: `crates/nexus-audit/src/sink.rs`

**Interfaces:**
- Produces: `AuditSink` trait, `MemoryAuditSink`, `SinkError`.
- Methods: `AuditSink::record(&self, event: &ChainedAuditEvent) -> Result<(), SinkError>`, `MemoryAuditSink::events(&self) -> Vec<ChainedAuditEvent>`.

- [ ] **Step 1: Write failing unit tests for async audit sinks and recording**
- [ ] **Step 2: Implement `AuditSink` trait, `MemoryAuditSink`, and composite/broadcasting sink**
- [ ] **Step 3: Run `cargo test -p nexus-audit` and verify PASS**
- [ ] **Step 4: Commit**

---

### Task 4: Status Documentation and Workspace Verification

**Files:**
- Modify: `docs/IMPLEMENTATION_STATUS.md`

- [ ] **Step 1: Run workspace verification (fmt, clippy, test --workspace)**
- [ ] **Step 2: Update `docs/IMPLEMENTATION_STATUS.md` marking `nexus-audit` as In progress with implemented features**
- [ ] **Step 3: Commit**
