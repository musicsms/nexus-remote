# Phase 1 MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the Phase 1 Windows MVP: enroll a Windows host and client, control the host through a relay, stream H.264 video with authenticated input/cursor, reconnect safely, and expose measurable session quality.

**Architecture:** Keep the existing OS-independent crates and control-plane contracts stable. Add Windows implementations behind safe traits, connect them through the existing desktop-host/agent pipeline, build a native Windows client, then validate the complete client → nexusd → relay → agent → desktop-host path on real Windows machines.

**Tech Stack:** Rust 2021, Tokio, Quinn, Axum, SQLx/SQLite, prost, `windows` crate behind platform modules, Media Foundation/Windows Graphics Capture or DXGI, native Windows UI/rendering selected by a platform ADR, H.264 hardware encoder with software fallback, existing ChaCha20-Poly1305 frame AEAD.

**Spec:** `docs/Nexus Remote Desktop Platform - Spec.md` (Phase 1 sections and Sections 2, 5, 9, 21, 22, 33, 48, 56–57)

## Global Constraints

- Windows host and Windows client are the only Phase 1 platform target.
- Relay-only transport is mandatory for v0.1; P2P/NAT traversal is excluded.
- H.264 is mandatory; hardware encoding is preferred and software fallback is required for testability.
- Capture/encode queues are bounded and drop stale frames.
- All remote input and packet fields are hostile input and must be validated.
- No privileged operation occurs without an audit event.
- No blocking Windows or filesystem I/O runs on Tokio worker threads.
- OS/codec FFI is wrapped behind safe abstractions; `unsafe` is limited to documented narrow modules.
- Every network message has an explicit maximum size and timeout.
- Native client is not Electron/browser/mobile.
- Phase 1 exit condition is a real Windows host/client session controlled through a relay with first frame and input latency measurements recorded.

## Dependency Graph and Workstreams

```text
W1 Windows capture/codec/input ─┐
                                ├─> W2 desktop-host + IPC/service boundary
W3 Windows agent/service ───────┘             │
                                              ├─> W4 native Windows client
                                              └─> W5 full relay session/reconnect
                                                       │
                                                       └─> W6 observability/performance acceptance
```

### Task 1: Freeze Windows Platform Decisions and Scaffolding

**Files:**
- Create: `docs/adr/ADR-026-windows-capture-api-selection.md`
- Create: `docs/adr/ADR-027-windows-client-rendering-stack.md`
- Create: `platform/windows/Cargo.toml`
- Create: `platform/windows/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `docs/IMPLEMENTATION_STATUS.md`

**Interfaces:**
- Produces `platform_windows::CaptureBackend`, `EncoderBackend`, `InputBackend`, and `CursorBackend` feature-gated for Windows.
- Produces ADRs freezing API selection, thread/apartment requirements, fallback behavior, and rendering choice.

- [ ] Verify current Windows SDK/toolchain requirements against the existing Rust MSVC toolchain (real Windows toolchain/hardware validation remains unverified).
- [x] Compare Windows Graphics Capture vs DXGI duplication for session, pre-login, and multi-monitor constraints; record the selected API and fallback in ADR-026.
- [x] Compare DirectComposition/Win32 renderer options; record the selected client stack in ADR-027.
- [x] Scaffold platform crate with `cfg(windows)` implementations and non-Windows compile stubs that return `UnsupportedPlatform`.
- [ ] Add compile tests for Windows target and Linux workspace target (Windows-target verification remains unverified; the available GNU target currently fails the existing cfg-gated test build).
- [x] Update status to mark `platform/windows/` scaffolded and the ADRs done.
- [x] Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all -- --check`.
- [x] Commit: `feat(platform): scaffold Windows Phase 1 backends` (evidenced by commits `fde86ab`, `bcff411`, and `40f7946`; this fix round remains a documentation amendment).

### Task 2: Implement Real Windows Capture, H.264, Input, and Cursor Backends

**Files:**
- Create: `platform/windows/src/capture.rs`
- Create: `platform/windows/src/codec.rs`
- Create: `platform/windows/src/input.rs`
- Create: `platform/windows/src/cursor.rs`
- Modify: `crates/nexus-capture/src/lib.rs`
- Modify: `crates/nexus-codec/src/lib.rs`
- Modify: `crates/nexus-input/src/lib.rs`
- Modify: `proto/nexus.proto`
- Create: `platform/windows/tests/backend_smoke.rs`

