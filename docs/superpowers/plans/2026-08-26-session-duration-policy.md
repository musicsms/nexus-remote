# Session Duration Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement ADR-014's separate max-duration policy for established sessions without changing the wire schema.

**Architecture:** Keep capability establishment timestamps and active-session duration policy in `nexus-session`. The state machine records the ESTABLISHED instant and exposes a deterministic expiry predicate; callers can transition to `Expired` or `Revoked` explicitly.

**Tech Stack:** Rust, `std::time::Instant`/`Duration`, existing `nexus-session` state machine and unit tests.

**Spec:** `docs/adr/ADR-014-session-capability-ttl-semantics.md`

## Global Constraints

- Do not add a wire field or reinterpret `SessionCapability.expires_at` after establishment.
- Keep time-dependent behavior deterministic by accepting an explicit `Instant` in testable APIs.
- Preserve existing reconnect semantics and invalid-transition protections.

### Task 1: Add deterministic established-session duration tracking

**Files:** Modify `crates/nexus-session/src/state.rs`; update `docs/IMPLEMENTATION_STATUS.md`.

- [ ] Add a non-zero `SessionDurationPolicy` and state-machine methods to record establishment and report expiry.
- [ ] Add tests for expiry boundary, no-policy behavior, and explicit `Expired`/`Revoked` transitions.
- [ ] Run formatting, workspace tests, and clippy.
- [ ] Commit, push, create a PR, and merge when CI is non-conflicting.
- [ ] Run independent `agy` review covering code quality, algorithm, and architecture.
