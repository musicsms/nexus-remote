# Windows Phase 1 Platform Backends Design

## Goal

Deliver the next Phase 1 slice: production-shaped Windows capture, H.264
encoding, semantic input injection, and cursor backends behind safe,
OS-independent contracts. In the same implementation series, synchronize the
repository status documents with the code that already exists.

This slice does not complete Phase 1. Native client rendering, the Windows
service/desktop-host privilege boundary, full relay session integration, and
real-hardware acceptance measurements remain later Phase 1 work.

## Architectural Constraints

- Windows Graphics Capture (WGC) is the primary interactive-session capture
  API. DXGI Desktop Duplication is the runtime fallback, per ADR-026.
- Capture and encoder work runs on dedicated native threads; Windows API and
  blocking work never runs on Tokio worker threads.
- Windows types remain inside `platform/windows`. Core crates expose portable
  frame, encoder, input, cursor, and error contracts.
- Capture-to-encode buffering has depth one with replace-not-block semantics,
  per ADR-022.
- H.264 is the Phase 1 codec. Media Foundation is the initial Windows hardware
  path, with the existing deterministic software encoder retained for tests
  and unsupported hardware.
- All externally supplied input and cursor data is bounded and validated
  before it reaches a Windows API.
- OS/COM/FFI code is isolated in narrow modules with documented invariants.
- Non-Windows builds fail closed and continue to verify portable contracts.

## Module Boundaries

`platform/windows` is split into four focused modules:

- `capture`: owns WGC/DXGI objects on a dedicated thread and emits validated
  BGRA frames through the existing latest-frame contract. Initialization,
  access loss, device removal, and shutdown are explicit states.
- `codec`: owns Media Foundation startup, encoder selection, BGRA-to-NV12
  conversion, H.264 configuration, forced keyframes, reconfiguration, and
  device-loss reporting.
- `input`: translates validated semantic keyboard, text, mouse, and wheel
  events into `SendInput` records. It uses scan codes where possible, bounds
  Unicode text, rejects unsupported keys, and clamps absolute coordinates.
- `cursor`: captures cursor position, visibility, hotspot, and bounded shape
  data separately from the video stream so the client can render it locally.

The platform crate implements or adapts the existing core traits. It must not
create a second competing model for frames or input events. Where an existing
trait is too narrow for lifecycle or recoverable-error semantics, the core
trait is extended minimally and remains platform-neutral.

## Capture and Encoding Flow

1. The desktop-host requests a Windows capture source.
2. A dedicated capture thread initializes COM and attempts WGC.
3. If WGC is unavailable at initialization, the backend records the reason and
   attempts DXGI Desktop Duplication. Runtime access/device loss is surfaced as
   recoverable rather than silently switching with stale GPU state.
4. Captured dimensions, stride, pixel format, and payload length are validated.
5. The newest frame replaces any older unconsumed frame in the depth-one queue.
6. The encoder thread converts BGRA/RGBA to NV12 and submits it to Media
   Foundation. A resolution change reconfigures the encoder and forces the
   next output to be a keyframe.
7. Encoded output uses the existing `EncodedFrame` metadata and continues into
   the existing authenticated packetization pipeline.

The initial implementation may use a CPU-visible copy at the safe boundary if
the existing `CapturedFrame` contract requires it. Zero-copy D3D11 texture
handoff is an optimization only after correctness, device-loss behavior, and
latency instrumentation are established.

## Error and Lifecycle Model

Backend errors distinguish unsupported platform/API, permission denial,
invalid configuration, initialization failure, device/access loss, malformed
input, and shutdown. Recoverable device/access loss is observable by the
desktop-host so a later service/runtime slice can rebuild the backend without
terminating authorization state.

Startup and shutdown are bounded. Dropping a backend requests shutdown and
joins owned native threads outside Tokio workers. Partial initialization
releases COM, Media Foundation, and GPU resources in reverse ownership order.
No backend silently falls back after processing has begun unless it can first
tear down all state and revalidate dimensions and encoder configuration.

## Security and Validation

- Keyboard injection accepts defined semantic events and an allow-listed set
  of scan-code/virtual-key forms; raw Windows structures are not accepted over
  the network.
- Text length and cursor shape payload sizes retain explicit protocol limits.
- Mouse coordinates are checked for finite/ranged values and clamped only after
  monitor-space mapping is validated.
- Integer calculations for stride, dimensions, and payload size use checked
  arithmetic.
- Cursor hotspots must lie within cursor bounds.
- Privileged input auditing remains the responsibility of the desktop-host
  orchestration layer; this backend returns enough structured outcome data for
  that layer to audit success or denial.

## Testing Strategy

Implementation follows red-green-refactor for every new behavior.

- Portable unit tests verify configuration validation, checked frame sizing,
  recoverable error classification, input translation rules, coordinate
  mapping, cursor bounds, reconfiguration, and forced-keyframe behavior.
- Non-Windows tests verify every constructor and operation fails closed rather
  than pretending native functionality is available.
- `cfg(windows)` contract tests use deterministic adapters around the narrow
  native boundary to exercise lifecycle and translation without requiring an
  interactive desktop for every test.
- Windows smoke tests require a real interactive desktop and verify WGC/DXGI
  initialization, at least one valid frame, Media Foundation H.264 output,
  cursor capture, and controlled input injection.
- Workspace format, build, test, and clippy checks remain mandatory. Windows
  target compilation is recorded separately from real-hardware smoke results.

Tests that require Windows hardware are explicitly skipped or unavailable on
Linux; Linux success must never be reported as proof that native capture or
encoding works.

## Documentation Synchronization

The implementation series updates `README.md`, `CLAUDE.md`,
`docs/IMPLEMENTATION_STATUS.md`, and the Phase 1 plan so they consistently
state:

- the OS-independent Phase 0 foundation is complete;
- Phase 1 is in progress;
- SQLite control-plane persistence and Windows platform scaffolding exist;
- ADR-026 and ADR-027 are accepted;
- native Windows backends are only marked implemented after their relevant
  code and tests exist;
- Phase 1 is not marked done until the full Windows host/client-through-relay
  exit condition and measurements pass.

## Out of Scope

- Native Windows client windowing, decoding, or Direct3D rendering.
- Agent service installation, named-pipe IPC, privilege switching, Winlogon,
  or Secure Desktop UX.
- Phase 2 P2P, NAT traversal, and adaptive bitrate.
- Audio, file transfer, recording, multi-monitor streaming, or non-Windows
  host support.
- Claiming live Windows acceptance based only on cross-compilation.

## Acceptance Criteria

- The four backend modules have safe, documented contracts and fail closed on
  unsupported platforms.
- Validation and lifecycle behaviors are covered by tests observed failing
  before their implementations are added.
- Windows-target code compiles for the supported MSVC target when that target
  and SDK bindings are available.
- Real Windows smoke results are recorded honestly; unavailable hardware is an
  explicit remaining condition, not treated as success.
- Repository status documents match the resulting implementation and retain
  Phase 1 as in progress.
- `cargo fmt --all -- --check`, `cargo build --workspace`,
  `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings` pass.
