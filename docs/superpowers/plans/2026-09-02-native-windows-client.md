# Native Windows Client Milestone Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `nexus-client` stub with a bounded, authenticated native Windows viewer/controller boundary and synthetic loopback test for Phase 1.

**Architecture:** Keep session lifecycle, packet validation, AEAD, reassembly, and semantic input OS-independent. Put Win32 windowing, D3D11 rendering, and Media Foundation decoding behind private `cfg(windows)` adapters on dedicated threads; Tokio owns only network I/O and timers.

**Tech Stack:** Rust 2021, Tokio, Quinn, existing `nexus-auth`, `nexus-crypto`, `nexus-protocol`, `nexus-transport`, `nexus-input`, Windows 0.58, Win32, Direct3D 11, Media Foundation.

**Spec:** `docs/superpowers/specs/2026-09-02-native-windows-client-design.md`

## Global Constraints

- Native Win32 + D3D11 rendering is mandatory for Windows per ADR-027.
- Core/session/transport code remains OS-independent; Windows types stay private to `cfg(windows)` modules.
- Tokio handles network I/O/timers only; window, decode, and GPU resources belong to dedicated native threads.
- All channels and queues are bounded; video queues use depth-one drop-stale semantics.
- Capability, relay token, packet header, AEAD nonce/AAD, input, cursor, and payload fields are validated before use.
- Reconnect preserves session ID and established-duration policy; expiry clears queues and closes transport.
- No plaintext frame, key, or text-input logging; no unauthenticated rendering or input fallback.
- Linux/GNU compilation is not evidence of live Windows GUI/GPU behavior.

---

### Task 1: Build the Portable Client Session State Machine

**Files:**
- Modify: `apps/nexus-client/Cargo.toml`
- Create: `apps/nexus-client/src/lib.rs`
- Create: `apps/nexus-client/src/session.rs`
- Create: `apps/nexus-client/tests/session_state.rs`

**Interfaces:**
- Produces `ClientState::{Disconnected, Connecting, Connected, Reconnecting, Expired}`.
- Produces `ClientSession::new(capability, relay_token, clock)`, `begin_connect(now)`, `connected(now)`, `transport_lost(now)`, `reconnect_deadline()`, `expire()`, and `state()`.
- Consumes `SessionCapability`, relay-token metadata, `Clock`, and `SessionAggregate` duration policy without exposing private keys.

- [ ] **Step 1: Add core dependencies**

Add workspace dependencies for `nexus-auth`, `nexus-crypto`, `nexus-session`, `nexus-transport`, and `nexus-input` in the client manifest. Keep Windows dependencies target-specific.

- [ ] **Step 2: Write failing lifecycle tests**

Add tests asserting:

```rust
let mut client = ClientSession::new(capability, relay_token, MockClock::new(100));
assert_eq!(client.state(), ClientState::Disconnected);
client.begin_connect(100).unwrap();
client.connected(101).unwrap();
client.transport_lost(102).unwrap();
assert_eq!(client.state(), ClientState::Reconnecting);
assert!(client.reconnect_deadline().is_some());
client.expire().unwrap();
assert_eq!(client.state(), ClientState::Expired);
```

Also test skipped transitions, capability expiry, max-duration expiry, and reconnect-window boundary.

- [ ] **Step 3: Run lifecycle tests and observe RED**

Run: `cargo test -p nexus-client --test session_state`

Expected: compile failure because client session types do not exist.

- [ ] **Step 4: Implement minimal state machine**

Validate capability and relay-token identity before `begin_connect`. Reuse existing session duration/reconnect policy; reject all transitions after `Expired` and never reset established start time on reconnect.

- [ ] **Step 5: Run lifecycle tests and observe GREEN**

Run: `cargo test -p nexus-client --test session_state`

Expected: all lifecycle, expiry, and invalid-transition tests pass.

- [ ] **Step 6: Commit**

```bash
git add apps/nexus-client
git commit -m "feat(client): add bounded session lifecycle"
```

### Task 2: Implement Authenticated Video/Input Receiver

**Files:**
- Create: `apps/nexus-client/src/receiver.rs`
- Modify: `apps/nexus-client/src/lib.rs`
- Create: `apps/nexus-client/tests/receiver_protocol.rs`

**Interfaces:**
- Produces `ClientReceiver::new(frame_key, nonce_domain)`, `accept_datagram(bytes)`, `accept_control(bytes)`, and `drain_latest_frame()`.
- Produces `DecodedFrameJob { frame_id, timestamp_us, keyframe, access_unit }` and `ClientInputSender::encode(event)`.
- Consumes `VideoPacketHeader`, `VideoFrameReassembler`, `open_encoded_frame`, `encode_framed_control`, and semantic `InputEvent`.

