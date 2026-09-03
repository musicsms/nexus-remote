# Task 5 Report: Media Foundation H.264 Encoder Lifecycle

## Status

Implemented the Windows H.264 encoder lifecycle with private deterministic
unit contracts and a compile-checked native Media Foundation boundary. The
native path fails closed when unavailable; no software or simulated encoder
is reported as Media Foundation success. The public `VideoEncoder` API still
returns one `EncodedFrame` per successful call; `CodecError::OutputPending`
means an accepted asynchronous input has no output available yet.

The chronological fix-round evidence below contains historical commands and
test counts. Current validation uses the default private `codec.rs` unit tests:
there is no exported transform seam, `codec_contract` integration target, or
`test-support` feature in the current tree.

## TDD Evidence

### Configuration RED

Command:

```text
cargo test -p platform-windows --lib codec::tests configuration
```

Observed failure:

```text
error[E0432]: unresolved imports `platform_windows::EncoderTransform`,
`platform_windows::WindowsH264Encoder`
```

This was the expected absence failure before the encoder boundary existed.

### Configuration GREEN

The same focused private codec-unit test passed after the minimal state
boundary was implemented. Current verification intentionally does not record
an obsolete filtered-test count.

Covered behavior:

- invalid values never reach the transform;
- valid H.264 configuration reaches it exactly once;
- encoding before configuration returns `CodecError::NotConfigured`;
- mismatched dimensions return `CodecError::FrameDimensionsMismatch`;
- malformed BGRA data never reaches the transform.

The malformed-frame test was separately observed RED as a compile failure
because `CodecError::InvalidFrame` did not exist, then GREEN after adding the
minimal portable error and validation.

### Keyframe and Reconfiguration RED

Command:

```text
cargo test -p platform-windows --lib codec::tests keyframe
```

Observed result before transition logic:

```text
running 5 tests
2 passed; 3 failed
```

The intended failures were:

- bitrate-only reconfiguration incorrectly forced a keyframe;
- reconfiguration did not drain before configuring;
- failed replacement configuration left the old configuration usable.

The request-keyframe and dimension-change assertions already passed because
their state was introduced during the minimal configuration implementation.

### Keyframe and Reconfiguration GREEN

The same focused private codec-unit test passed after implementing validation,
unavailable intermediate state, drain-before-configure ordering, conditional
dimension forcing, and commit-after-success.

### Lifecycle and Conversion RED/GREEN

`lifecycle_drop_drains_before_shutting_down_the_transform` first failed with
only `Configure` recorded instead of `Configure, Drain, Shutdown`, then passed
after the drop lifecycle was implemented.

The Windows GNU test check initially failed because
`WindowsH264Encoder::new()` was absent. It passed after the real native worker
and constructor were implemented.

The CPU conversion tests first failed to compile because `bgra_to_nv12` was
absent. They then passed with the literal 2x2 black-frame NV12 result
`[16, 16, 16, 16, 128, 128]` and odd-dimension rejection.

## Native Scope

- `WindowsH264Encoder::new()` starts a named `nexus-windows-encoder` thread.
- The worker initializes a multithreaded COM apartment and calls `MFStartup`.
- `MFTEnumEx` selects only hardware, sorted/filtered H.264 encoder
  transforms with NV12 input and H.264 output registration types.
- Media types configure frame size, cadence, pixel aspect ratio, progressive
  scan, and bitrate.
- Asynchronous MFTs are detected through `MF_TRANSFORM_ASYNC`, unlocked with
  `MF_TRANSFORM_ASYNC_UNLOCK`, and driven with NeedInput, HaveOutput, and
  DrainComplete events.
- Forced keyframes use `ICodecAPI` and a `VT_UI4` value for
  `CODECAPI_AVEncVideoForceKeyFrame`.
- BGRA conversion tries a D3D11 video processor first. Initialization-time D3D
  unavailability selects the checked CPU NV12 fallback. Runtime D3D/MF device
  loss is surfaced instead of silently switching implementations.
- Output bytes and Media Foundation clean-point metadata become the portable
  `EncodedFrame` without exposing Windows types.
