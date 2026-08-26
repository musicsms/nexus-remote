# Session Key Derivation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the ADR-006 X25519 + HKDF-SHA256 session-key derivation primitive without deciding application framing yet.

**Architecture:** Both endpoints contribute X25519 static/ephemeral secret material and derive the same 32-byte symmetric root key. HKDF binds a domain-separation label and caller-provided transcript context; all-zero shared secrets are rejected. AEAD/channel key expansion remains a later layer.

**Spec:** `docs/adr/ADR-006-application-e2e-encryption-through-relay.md`

### Task 1: Add and verify derivation primitive

- [ ] Add dependencies and `derive_session_key` API with explicit errors.
- [ ] Add symmetry, context separation, and low-order input tests.
- [ ] Update status, run fmt/test/clippy.
- [ ] Create/merge PR and run independent `agy` review of code, algorithm, and architecture.