- [ ] **Step 1: Write failing hostile-packet tests**

Test truncated/oversized headers, malformed fragments, modified AAD/header, duplicate nonce, stale frame replacement, and invalid semantic input. Assert no job is emitted for each rejection.

- [ ] **Step 2: Run receiver tests and observe RED**

Run: `cargo test -p nexus-client --test receiver_protocol`

Expected: compile failure because receiver types do not exist.

- [ ] **Step 3: Implement bounded datagram receiver**

Decode and validate the header before AEAD open. Reassemble with existing bounded drop-stale logic, use header-derived canonical AAD and directional nonce, and emit only authenticated jobs. Preserve encoded frame metadata exactly.

- [ ] **Step 4: Implement bounded semantic input sender**

Call `InputEvent::validate()` before protobuf serialization, enforce the existing control payload maximum, and return a typed error rather than truncating or logging text.

- [ ] **Step 5: Run receiver tests and observe GREEN**

Run: `cargo test -p nexus-client --test receiver_protocol`

Expected: all malformed-input, AEAD, replay, freshness, and input tests pass.

- [ ] **Step 6: Commit**

```bash
git add apps/nexus-client/src
git commit -m "feat(client): add authenticated video and input receiver"
```

### Task 3: Add Bounded Renderer and Windows Decode Boundary

**Files:**
- Create: `apps/nexus-client/src/renderer.rs`
- Create: `apps/nexus-client/src/decoder.rs`
- Modify: `apps/nexus-client/src/lib.rs`
- Create: `apps/nexus-client/tests/render_queue.rs`
- Create: `apps/nexus-client/tests/windows_decoder_smoke.rs`

**Interfaces:**
- Produces `RenderQueue::new()`, `push_latest(DecodedFrameJob)`, `take_latest()`, and drop counters.
- Produces private `FrameDecoder` and `DecodedSurface` portable metadata contracts.
- Produces private `cfg(windows)` `MediaFoundationDecoder` and `D3D11Renderer` adapters, each owned by a named native thread.

- [ ] **Step 1: Write failing render-queue tests**

Assert depth-one replacement keeps only the newest frame, records dropped count, rejects an empty access unit, and refuses frames after shutdown.

- [ ] **Step 2: Run render tests and observe RED**

Run: `cargo test -p nexus-client --test render_queue`

Expected: compile failure because queue/decoder types do not exist.

- [ ] **Step 3: Implement portable queue and surface contracts**

Use a bounded mutex/channel or existing latest-frame pattern; never block a network receiver on rendering. Validate dimensions, keyframe metadata, and access-unit maximum before enqueueing.

- [ ] **Step 4: Run render tests and observe GREEN**

Run: `cargo test -p nexus-client --test render_queue`

Expected: queue and metadata tests pass.

- [ ] **Step 5: Add Windows Media Foundation decoder adapter**

On a dedicated `nexus-client-decoder` thread, initialize COM/MF, negotiate H.264 input and NV12 output, submit access units, drain zero-or-more output samples with bounded deadlines, and map device/MF loss to typed client errors. Drop/reconfigure releases MF resources on the owning thread. Do not emit a frame without a valid keyframe/sequence header.

- [ ] **Step 6: Add Windows D3D11 renderer adapter**

On a dedicated `nexus-client-renderer` thread, create the Win32-compatible D3D11 device/swap chain surface, copy/upload decoded NV12/RGBA data, present the newest frame, and report device loss. Keep all HWND/D3D interfaces private to the Windows module.

- [ ] **Step 7: Add ignored Windows smoke test**

`windows_decoder_smoke.rs` is `#![cfg(windows)]`, ignored with `requires an interactive Windows D3D11 and Media Foundation environment`, and verifies one authenticated decoded surface can be presented. It must not claim success on Linux/GNU compilation.

- [ ] **Step 8: Commit**

```bash
git add apps/nexus-client
git commit -m "feat(client): add bounded decode and D3D11 render boundary"
```

### Task 4: Add Win32 Window Thread and Input/Cursor Controller

**Files:**
- Create: `apps/nexus-client/src/window.rs`
- Create: `apps/nexus-client/src/input.rs`
- Modify: `apps/nexus-client/src/lib.rs`
- Modify: `apps/nexus-client/Cargo.toml`
- Create: `apps/nexus-client/tests/input_controller.rs`
- Create: `apps/nexus-client/tests/windows_client_smoke.rs`

**Interfaces:**
- Produces `WindowController::start(config)`, bounded `WindowCommand`/`WindowEvent`, and `shutdown(deadline)`.
- Produces `InputController::new(monitor)`, `handle_window_event`, and bounded encoded control messages.
- Consumes `nexus-input` semantic events and `CursorSnapshot` validation from `platform-windows` without importing Windows types into core code.

