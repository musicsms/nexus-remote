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
- Replaced the binary no-op with tracing plus validated endpoint and explicit
  authenticated bootstrap configuration. The entrypoint constructs/runs the
  runtime when capability, relay token, certificate, dimensions, and frame key
  are supplied, and returns an explicit error when bootstrap is absent; no
  unattended private-key or browser handling was added.
- Updated implementation status and README while keeping Phase 1 **In
  progress** and documenting the missing MSVC/live-Windows/full acceptance
  evidence.

## Verification

- `cargo fmt --all -- --check` — passed.
- `cargo test -p nexus-client --all-targets` — passed (52 tests including the
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
- `cargo test -p nexus-client --all-targets` — passed (52 tests).
- `cargo clippy -p nexus-client --all-targets -- -D warnings` — passed.
- `cargo build --workspace` — passed.
- `cargo test --workspace` — passed.
- `cargo check -p nexus-client --tests --target x86_64-pc-windows-gnu` —
  remains blocked by the missing `x86_64-w64-mingw32-gcc` compiler.

## Review fix round 3

- Reconnect now retries transient endpoint/transport failures on a bounded
  timer and keeps the authenticated session in `Reconnecting` until the
  original reconnect deadline expires or shutdown is requested.
- Transport loss increments a native pipeline generation, clears its pending
  slot, and drops stale decoded output; the next submitted job starts a fresh
  decoder continuity epoch and therefore requires a keyframe.
- The configured binary path now parses the control-plane capability, relay
  metadata/signatures, trusted server certificate, negotiated monitor/stream
  dimensions, and frame key from explicit bootstrap environment variables,
  then constructs `ClientRuntime::connect` and runs/shuts it down. No private
  identity key or browser credential is read.
- Added reconnect-failure state coverage and decoder-gate reset coverage.

## Review fix round 4

- Reconnect generation changes now invoke the native Media Foundation decoder
  reset command, flushing MFT pending input/output and resetting keyframe gate
  state before the next post-gap job.
- Configured runtime orchestration loops from `run()` through
  `reconnect_with_retry()` after clean transport loss instead of shutting down
  after the first connection.
- Added shared cancellation notification so an owner can interrupt reconnect
  sleeps and in-flight connect waits with `request_shutdown`, then perform the
  bounded final worker join through `shutdown`.
- Hex parsing checks ASCII and byte pairs before conversion, with malformed
  Unicode coverage; configured bootstrap absence is covered by an explicit
  fail-closed entrypoint test.

## Review fix round 5

- Native pipeline workers now maintain their own generation cursor. Jobs older
  than the shared reconnect generation are discarded without resetting or
  terminating the decoder; newer generations invoke the explicit decoder reset
  and reject deltas until a keyframe arrives.
- Exported cloneable `RuntimeCancellation`/`ShutdownHandle` backed by an atomic
  flag and permit-retaining `Notify`, and retained the handle through the
  configured run/reconnect path for prompt cancellation.
- Added configured-entrypoint, malformed-Unicode, decoder-reset, and
  generation/stale-job regression coverage; report count refreshed to 52.

## Review fix round 6

- Added `run_configured_with_cancellation` and kept `run_configured` as a
  convenience wrapper with a fresh token. `main` now retains a cloneable
  handle and cancels the configured runtime on Ctrl-C.
- The configured path test proves a caller-cancelled handle exits before
  bootstrap parsing or transport setup; report count refreshed to 52.
