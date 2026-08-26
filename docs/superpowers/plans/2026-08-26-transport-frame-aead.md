# Transport Encoded-Frame AEAD Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Expose ADR-025 encoded-frame seal/open through `nexus-transport` without coupling crypto to packet fragmentation.

**Architecture:** Transport maps stable `VideoPacketHeader` metadata to the crypto contract, encrypts the complete encoded frame first, and leaves packetization/reassembly to the existing video layer. Packetizer-specific fields are not authenticated by this API.

**Spec:** `docs/adr/ADR-025-encoded-frame-aead-framing.md`

### Task 1: Add transport integration

- [ ] Add `seal_video_frame`/`open_video_frame` wrappers.
- [ ] Test round-trip and header metadata tampering.
- [ ] Run fmt/test/clippy, create/merge PR, and run independent `agy` review.
