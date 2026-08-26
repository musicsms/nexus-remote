# Crypto Key Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a small OS-independent Ed25519 identity/key lifecycle abstraction for capability signing.

**Architecture:** `nexus-crypto` owns the private signing material and derives a stable public identity. Callers can construct a keypair from securely provisioned 32-byte seed material, sign payloads, and verify using the exported public key; storage, rotation, and OS secret integration remain higher-layer concerns.

**Spec:** `docs/adr/ADR-005-signed-capability-authorization.md`

## Constraints

- Never serialize or expose private key bytes through the public API.
- Keep the existing `SignedPayload` and free functions backward compatible.
- Add deterministic tests using fixed seed material; do not introduce a weak test RNG.

### Task 1: Implement and verify keypair lifecycle

- [ ] Add `DeviceKeypair` construction, public key access, signing, and verification helpers.
- [ ] Add round-trip and wrong-key tests.
- [ ] Update implementation status and run fmt/test/clippy.
- [ ] Create and merge a PR, then run independent `agy` architecture/code review.