- Drop drains the configured transform, requests worker shutdown, calls
  `IMFShutdown::Shutdown` when available, releases native interfaces, balances
  `MFShutdown`, and uninitializes COM on the owning worker thread.
- Known DXGI removal/reset/hung and Media Foundation hardware-start failures
  map to `CodecError::BackendLost`; initialization/unavailable failures map to
  `CodecError::BackendUnavailable`.

## Portable Contract Changes

The public `VideoEncoder`, `EncoderConfig`, `CapturedFrame`, and `EncodedFrame`
shapes are unchanged. `CodecError` gained only the portable variants required
at this boundary:

- `InvalidFrame`
- `BackendUnavailable`
- `BackendLost`

The deterministic recording transform and all Windows interfaces are private
to `codec.rs`; no transform test seam is exported from the platform crate.

## Files

- `crates/nexus-codec/src/types.rs`
- `platform/windows/src/codec.rs`
- `platform/windows/src/lib.rs`
- `platform/windows/tests/windows_codec_smoke.rs`

## Fresh Verification

All commands completed successfully:

```text
cargo test -p platform-windows --lib codec::tests
cargo test -p nexus-codec -p platform-windows
cargo clippy -p nexus-codec -p platform-windows --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p platform-windows --tests --target x86_64-pc-windows-gnu
cargo clippy -p platform-windows --all-targets --target x86_64-pc-windows-gnu -- -D warnings
git diff --check
```

## Fix Round 4

### Status

Corrected desktop-host packet metadata for delayed asynchronous encoder output
and made `OutputPending` a non-fatal streaming result.

### Changes

- Packet headers now use `EncodedFrame.frame_id` and
  `EncodedFrame.timestamp_us`, ensuring the header and AEAD associated data
  describe the bytes that were actually encoded even when output belongs to an
  earlier input.
- The streamer handles `CodecError::OutputPending` by emitting no datagrams
  for that cycle and continuing normally; all other codec errors retain their
  existing propagation behavior.
- Removed the streamer-local frame ID counter, which could not represent the
  metadata of delayed output.

### TDD Evidence

RED:

```text
cargo test -p nexus-desktop-host streamer::tests
error[E0599]: no method named `packetize_encoded_frame` found
error[E0599]: no method named `packetize_encode_result` found
```

GREEN:

```text
cargo test -p nexus-desktop-host streamer::tests
test result: ok. 2 passed; 0 failed
```

The metadata regression decodes a sealed datagram and verifies that both
header fields match deliberately delayed encoded metadata. The pending-output
regression verifies that `OutputPending` returns an empty packet list without
an error.

### Fresh Verification

All completed with exit status 0:

```text
cargo fmt --all
cargo test -p nexus-desktop-host
cargo test -p nexus-codec -p platform-windows
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p platform-windows --tests --target x86_64-pc-windows-gnu
cargo clippy -p platform-windows --all-targets --target x86_64-pc-windows-gnu -- -D warnings
git diff --check
```

Focused package suites completed with zero failures. Exact counts are omitted
because the old codec-contract target and its historical counts no longer
exist; the entire workspace test command also completed with zero failures.

## Self-Review

- Re-read the task brief and design acceptance criteria against the final code.
- Confirmed configuration validation occurs before adapter calls and before
  state mutation.
- Confirmed replacement configuration is committed only after drain and
  successful native configuration; failures leave the encoder unavailable.
- Confirmed bitrate-only reconfiguration preserves a pending explicit
  keyframe request but does not create a new one.
- Confirmed the keyframe request is cleared only after successful encode.
- Confirmed checked frame sizing and pixel-format validation happen before
  native conversion.
- Confirmed all COM, Media Foundation, and D3D11 values are private to and
  created/destroyed on the encoder worker.
- Confirmed initial D3D fallback and runtime device-loss behavior do not claim
  fake native success.
- Confirmed no placeholders, unrelated changes, whitespace errors, or compiler
  and clippy warnings remain.

## Hardware and Environment Limitations

- Verification ran on Linux; the ignored Media Foundation smoke test could not
  be executed against a real Windows encoder or GPU.
- The Windows GNU target compile-check proves Rust/API boundary compilation,
  not that a particular vendor MFT accepts the negotiated types or produces
  output.
