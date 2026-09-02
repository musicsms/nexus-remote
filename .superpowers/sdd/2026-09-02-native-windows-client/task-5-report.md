# Task 5 implementation report

## Delivered

- Added a synthetic QUIC loopback integration test that sends one sealed v2
  video datagram and receives one validated semantic mouse control message.
  The test asserts authenticated frame metadata, payload, and exactly one
  outbound input message.
- Added `ClientRuntime`, which owns an established QUIC connection, the
  authenticated receiver, bounded Win32/window boundary, depth-one render
  handoff, and bounded semantic input queue. QUIC datagrams and a short
  Tokio polling interval are the only async orchestration; native handles stay
  private to the window/renderer/decoder modules.
- Added bounded runtime shutdown with transport close, input expiry, render
  queue clearing, and a caller-provided native-worker deadline.
- Replaced the binary no-op with tracing plus validated, non-secret endpoint
  configuration. The entrypoint returns an explicit error until the
  authenticated control-plane bootstrap supplies capability, relay token,
  certificate, and frame key; no unattended private-key or browser handling
  was added.
- Updated implementation status and README while keeping Phase 1 **In
  progress** and documenting the missing MSVC/live-Windows/full acceptance
  evidence.

## Verification

- `cargo fmt --all -- --check` — passed.
- `cargo test -p nexus-client --all-targets` — passed (46 tests including the
  loopback test; Windows-only smoke files compile to zero Linux tests).
- `cargo clippy -p nexus-client --all-targets -- -D warnings` — passed.
- `cargo check -p nexus-client --tests --target x86_64-pc-windows-gnu` — not
  completed: this environment lacks `x86_64-w64-mingw32-gcc`; `ring` and
  `aws-lc-sys` fail before the target client can compile.

## Notes

The loopback test uses the existing self-signed QUIC endpoint helper and is
not evidence of relay, MSVC, interactive Win32, D3D11, or Media Foundation
behavior. Full host/client/service/relay acceptance remains a Phase 1 exit
condition.

## Review fix round 1

- Runtime connection now validates `ClientSession` claims before opening the
  endpoint, retains the stable session ID/state, rechecks active claims and
  duration during pumping, and fails closed on authentication, expiry,
  decoder, input, or window errors.
- Native Windows runtime wiring owns persistent Media Foundation decoder and
  D3D11 renderer adapters; non-Windows builds retain a portable handoff-only
  pipeline because no native decoder exists there.
- Render jobs use `WindowController::render_latest` and consume the shared
  depth-one slot immediately, avoiding the FIFO command queue for video.
- Added loopback coverage for pre-transport session rejection, expiry cleanup,
  window-close cleanup, session identity, and a second-empty render drain.
- Corrected status documentation to report the unavailable MinGW compiler
  rather than claiming GNU-target evidence.

## Review fix round 2

- Added bounded `ClientRuntime::reconnect`, which reuses the authenticated
  `ClientSession`, revalidates the reconnect window, preserves the session ID,
  and rejects reconnect after expiry.
- Moved Windows decoder/renderer calls behind a dedicated latest-job pipeline
  worker. Tokio receive work only replaces a bounded pending slot and polls a
  bounded error channel; decoder/device errors still fail closed.
- Shutdown now uses the caller's absolute deadline for the pipeline and window
  workers, retaining a reaper when a native worker exceeds that deadline.
- Added explicit validated `VideoStreamConfig`; native decoder dimensions come
  from authenticated/negotiated stream configuration rather than a hardcoded
  1280x720 default.
- Expanded loopback coverage for reconnect identity/state and corrected the
  generated brief/report metadata.
- Replaced the binary's unconditional stub return with the validated
  `ClientRuntime::run_configured` entry boundary; it reports the missing
  authenticated control-plane bootstrap without reading private keys or
  browser credentials.

## Review fix verification

- `cargo fmt --all -- --check` — passed.
- `cargo test -p nexus-client --all-targets` — passed (46 tests).
- `cargo clippy -p nexus-client --all-targets -- -D warnings` — passed.
- `cargo build --workspace` — passed.
- `cargo test --workspace` — passed.
- `cargo check -p nexus-client --tests --target x86_64-pc-windows-gnu` —
  remains blocked by the missing `x86_64-w64-mingw32-gcc` compiler.
