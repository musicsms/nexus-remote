# Encoded-Frame Crypto Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement ADR-025's canonical encoded-frame AAD and nonce-consuming seal/open contract.

**Architecture:** `nexus-crypto` serializes stable frame metadata into a fixed big-endian AAD and combines it with the caller-owned `NonceSequence` and existing AEAD primitive. Packetizer fields and transport reassembly remain outside the crypto layer.

**Spec:** `docs/adr/ADR-025-encoded-frame-aead-framing.md`

### Task 1: Add and verify encoded-frame contract

- [ ] Add canonical metadata, encrypted-frame result, and typed seal/open helpers.
- [ ] Test successful round-trip and metadata authentication failure.
- [ ] Run fmt/test/clippy, create/merge PR, and run independent `agy` review.