- No Windows MSVC SDK runner was available in this environment.
- D3D11 video-processor selection, CPU fallback selection, hardware keyframe
  control, actual H.264 access-unit contents, driver-loss recovery, shutdown
  latency, and performance remain to be exercised on Windows hardware.
- The smoke test is deliberately ignored with the exact reason
  `requires Media Foundation H.264 encoder availability` and requires a
  non-empty access unit plus first-output keyframe when run.

## Fix Round 1

### Status

Addressed the review findings in the Media Foundation worker without
simulating successful hardware encoding. The deterministic transform seam is
now opt-in through the `platform-windows/test-support` Cargo feature rather
than part of the normal public release surface.

### Changes

- Replaced the async MFT readiness Boolean with a bounded eight-credit pump.
  `METransformNeedInput`, `METransformHaveOutput`, and drain-complete events
  are counted independently; zero, one, or multiple output events are pumped
  without treating one input as one immediate output.
- Replaced blocking startup/request/shutdown response waits with bounded
  `recv_timeout` operations; command submission is non-blocking, and `Drop`
  only joins a worker after `is_finished()` confirms it cannot wait. MFT event
  polling uses `MF_EVENT_FLAG_NO_WAIT` and an explicit request deadline.
- Negotiates constrained-baseline H.264 through `MF_MT_MPEG2_PROFILE`, obtains
  `MF_MT_MPEG_SEQUENCE_HEADER` from the negotiated output type, requires a
  non-empty Annex-B SPS/PPS header, and prefixes every clean-point access unit.
  Missing negotiation/header data fails with `BackendUnavailable`.
- Uses `MFCreateAlignedMemoryBuffer` when `cbAlignment` is non-zero, honors
  `cbSize`, and ignores `MFT_OUTPUT_DATA_BUFFER_NO_SAMPLE` and zero-length
  buffers so no empty `EncodedFrame` can be emitted.
- Added checked NV12 layout arithmetic, a single documented mapped-D3D unsafe
  boundary, and a safe padded-row copy helper. Converter resources are cleared
  before the COM apartment can drop even when drain reports an error.

### TDD Evidence

RED (new behaviors absent):

```text
cargo test -p platform-windows --lib codec::tests
error[E0422]: cannot find struct, variant or union type `OutputBufferSpec`
error[E0433]: cannot find type `AsyncPump` in this scope
error[E0425]: cannot find function `output_sample_is_usable` in this scope
error[E0433]: cannot find type `Nv12Layout` in this scope
error[E0425]: cannot find function `receive_response` in this scope
```

Second RED (sequence handling/copy behavior absent):

```text
cargo test -p platform-windows --lib codec::tests
error[E0425]: cannot find function `h264_access_unit` in this scope
error[E0425]: cannot find function `copy_pitched_nv12` in this scope
```

GREEN:

```text
cargo test -p platform-windows --lib codec::tests
test result: ok. 9 passed; 0 failed

cargo test -p platform-windows --test codec_contract --features test-support
test result: ok. 12 passed; 0 failed
```

The new deterministic coverage proves that repeated NeedInput credits survive
before output arrives, a stalled response is bounded, aligned output sizing is
preserved, no-sample/empty output is suppressed, keyframes receive the
negotiated Annex-B header, and padded mapped NV12 rows copy only valid bytes.

### Fresh Verification

All completed with exit status 0:

```text
cargo fmt --all -- --check
cargo test -p nexus-codec -p platform-windows --features test-support
cargo clippy -p nexus-codec -p platform-windows --all-targets --features test-support -- -D warnings
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p platform-windows --tests --target x86_64-pc-windows-gnu --features test-support
cargo clippy -p platform-windows --all-targets --target x86_64-pc-windows-gnu --features test-support -- -D warnings
git diff --check
```

That historical package run passed with no failures. Its exact count and the
retired codec-contract target are intentionally not presented as current
validation; the full workspace test command also passed with no failures.

### Self-Review

- Rechecked every command path: bounded channels, command waits, event polling,
  output queues, and MFT callbacks cannot make the public caller or `Drop`
  wait indefinitely.
- Verified that each async credit is counted, processing is bounded to eight
  pending input/output records, and surplus/no-sample output is never emitted
  as a fabricated frame.
