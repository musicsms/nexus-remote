# AEAD Nonce Sequence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement ADR-025's fail-closed 96-bit nonce sequence allocator.

**Architecture:** A per-direction/channel `NonceSequence` owns a 32-bit domain and a 64-bit monotonic counter. It emits big-endian nonces and refuses to wrap, leaving session teardown/rekey policy to the caller.

**Spec:** `docs/adr/ADR-025-encoded-frame-aead-framing.md`

### Task 1: Add and verify nonce allocation

- [ ] Add typed allocator and exhaustion error.
- [ ] Test domain encoding, monotonicity, and overflow behavior.
- [ ] Run fmt/test/clippy, create/merge PR, and run independent `agy` review.