**Interfaces:**
- Implements existing `CaptureSource`, `VideoEncoder`, and input dispatch traits without leaking Windows types.
- Produces bounded capture frames in the existing BGRA contract.
- Produces H.264 access units with keyframe/configuration metadata and reconfigure support.
- Produces validated semantic keyboard/mouse events and cursor shape/position messages.

- [ ] Write Windows-targeted tests for frame dimensions/stride, device-loss recovery, encoder configuration, keyframe requests, input bounds, and cursor payload limits.
- [ ] Implement capture on a dedicated OS thread/apartment; copy only validated frame data into the bounded latest-frame queue.
- [ ] Implement Media Foundation hardware H.264 with explicit profile/level, bitrate, frame-rate, keyframe, and device-loss errors; retain `SoftwareFallbackEncoder` for unsupported hardware.
- [ ] Implement `SendInput`-based keyboard/mouse injection with allow-listed virtual keys, bounded text, and coordinate clamping.
- [ ] Implement cursor capture/serialization with bounded dimensions and shape payloads.
- [ ] Add device-loss and fallback telemetry counters.
- [ ] Run tests on Windows CI/local hardware and Linux cross-compile tests; record measured encoder/capture timings.
- [ ] Commit: `feat(windows): implement capture codec input and cursor backends`.

### Task 3: Complete Desktop-Host Runtime, Agent IPC, Service Runner, and Respawn

**Files:**
- Modify: `apps/nexus-desktop-host/src/worker.rs`
- Modify: `apps/nexus-desktop-host/src/streamer.rs`
- Modify: `apps/nexus-desktop-host/src/input_handler.rs`
- Create: `apps/nexus-agent/src/service.rs`
- Create: `apps/nexus-agent/src/ipc.rs`
- Modify: `apps/nexus-agent/src/session_manager.rs`
- Modify: `apps/nexus-agent/src/main.rs`
- Create: `platform/windows/src/service.rs`
- Create: `apps/nexus-desktop-host/tests/windows_pipeline_e2e.rs`

**Interfaces:**
- Produces authenticated agent↔desktop-host IPC with process identity/signature verification per ADR-020.
- Produces service lifecycle APIs for start/stop, reconnect, and crash respawn per ADR-024.
- Uses `SYSTEM` only for pre-login operations and as-user context for in-session capture per ADR-021.

- [ ] Write tests for IPC handshake rejection, wrong process identity, capability/session mismatch, service restart, crash respawn, and reconnect-window expiry.
- [ ] Implement Windows named-pipe IPC with bounded framed messages, peer identity verification, and explicit shutdown timeouts.
- [ ] Implement service runner and per-user desktop-host launch/monitoring without blocking Tokio threads.
- [ ] Connect real capture/encoder/input/cursor backends to `HostVideoStreamer` and `HostInputHandler`.
- [ ] Ensure host worker drops stale frames, emits audit events for privileged input, and stops on capability expiry.
- [ ] Add Windows integration test that captures a real or deterministic desktop source, injects input, and observes encrypted datagrams.
- [ ] Commit: `feat(windows): complete agent desktop-host runtime boundary`.

### Task 4: Build Native Windows Client

**Files:**
- Create: `apps/nexus-client/src/lib.rs`
- Create: `apps/nexus-client/src/main.rs`
- Create: `apps/nexus-client/src/session.rs`
- Create: `apps/nexus-client/src/renderer.rs`
- Create: `apps/nexus-client/src/input.rs`
- Create: `apps/nexus-client/src/window.rs`
- Modify: `apps/nexus-client/Cargo.toml`
- Create: `apps/nexus-client/tests/client_protocol.rs`

**Interfaces:**
- Produces native Windows viewer/controller binary with session connect/disconnect state machine.
- Consumes `SessionCapability`, relay tokens, encoded-frame AEAD contract, `VideoPacketHeader`, cursor messages, and input protobufs.
- Produces decoded frame surfaces, bounded render queue, semantic input events, and explicit user-visible connection errors.

- [ ] Write protocol tests for capability verification, relay-token verification, malformed frame rejection, nonce replay, cursor bounds, and input rate/size limits.
- [ ] Implement enrollment/credential loading without exposing private keys to the control plane.
- [ ] Implement relay-only Quinn connection, control stream, datagram reassembly, frame AEAD open, and keyframe request handling.
- [ ] Implement H.264 decoder and renderer using the ADR-027 stack; keep decode/render work off Tokio runtime threads.
- [ ] Implement keyboard, mouse, wheel, text, and cursor interaction with focus/coordinate mapping.
- [ ] Add reconnect UI/state transitions and session expiry handling.
- [ ] Run the client against synthetic loopback and a real Windows host; record first-frame and input timings.
- [ ] Commit: `feat(client): add native Windows viewer controller`.

