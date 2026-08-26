# Phase 0 Foundation Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the full Phase 0 Exit Condition (Spec Section 48) by implementing synthetic desktop capture, software-fallback H.264 video encoder, and an end-to-end integration test proving the entire pipeline (Capture -> Queue -> Encode -> AEAD Encrypt -> Fragment -> QUIC Datagrams -> Reassemble -> AEAD Decrypt) runs between local processes.

**Architecture:** 
- `nexus-capture::synthetic`: Provides `SyntheticCaptureSource` implementing `CaptureSource` to generate test desktop frames.
- `nexus-codec::software`: Provides `SoftwareFallbackEncoder` implementing `VideoEncoder` for software-based test encoding.
- `nexus-transport::tests/phase0_e2e_pipeline.rs`: Integrates the full capture, encode, encrypt, datagram transport, reassembly, and decryption pipeline across a live QUIC connection.
- `docs/IMPLEMENTATION_STATUS.md`: Updates Phase 0 status to **Done**.

**Tech Stack:** Rust, Tokio, Quinn (QUIC), ChaCha20-Poly1305, bytes, nexus-capture, nexus-codec, nexus-crypto, nexus-transport.

**Spec:** `docs/Nexus Remote Desktop Platform - Spec.md` (Section 20, 21, 48, 52), `docs/adr/ADR-004-h264-mandatory-mvp-codec.md`, `docs/adr/ADR-022-capture-encode-backpressure-drop-stale.md`, `docs/adr/ADR-025-encoded-frame-aead-framing.md`.

## Global Constraints

- Keep OS-independent test abstractions clean and free of platform-specific headers.
- Datagram payload sizes must respect MTU limits (<= 1200 bytes).
- Full end-to-end pipeline must run deterministically in CI and local machines.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` must pass.

---

### Task 1: Synthetic Desktop Capture Backend (`nexus-capture::synthetic`)

**Files:**
- Create: `crates/nexus-capture/src/synthetic.rs`
- Modify: `crates/nexus-capture/src/lib.rs`
- Test: `crates/nexus-capture/src/synthetic.rs`

**Interfaces:**
- Produces: `SyntheticCaptureSource { width: u32, height: u32, fps: u32 }` implementing `CaptureSource`.
- Methods: `new(width, height, fps) -> Self`, `next_frame(&mut self) -> Result<CapturedFrame, Infallible>`.

- [ ] **Step 1: Write failing unit test for `SyntheticCaptureSource`**
- [ ] **Step 2: Implement `SyntheticCaptureSource` and pattern generator**
- [ ] **Step 3: Run `cargo test -p nexus-capture` and verify PASS**
- [ ] **Step 4: Commit**

---

### Task 2: Software Fallback Video Encoder (`nexus-codec::software`)

**Files:**
- Create: `crates/nexus-codec/src/software.rs`
- Modify: `crates/nexus-codec/src/lib.rs`
- Test: `crates/nexus-codec/src/software.rs`

**Interfaces:**
- Produces: `SoftwareFallbackEncoder` implementing `VideoEncoder`.
- Methods: `new() -> Self`, `configure`, `encode`, `request_keyframe`, `reconfigure`.

- [ ] **Step 1: Write failing unit test for `SoftwareFallbackEncoder`**
- [ ] **Step 2: Implement `SoftwareFallbackEncoder` with keyframe interval handling and dimension checks**
- [ ] **Step 3: Run `cargo test -p nexus-codec` and verify PASS**
- [ ] **Step 4: Commit**

---

### Task 3: Phase 0 End-to-End Live Pipeline Test (`nexus-transport`)

**Files:**
- Create: `crates/nexus-transport/tests/phase0_e2e_pipeline.rs`
- Modify: `crates/nexus-transport/Cargo.toml` (dev-dependencies on `nexus-capture` and `nexus-codec`)

**Interfaces:**
- Tests live streaming between Host task and Client task over QUIC:
  `SyntheticCaptureSource` -> `LatestFrameQueue` -> `SoftwareFallbackEncoder` -> `seal_video_frame` -> `packetize_video_frame` -> QUIC Datagrams -> `VideoFrameReassembler` -> `open_video_frame` -> verified match.

- [x] **Step 1: Write `phase0_e2e_pipeline.rs` integration test**
- [x] **Step 2: Run `cargo test -p nexus-transport --test phase0_e2e_pipeline` and verify PASS**
- [x] **Step 3: Commit**

---

### Task 4: Mark Phase 0 Foundation as Done in Implementation Status

**Files:**
- Modify: `docs/IMPLEMENTATION_STATUS.md`

- [ ] **Step 1: Run full workspace test suite and clippy**
- [ ] **Step 2: Update `docs/IMPLEMENTATION_STATUS.md` marking Phase 0 as Done**
- [ ] **Step 3: Commit**
