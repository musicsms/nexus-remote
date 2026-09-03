# Native Windows Client — Phase 1 Milestone 1 Design

## Goal

Replace the `nexus-client` stub with a native Windows viewer/controller
boundary that can verify a session capability, connect through the relay,
receive authenticated video datagrams, render decoded frames, send validated
semantic input, and reconnect or expire deterministically.

This milestone does not complete the Phase 1 exit condition. The complete
agent service/IPC path, real Windows host/client-through-relay acceptance, and
performance measurements remain later work.

## Architecture

The client binary is split into a portable session/transport layer and a
Windows UI/media layer:

- `session.rs` owns the lifecycle
  `Disconnected → Connecting → Connected → Reconnecting → Expired`,
  capability verification, relay-token validation, deadlines, and reconnect
  policy. It never exposes private key bytes.
- `receiver.rs` owns the client side of the existing Quinn control stream and
  datagram path. It validates `VideoPacketHeader`, reassembles bounded frames,
  opens frame AEAD with the existing nonce/AAD contract, and emits only
  authenticated decoded-frame jobs. Malformed packets, replay, expiry, and
  protocol mismatch are terminal or recoverable errors according to the
  session state.
- `renderer.rs` defines a small renderer trait and a bounded latest-frame
  render queue. The Windows implementation owns D3D11 resources and decode
  surfaces; no GPU or window handle crosses into Tokio workers.
- `window.rs` owns the native Win32 message loop on a dedicated thread and
  forwards focus, resize, close, and pointer-coordinate mapping commands over
  bounded channels.
- `input.rs` translates Win32 keyboard/mouse/text/cursor interactions into the
  existing semantic protobuf messages. It applies the same size, key, wheel,
  and coordinate validation as the host backend before enqueueing control
  messages.

## Data Flow

```text
SessionCapability + RelayToken
        │ verify locally
        ▼
Quinn relay connection ── control stream / datagrams
        │ bounded parse + AEAD open + reassembly
        ▼
Decoded frame queue ──► D3D11 renderer / local cursor
        ▲
Win32 input ──► semantic protobuf ──► bounded control stream
```

The receive path drops stale frames at queue depth one. It never retries a
stale video packet or logs plaintext frame data, keys, or text input.

## Threading and Lifecycle

- Tokio handles network I/O and session timers only.
- A dedicated window thread owns Win32 message dispatch.
- A dedicated media thread owns Media Foundation decoder and D3D11 resources.
- All cross-thread channels are bounded; shutdown has explicit deadlines.
- Reconnect preserves the session ID and established-duration policy; it does
  not reset capability TTL or extend max duration.
- On capability expiry or unrecoverable authentication failure, the client
  closes transport, clears render/input queues, and transitions to `Expired`.

## Security and Validation

- Verify the signed `SessionCapability` locally before opening media/control
  channels, including protocol range, TTL, permissions, and replay nonce.
- Verify relay tokens and bind the expected relay/session/device identities.
- Open frame AEAD only with the header-derived canonical AAD and directional
  nonce domain; reject modified headers, duplicate nonces, oversized payloads,
  and incomplete reassembly.
- Validate all semantic input before serialization and rate-limit the bounded
  control queue. Local cursor rendering accepts only bounded, validated cursor
  shape/position messages.
- Renderer failures are explicit errors; they never weaken packet
  authentication or cause plaintext fallback.

## Windows Rendering and Decode

The native path uses a Win32 window and Direct3D 11 renderer per ADR-027.
Media Foundation H.264 decoding is isolated behind a private adapter and
selected only on Windows. Decoder initialization or device loss is surfaced as
a recoverable/terminal client error according to session state; no fake frame
is emitted. A deterministic non-Windows renderer/decoder adapter exists only
for protocol and lifecycle tests and fails closed for native operations.

## Testing Strategy

- Portable unit tests cover every session transition, timeout, reconnect-window
  boundary, capability/token rejection, malformed packet, AEAD failure, nonce
  replay, queue freshness, cursor bounds, input limits, and renderer error.
- A synthetic loopback integration test sends an encrypted encoded frame and a
  semantic input message through the existing transport helpers, verifies AAD
  and nonce handling, and observes one decoded-frame job plus one input event.
- `cfg(windows)` smoke tests compile and, on an interactive Windows machine,
  create the Win32 window, initialize D3D11/Media Foundation, render one valid
  frame, and send one controlled input event. They are ignored with an explicit
  hardware requirement when run elsewhere.
- Linux verification remains `cargo fmt`, workspace build/test/clippy, and
  protocol/loopback tests; it is not evidence of Windows GUI/GPU behavior.

## Acceptance Criteria

- `nexus-client` is no longer a stub and exposes the specified lifecycle and
  bounded receiver/renderer/input boundaries.
- Normal builds preserve OS independence in core crates; Windows API types
  remain private to `cfg(windows)` modules.
- The synthetic loopback test proves authenticated receive/render handoff and
  semantic input emission.
- Native Windows code compiles for the available Windows target and has an
  ignored interactive smoke test; real smoke results are recorded only when
  actually run on Windows.
- The Phase 1 status remains **In progress** until the full host/client/service/
  relay flow and measured exit condition pass.

## Out of Scope

- Agent Windows service/IPC and desktop-host privilege launching.
- P2P/NAT traversal, adaptive bitrate, clipboard, audio, file transfer,
  recording, or multi-monitor support.
- Browser/mobile clients or non-Windows native UI.
