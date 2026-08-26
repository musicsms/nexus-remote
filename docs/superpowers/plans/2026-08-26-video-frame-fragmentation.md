# Video Frame Fragmentation and Reassembly Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement packetization of large encoded/encrypted video frames into bounded Section 21 `VideoPacketHeader` datagram fragments (<= 1200 bytes) and robust, bounded drop-stale reassembly on the receiver.

**Architecture:** 
- `packetize_video_frame`: Slices frame payload into MTU-safe chunks, sets `FRAME_START`/`FRAME_END` flags and monotonic `packet_id`/`packet_count`.
- `VideoFrameReassembler`: Bounded in-flight frame assembler that reassembles fragments in arbitrary arrival order, enforces size/packet limits, drops stale/incomplete frames (ADR-022), and produces assembled frames for AEAD decryption and decoding.

**Spec:** `docs/Nexus Remote Desktop Platform - Spec.md` (Section 21), `docs/adr/ADR-022-capture-encode-backpressure-drop-stale.md`, `docs/adr/ADR-025-encoded-frame-aead-framing.md`.

## Constraints

- Datagram payload limit: <= 1200 bytes per fragment (`MAX_PAYLOAD_LEN`).
- Bounded memory: Limit in-flight reassembly buffers and maximum frame size.
- Drop-stale: Dropping older incomplete frames when newer frames arrive.
- Input validation: Reject malformed header flags, invalid `packet_id >= packet_count`, payload length mismatches, and duplicate invalid chunks.

### Task 1: Implement packetizer and reassembler with unit tests in `nexus-transport`

- [x] Add `packetize_video_frame` and `VideoFrameReassembler` in `crates/nexus-transport/src/video.rs`.
- [x] Add comprehensive tests for single-packet frames, multi-packet frames, out-of-order arrival, duplicate packets, stale frame pruning, and malformed fragment rejection.
- [x] Run formatting, workspace tests, and clippy.
- [x] Update `docs/IMPLEMENTATION_STATUS.md`.