### Task 5: Full Relay Session, Enrollment, Reconnect, and Authorization Flow

**Files:**
- Modify: `apps/nexusd/src/routes.rs`
- Modify: `apps/nexusd/src/state.rs`
- Modify: `apps/nexus-agent/src/session_manager.rs`
- Modify: `apps/nexus-relay/src/server.rs`
- Create: `test/integration/phase1_relay_session.rs`
- Create: `test/network-sim/relay_profiles.md`

**Interfaces:**
- Produces a complete client→nexusd→relay→agent session lifecycle with signed capability and directional frame AEAD.
- Enforces TTL, max duration, protocol range, replay defense, concurrent-control exclusivity, and reconnect-window rules.

- [ ] Write integration tests covering host/client enrollment, session request, relay token pairing, bidirectional control/media, capability expiry, duplicate nonce, and unauthorized input.
- [ ] Add relay session health/timeouts and explicit endpoint disconnect cleanup.
- [ ] Implement client and agent reconnect race with relay-only fallback, preserving session ID and established-duration policy.
- [ ] Ensure every authorization, denial, connect, disconnect, expiry, and privileged input event reaches durable audit storage.
- [ ] Add relay network profiles for latency, loss, reordering, and disconnect/reconnect; verify stale-frame policy under each profile.
- [ ] Run the complete flow on two Windows processes and a relay process; capture logs and packet/frame counters.
- [ ] Commit: `feat(phase1): complete relay session lifecycle and reconnect`.

### Task 6: Observability, Performance, Security Review, and Phase Exit

**Files:**
- Modify: `crates/nexus-observability/src/lib.rs`
- Modify: `apps/nexusd/src/routes.rs`
- Modify: `apps/nexus-relay/src/server.rs`
- Modify: `apps/nexus-agent/src/session_manager.rs`
- Modify: `apps/nexus-desktop-host/src/worker.rs`
- Create: `test/performance/phase1_benchmarks.md`
- Create: `docs/security/phase1-threat-model.md`
- Modify: `docs/IMPLEMENTATION_STATUS.md`
- Modify: `README.md`

**Interfaces:**
- Produces metrics for first-frame time, capture/encode/decode latency, input-to-host latency, frame drops, bitrate, reconnect count, and session duration.
- Produces a telemetry overlay and structured logs with session/device correlation IDs.

- [ ] Write metric assertions and a benchmark harness that records p50/p95 first-frame, input latency, frame age, and drop rate.
- [ ] Implement bounded metric collection and telemetry overlay; never log frame plaintext, keys, or sensitive input text.
- [ ] Run LAN and relay tests at 1080p60 where hardware permits; document actual measurements and fallback behavior.
- [ ] Write the Phase 1 threat-model review covering Windows capture, IPC spoofing, input injection, capability theft, relay blindness, and crash recovery.
- [ ] Run `cargo fmt --all -- --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, Windows integration tests, and security checks.
- [ ] Confirm Phase 1 exit condition on real Windows host/client/relay and attach results to the status document.
- [ ] Mark Phase 1 `Done` only after the measured exit condition passes; otherwise record the exact unmet condition.
- [ ] Commit: `docs: record Phase 1 MVP acceptance results`.

## Final Verification Checklist

- [ ] Fresh Windows host enrollment persists and produces a valid credential.
- [ ] Fresh Windows client enrollment persists and can request a session.
- [ ] Relay verifies both endpoint tokens without decrypting application payloads.
- [ ] Host captures and encodes H.264; client decodes and renders the first frame.
- [ ] Client keyboard/mouse input reaches the host and is audited.
- [ ] Cursor updates render correctly and malformed cursor/frame packets are rejected.
- [ ] Capability expiry, reconnect window, max duration, and duplicate nonce behavior are verified.
- [ ] Host desktop-host crash respawns without incorrectly terminating the authorized session.
- [ ] Metrics and performance results are stored in `test/performance/phase1_benchmarks.md`.
- [ ] `docs/IMPLEMENTATION_STATUS.md` and README match the actual result.
- [ ] All required Rust and Windows verification commands pass.