- Verified profile/header negotiation and clean-point validation fail closed;
  SPS/PPS are emitted only in valid Annex-B H.264 access units.
- Verified output allocation follows MFT `cbSize`/`cbAlignment`, and all NV12
  source/destination dimensions and mapped row lengths use checked arithmetic
  before the one documented unsafe slice construction.
- Verified normal builds do not export `EncoderTransform` or `with_transform`;
  contract tests must explicitly enable `test-support`.

### Hardware Limitations

- Verification ran on Linux. Windows GNU checks compile the native boundary but
  do not execute Media Foundation, a vendor H.264 MFT, D3D11 conversion, or
  the ignored smoke test.
- A hardware MFT that does not expose `MF_MT_MPEG_SEQUENCE_HEADER` immediately
  after negotiation now fails explicitly; this is deliberate rather than
  emitting a keyframe whose SPS/PPS are unavailable.
- Hardware-specific timing, event ordering, profile acceptance, sample-time
  preservation, GPU/device-loss recovery, output buffer alignment acceptance,
  and shutdown latency still require an interactive Windows hardware run.

## Fix Round 2

### Status

Addressed the second scoped review of the Media Foundation encoder worker.
The encoder boundary now exposes the asynchronous reality of hardware MFTs:
one submitted input may produce zero, one, or multiple access units during a
pump cycle. `VideoEncoder::encode` therefore returns `Vec<EncodedFrame>`;
the software encoder returns one element, while the desktop streamer seals and
packetizes every emitted frame with its matched capture metadata.

### Changes

- Replaced the per-command async-output wait with an input scheduler. It queues
  bounded submissions, consumes every independent NeedInput credit, pumps
  events before and after submissions, and lets the worker keep pumping during
  idle command periods. Multiple credited inputs can now reach a valid MFT
  before its first output exists.
- Corrected `MFCreateAlignedMemoryBuffer` use to pass a checked alignment mask
  (`cbAlignment - 1`) only for non-zero power-of-two alignment requirements.
  ProcessOutput now treats `pdwStatus` as `_MFT_PROCESS_OUTPUT_STATUS` and
  repeats calls when `MFT_OUTPUT_DATA_BUFFER_INCOMPLETE` is set. No-sample and
  empty buffers remain non-emittable.
- Replaced timeout-time JoinHandle drops with `WorkerLifecycle`. Startup,
  request, and shutdown callers remain bounded, while a named reaper retains
  and joins the native worker so its COM, MF, and D3D cleanup eventually runs.
- Removed the public `test-support` feature, exported adapter trait, and
  simulated constructor. Deterministic transform coverage now lives in private
  crate unit tests and runs in the default workspace suite.
- Made the converter release on every drain result explicit and covered that
  failure path with a deterministic unit test.

### TDD Evidence

RED:

```text
cargo test -p platform-windows --lib codec::tests
error[E0433]: cannot find type `AsyncInputScheduler` in this scope
error[E0425]: cannot find function `output_requires_retry` in this scope
error[E0433]: cannot find type `WorkerLifecycle` in this scope
```

The converter cleanup test then failed because
`clear_converter_after_drain` was absent, and the incomplete-output loop test
failed because `OutputPoll` / `collect_output_polls` were absent.

GREEN:

```text
cargo test -p platform-windows --lib codec::tests
59 passed; 0 failed

cargo test -p nexus-codec -p platform-windows
historical package suite passed with no failures
```

The scheduler regression queues two inputs, supplies two NeedInput credits,
submits both before any HaveOutput event, then observes output. The incomplete
output regression exercises the same bounded output-collection loop used by
the native ProcessOutput path. The lifecycle regression proves that a bounded
caller leaves a join owner until the worker exits and its handle is reaped.

### Fresh Verification

All completed with exit status 0:

```text
cargo fmt --all -- --check
cargo test -p nexus-codec -p platform-windows
cargo test -p nexus-transport --test phase0_e2e_pipeline
cargo clippy -p nexus-codec -p platform-windows --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p platform-windows --tests --target x86_64-pc-windows-gnu
cargo clippy -p platform-windows --all-targets --target x86_64-pc-windows-gnu -- -D warnings
git diff --check
```

