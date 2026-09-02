# Task 3 report — bounded renderer and Windows decode boundary

## Outcome

- Added `RenderQueue`, a shared depth-one handoff that replaces stale pending
  jobs, counts replacements, validates bounded non-empty access units, and
  rejects jobs after explicit shutdown.
- Added private portable `FrameDecoder` / `DecodedSurface` contracts with
  bounded even dimensions and exact NV12/RGBA surface layouts.
- Added private `cfg(windows)` Media Foundation H.264 decoder and D3D11 upload
  renderer workers named `nexus-client-decoder` and `nexus-client-renderer`.
  They own COM/MF/D3D objects, use bounded command/reply channels, validate an
  initial keyframe sequence header, and surface backend loss explicitly.
- Added an ignored Windows-only smoke entrypoint that initializes both native
  adapters and uploads one synthetic decoded surface. It is not presented as
  evidence of GUI/GPU success on GNU/Linux.

## TDD evidence

`cargo test -p nexus-client --test render_queue` was first run with the
render-queue imports missing. It failed with unresolved `RenderQueue` and
`RenderQueueError` imports. After implementation, the three tests passed:
latest-frame replacement/drop count, empty-access-unit rejection without
replacement, and shutdown rejection.

## Verification

- `cargo fmt --all -- --check` — passed
- `cargo test -p nexus-client` — passed (29 tests; Windows smoke has 0 tests
  on Linux because its crate is `cfg(windows)`)
- `cargo clippy -p nexus-client --all-targets -- -D warnings` — passed
- `git diff --check` — passed
- `cargo check -p nexus-client --tests --target x86_64-pc-windows-gnu` — not
  completed: this environment lacks `x86_64-w64-mingw32-gcc`, so `ring` fails
  during its build script before client Windows sources are compiled. This is
  an environment limitation, not a passing Windows validation result.

## Follow-up boundary

Task 4 owns the HWND and Task 5 owns the authenticated end-to-end runtime.
The renderer currently creates its dedicated D3D11 device and upload texture;
it deliberately does not create an HWND-bound swap chain before the window
thread exists.

## Fix round 1

- Added portable regression tests for decoder initialization gating, negotiated
  surface bounds, and row-pitch repacking. The new tests were run RED first
  (missing `DecoderGate` and `repack_nv12`) and then GREEN.
- Initialization now requires a keyframe with an H.264 sequence header only
  until the decoder is initialized; normal inter frames are accepted after it.
- Native output draining now loops over incomplete output, associates output
  metadata using Media Foundation sample timestamps, validates dimensions
  before native startup/allocation, repacks padded NV12 rows, releases every
  unselected MFT activation, and moves timed-out worker joins to a reaper.
- Added an HWND-capable D3D11 swap-chain startup path and Present path; Task 4
  supplies the HWND. The existing ignored smoke stays explicitly non-Windows
  on GNU/Linux and cannot be treated as live GPU evidence.

## Fix round 2

- Replaced the stale native `nv12_len` reference with the validated NV12
  length helper used by the portable surface contract.
- Bounded MFT output-buffer allocation by the 64 MiB surface limit and
  validated Media Foundation current/max/contiguous lengths before every
  native slice. Padded `IMF2DBuffer` rows now require both a valid pitch and a
  sufficient contiguous extent before repacking.
- Continued draining when Media Foundation reports `NO_SAMPLE | INCOMPLETE`
  so delayed output is not silently abandoned.
- Made swap-chain presentation dimensions explicit: resize on decoded-size
  changes, re-check the actual backbuffer immediately before `CopyResource`,
  and reject any remaining extent/format mismatch. RGBA surfaces are converted
  to BGRA before uploading to the swap chain.
- Updated the ignored Windows smoke to build and authenticate a receiver
  datagram, decode its resulting frame job through Media Foundation, and
  present that surface to an operator-provided HWND.

## Fix round 2 verification

- `cargo fmt --all` — passed
- `cargo test -p nexus-client` — passed (29 tests)
- `cargo clippy -p nexus-client --all-targets -- -D warnings` — passed
- `git diff --check` — passed
- Live Windows Media Foundation/D3D11 smoke remains intentionally ignored and
  was not run in this GNU/Linux environment.