- [ ] **Step 1: Write failing input/controller tests**

Assert keyboard/mouse/text mapping, coordinate clamping, cursor bounds, rate/payload limits, focus gating, and shutdown after window close.

- [ ] **Step 2: Run controller tests and observe RED**

Run: `cargo test -p nexus-client --test input_controller`

Expected: compile failure because controllers do not exist.

- [ ] **Step 3: Implement portable controller translation**

Use bounded queues and existing semantic validation. Reject input while unfocused or expired; do not inject local raw Windows events directly from the network thread.

- [ ] **Step 4: Run controller tests and observe GREEN**

Run: `cargo test -p nexus-client --test input_controller`

Expected: all controller tests pass.

- [ ] **Step 5: Add private Windows Win32 message loop**

Create/register a window class on `nexus-client-window`, translate focus/resize/close/mouse/keyboard messages into portable events, and forward render commands over bounded channels. The message loop owns HWND and never runs on Tokio workers.

- [ ] **Step 6: Add ignored Windows client smoke test**

Compile/run-gate a Windows-only ignored test that creates the window, exercises focus and close, and confirms one controlled input event reaches the portable sender.

- [ ] **Step 7: Commit**

```bash
git add apps/nexus-client
git commit -m "feat(client): add Win32 window and input controller"
```

### Task 5: Wire Client Binary and Synthetic Loopback Integration

**Files:**
- Modify: `apps/nexus-client/src/main.rs`
- Modify: `apps/nexus-client/src/lib.rs`
- Create: `apps/nexus-client/tests/client_loopback_e2e.rs`
- Modify: `docs/IMPLEMENTATION_STATUS.md`
- Modify: `README.md`

**Interfaces:**
- Produces `ClientRuntime::connect`, `run`, and bounded `shutdown` orchestration.
- Consumes session, receiver, renderer, window, and input modules without exposing private Windows handles.

- [ ] **Step 1: Write failing loopback test**

Use existing QUIC loopback helpers and a deterministic key to send one sealed `VideoPacketHeader`/payload and one semantic input control message. Assert the client verifies/open/reassembles exactly one frame job and emits exactly one validated input message.

- [ ] **Step 2: Run loopback test and observe RED**

Run: `cargo test -p nexus-client --test client_loopback_e2e`

Expected: compile failure because `ClientRuntime` and integration wiring do not exist.

- [ ] **Step 3: Implement runtime orchestration**

Wire bounded Tokio network tasks to the session and receiver; hand jobs to the depth-one renderer queue and window thread. Treat `OutputPending`/frame-unavailable as non-fatal, propagate authentication/expiry/device errors, and preserve session ID on reconnect.

- [ ] **Step 4: Run loopback test and observe GREEN**

Run: `cargo test -p nexus-client --test client_loopback_e2e`

Expected: one authenticated frame job and one semantic input message with no plaintext logging.

- [ ] **Step 5: Replace the stub main**

Initialize tracing, load validated client configuration, call `ClientRuntime`, and return explicit user-visible errors. Do not add unattended private-key handling or browser dependencies.

- [ ] **Step 6: Synchronize status docs**

Mark `nexus-client` **In progress** with the exact implemented modules and test evidence. Keep Phase 1 **In progress** and record absent MSVC/live-Windows smoke/full host-service-relay acceptance.

- [ ] **Step 7: Run complete verification**

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p nexus-client --tests --target x86_64-pc-windows-gnu
cargo clippy -p nexus-client --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

Expected: all available checks exit zero; Windows-only smoke tests remain explicitly ignored on Linux.

- [ ] **Step 8: Commit**

```bash
git add apps/nexus-client docs/IMPLEMENTATION_STATUS.md README.md
git commit -m "feat(client): wire native viewer loopback runtime"
```

## Final Verification Checklist

- [ ] `nexus-client` is no longer a stub.
- [ ] Capability and relay token are verified before transport.
- [ ] Malformed/oversized/replayed/tampered video packets are rejected.
- [ ] Frame AEAD AAD/nonce matches encoded frame metadata.
- [ ] Depth-one render queue drops stale frames.
- [ ] Win32 window, D3D11 renderer, and Media Foundation decoder are isolated on native threads.
- [ ] Semantic input/cursor validation and focus gating are covered.
- [ ] Synthetic QUIC loopback proves authenticated receive/render handoff and input emission.
- [ ] Linux workspace checks and GNU Windows target checks pass.
- [ ] Windows interactive smoke results are not claimed without actually running them.
- [ ] Phase 1 remains In progress until full host/client/service/relay acceptance and measurements pass.