### Hardware Limitations

- Linux verification and Windows GNU cross-compilation do not execute an
  actual hardware MFT, D3D converter, or ignored codec smoke test.
- Exact driver event timing, delayed final output handling under a real capture
  shutdown, and vendor alignment acceptance remain Windows-hardware checks.

## Fix Round 3

### Status

Restored the platform-neutral `VideoEncoder::encode` contract to
`Result<EncodedFrame, CodecError>`. Delayed asynchronous MFT output is now an
explicit `CodecError::OutputPending` result after the input has been accepted;
the worker retains its command channel, continues pumping MFT events, and
returns exactly one oldest pending output on a later encode command.

### Changes

- Kept the bounded async input/output queues from Fix Round 2, but changed the
  transform, native worker command, software encoder, streamer consumer, and
  Windows smoke test back to one encoded frame per successful encode call.
- Added `OutputPending` as the explicit non-fatal no-output availability state.
  It does not mark the Media Foundation worker as lost, and an accepted input
  still clears the forced-keyframe flag.
- Added a deterministic async command regression that supplies two MFT input
  credits, submits two distinct encode commands before any output, observes
  `OutputPending` for each, then confirms a delayed output is retrieved singly.
- Restored default-run Windows codec contract coverage as private crate unit
  tests. These tests cover initial `NotConfigured`, dimension mismatch,
  malformed frames, explicit keyframe request, bitrate-only reconfiguration
  cadence, failed reconfiguration, and non-Windows fail-closed construction.
  The recording transform remains private to `codec.rs`; no public test seam
  or `test-support` feature is exposed.

### TDD Evidence

RED:

```text
cargo test -p nexus-codec encoder_returns_one_encoded_frame_per_input
error[E0609]: no field `frame_id` on type `Vec<types::EncodedFrame>`
error[E0609]: no field `timestamp_us` on type `Vec<types::EncodedFrame>`

cargo test -p platform-windows pending_output_does_not_close_the_worker_request_channel
error[E0425]: cannot find function `response_requires_worker_shutdown` in this scope
```

GREEN:

```text
cargo test -p nexus-codec encoder_returns_one_encoded_frame_per_input
test software::tests::encoder_returns_one_encoded_frame_per_input ... ok

cargo test -p platform-windows
67 passed; 0 failed
```

### Fresh Verification

All completed with exit status 0:

```text
cargo fmt --all -- --check
cargo test -p platform-windows
cargo test -p nexus-codec
cargo test -p nexus-transport --test phase0_e2e_pipeline
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p platform-windows --tests --target x86_64-pc-windows-gnu
cargo clippy -p platform-windows --all-targets --target x86_64-pc-windows-gnu -- -D warnings
git diff --check
```

## Fix Round 4 (continued)

The Fix Round 4 implementation and TDD evidence are recorded above; this
marker keeps the round visible at the end of the report after the historical
Fix Round 3 section.

### Fix Round 4 Details

Packet headers use `EncodedFrame.frame_id` and `EncodedFrame.timestamp_us`,
including for delayed asynchronous output, so the header and AEAD associated
data match the encoded bytes. `CodecError::OutputPending` emits no datagrams
and is treated as non-fatal; all other codec errors still propagate.

TDD RED was observed with missing `packetize_encoded_frame` and
`packetize_encode_result` methods. GREEN passed two focused streamer tests:
metadata preservation and non-fatal pending output.

Fresh verification passed: desktop-host tests, codec/platform tests, workspace
build, workspace clippy, Windows GNU check/clippy, formatting, and diff check.

## Final Fix Wave (2026-09-02)

The streamer now rejects an `EncodedFrame.frame_id` that cannot fit the
protocol's `u32` wire header instead of silently narrowing it. The regression
uses `u32::MAX + 1` and receives `StreamerError::FrameIdOutOfRange`.
`CodecError::OutputPending` remains explicitly non-fatal: it emits no
datagrams for that cycle, while every other codec error still propagates.

The final wave reran formatting, the workspace build/test/clippy gates, GNU
Windows target check/clippy, and `git diff --check`; all passed. Live Media
Foundation execution remains an ignored interactive-Windows smoke test.
