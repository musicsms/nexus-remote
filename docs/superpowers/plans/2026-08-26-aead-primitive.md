# AEAD Primitive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add authenticated ChaCha20-Poly1305 seal/open helpers over the derived 32-byte session key.

**Architecture:** `nexus-crypto` owns only the cryptographic operation. Callers provide a unique 96-bit nonce and protocol-associated data; nonce allocation and packet framing remain outside this crate until the framing ADR is settled.

**Spec:** `docs/adr/ADR-006-application-end-to-end-encryption-through-relay.md`

### Task 1: Implement and verify AEAD

- [ ] Add ChaCha20-Poly1305 dependency and typed encryption error.
- [ ] Add seal/open helpers with AAD authentication and tamper/replay-independent tests.
- [ ] Update status, run fmt/test/clippy.
- [ ] Create/merge PR and run independent `agy` review of code, algorithm, and architecture.
