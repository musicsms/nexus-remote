# Common Core Primitives Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `nexus-common` foundational primitives: strongly typed entity IDs (`DeviceId`, `UserId`, `NodeId`, `TenantId`, `SessionId`), time/clock abstractions for deterministic testing, common error models, and shared configuration types.

**Architecture:** `nexus-common` provides shared, OS-independent core types with zero heavy dependencies (only `thiserror` and `serde`). Higher-level crates (`nexus-protocol`, `nexus-session`, `nexus-policy`, `nexus-auth`, `nexusd`) build on these common types without duplicating ID validation or time logic.

**Tech Stack:** Rust stable, `thiserror`, `serde`, `std::time`.

**Spec:** `docs/Nexus Remote Desktop Platform - Spec.md` (Section 5, Appendix A), `docs/adr/ADR-010-protocol-core-os-independent.md`.

## Global Constraints

- No OS-specific dependencies (`nexus-common` must remain strictly OS-independent).
- ID validation bounds: length between 1 and 128 bytes, printable ASCII / UTF-8 safe.
- Time abstraction must support deterministic mock clocks for reliable unit/integration testing without sleeping.
- All public types implement `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Serialize`, `Deserialize`, and `Display` where appropriate.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` must pass.

---

### Task 1: Strongly typed entity IDs (`nexus-common::id`)

**Files:**
- Create: `crates/nexus-common/src/id.rs`
- Modify: `crates/nexus-common/src/lib.rs`
- Test: `crates/nexus-common/src/id.rs` (unit tests)

**Interfaces:**
- Produces: `DeviceId`, `UserId`, `NodeId`, `TenantId`, `SessionId`, `ClientId`, `IdError`.
- Methods: `new(impl Into<String>) -> Result<Self, IdError>`, `as_str(&self) -> &str`, `Display`, `FromStr`, `Serialize`, `Deserialize`.

- [ ] **Step 1: Write failing unit tests for typed IDs**
- [ ] **Step 2: Implement `IdError` and generic validated string ID macro/structs**
- [ ] **Step 3: Run `cargo test -p nexus-common` to verify PASS**
- [ ] **Step 4: Commit**

---

### Task 2: Time and Clock Primitives (`nexus-common::time`)

**Files:**
- Create: `crates/nexus-common/src/time.rs`
- Modify: `crates/nexus-common/src/lib.rs`
- Test: `crates/nexus-common/src/time.rs` (unit tests)

**Interfaces:**
- Produces: `UnixTimestamp` (seconds/millis since epoch), `Clock` trait, `SystemClock`, `MockClock`.
- Methods: `Clock::now(&self) -> UnixTimestamp`, `MockClock::advance(&self, duration: Duration)`.

- [ ] **Step 1: Write failing unit tests for time and mock clock**
- [ ] **Step 2: Implement `UnixTimestamp`, `Clock` trait, `SystemClock`, and `MockClock`**
- [ ] **Step 3: Run `cargo test -p nexus-common` to verify PASS**
- [ ] **Step 4: Commit**

---

### Task 3: Shared Error Hierarchy (`nexus-common::error`)

**Files:**
- Create: `crates/nexus-common/src/error.rs`
- Modify: `crates/nexus-common/src/lib.rs`
- Test: `crates/nexus-common/src/error.rs` (unit tests)

**Interfaces:**
- Produces: `CommonError`, `ErrorCode`.

- [ ] **Step 1: Write failing unit tests for error codes and displays**
- [ ] **Step 2: Implement `CommonError` and `ErrorCode`**
- [ ] **Step 3: Run `cargo test -p nexus-common` to verify PASS**
- [ ] **Step 4: Commit**

---

### Task 4: Workspace Integration & Status Verification

**Files:**
- Modify: `docs/IMPLEMENTATION_STATUS.md`

- [ ] **Step 1: Run full workspace test suite, clippy, and format check**
- [ ] **Step 2: Update `docs/IMPLEMENTATION_STATUS.md` marking `nexus-common` In progress with implemented primitives**
- [ ] **Step 3: Commit**
