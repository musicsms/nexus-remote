# Windows Phase 1 Platform Backends Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add safe, validated Windows capture, H.264 encoder, input, and cursor backend boundaries for the next Phase 1 slice while synchronizing repository status documentation.

**Architecture:** Portable validation and lifecycle types live in the existing core crates; `platform/windows` owns all Windows API types and native-thread resources. Native operations are reached through small adapter traits so deterministic contract tests run on Linux, while `cfg(windows)` smoke tests prove actual WGC/DXGI, Media Foundation, `SendInput`, and cursor integration on Windows.

**Tech Stack:** Rust 2021, `windows` 0.58, Windows Graphics Capture, DXGI Desktop Duplication, Media Foundation, `SendInput`, Win32 cursor APIs, existing `nexus-capture`, `nexus-codec`, and `nexus-input` crates.

**Spec:** `docs/superpowers/specs/2026-08-31-windows-platform-backends-design.md`

## Global Constraints

- Windows host and client are the only Phase 1 platform target.
- WGC is primary capture and DXGI Desktop Duplication is fallback per ADR-026.
- Capture and encoder objects are owned by dedicated native threads, never Tokio workers.
- Capture-to-encode buffering is depth one and replace-not-block per ADR-022.
- H.264 is mandatory; a resolution change forces a keyframe.
- Raw Windows structures never cross the platform crate boundary.
- Every dimension, stride, coordinate, and payload calculation uses checked validation.
- Non-Windows implementations fail closed.
- Linux verification is not evidence of working native Windows APIs.

---

### Task 1: Synchronize Repository Status

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/IMPLEMENTATION_STATUS.md`
- Modify: `docs/superpowers/plans/2026-08-27-phase-1-mvp-implementation.md`

**Interfaces:**
- Consumes: commits through the accepted ADR-026/027 and Windows scaffold.
- Produces: one consistent statement that Phase 0 foundation is complete and Phase 1 is in progress.

- [ ] **Step 1: Write the expected status assertions**

Run this shell assertion before editing:

```bash
rg -n "pre-Phase-0|Phase 1.*Not started|All 25|24/24|ADR-025" README.md CLAUDE.md docs/IMPLEMENTATION_STATUS.md
```

Expected: matches identify stale Phase 0, Phase 1, and ADR counts.

- [ ] **Step 2: Update the status documents**

Make these exact semantic changes:

```text
README: Phase 0 foundation complete; Phase 1 MVP in progress.
CLAUDE: replace pre-Phase-0 language with current Phase 1 language and make ADR counts non-stale.
IMPLEMENTATION_STATUS: set Last audited to 2026-08-31, Phase 1 to In progress,
and index ADR-026 and ADR-027.
Phase 1 plan: check Task 1 items that are evidenced by commits and leave hardware-only checks unchecked.
```

- [ ] **Step 3: Verify stale assertions no longer match**

Run:

```bash
rg -n "pre-Phase-0|Phase 1.*Not started|All 25|24/24" README.md CLAUDE.md docs/IMPLEMENTATION_STATUS.md
```

Expected: no matches.

- [ ] **Step 4: Verify documentation diff**

Run: `git diff --check && git diff -- README.md CLAUDE.md docs/IMPLEMENTATION_STATUS.md docs/superpowers/plans/2026-08-27-phase-1-mvp-implementation.md`

Expected: no whitespace errors; the diff does not mark Phase 1 done.

- [ ] **Step 5: Commit**

```bash
git add README.md CLAUDE.md docs/IMPLEMENTATION_STATUS.md docs/superpowers/plans/2026-08-27-phase-1-mvp-implementation.md
git commit -m "docs: synchronize Phase 1 implementation status"
```

### Task 2: Harden Portable Frame and Input Validation

**Files:**
- Modify: `crates/nexus-capture/src/frame.rs`
- Modify: `crates/nexus-input/src/events.rs`

**Interfaces:**
- Produces: `CapturedFrame::expected_bgra_len(width, height) -> Result<usize, CaptureError>`.
- Produces: `InputEvent::validate()` rejection of invalid physical key codes and out-of-contract wheel values.
- Consumes: existing `CapturedFrame`, `CaptureError`, and `InputEvent` APIs.

- [ ] **Step 1: Write failing checked-size tests**

Add to `crates/nexus-capture/src/frame.rs` tests:

```rust
#[test]
fn bgra_size_uses_checked_arithmetic() {
    assert_eq!(
        CapturedFrame::expected_bgra_len(u32::MAX, u32::MAX),
        Err(CaptureError::FrameSizeOverflow)
    );
    assert_eq!(CapturedFrame::expected_bgra_len(2, 3), Ok(24));
}
```

- [ ] **Step 2: Run the capture test and observe RED**

Run: `cargo test -p nexus-capture bgra_size_uses_checked_arithmetic -- --exact`

Expected: compile failure because `FrameSizeOverflow` and `expected_bgra_len` do not exist.

- [ ] **Step 3: Implement checked BGRA sizing**

Add `CaptureError::FrameSizeOverflow` and implement:

```rust
pub fn expected_bgra_len(width: u32, height: u32) -> Result<usize, CaptureError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(CaptureError::FrameSizeOverflow)?;
    pixels.checked_mul(4).ok_or(CaptureError::FrameSizeOverflow)
}
```

Use it from `CapturedFrame::validate` after rejecting zero dimensions.

- [ ] **Step 4: Run capture tests and observe GREEN**

Run: `cargo test -p nexus-capture`

Expected: all `nexus-capture` tests pass.

- [ ] **Step 5: Write failing hostile-input tests**

Add to `crates/nexus-input/src/events.rs` tests:

```rust
#[test]
fn rejects_unrepresentable_physical_key_code() {
    let event = InputEvent::Key {
        physical_code: 0x1_0000,
        logical_code: 0,
        action: KeyAction::Down,
        modifiers: Modifiers::NONE,
    };
    assert_eq!(event.validate(), Err(InputError::InvalidPhysicalKeyCode));
}

#[test]
fn rejects_extreme_wheel_delta() {
    let event = InputEvent::MouseWheel { delta_x: 0, delta_y: i32::MAX };
    assert_eq!(event.validate(), Err(InputError::WheelDeltaOutOfRange));
}
```

- [ ] **Step 6: Run input tests and observe RED**

Run: `cargo test -p nexus-input`

Expected: compile failure because both error variants are absent.

- [ ] **Step 7: Implement minimal input validation**

Add error variants and validate physical codes as `u16`; bound each wheel delta to
`-120_000..=120_000`, which permits 1,000 standard wheel detents per message.

- [ ] **Step 8: Run input tests and commit**

Run: `cargo test -p nexus-capture -p nexus-input`

Expected: all tests pass.

```bash
git add crates/nexus-capture/src/frame.rs crates/nexus-input/src/events.rs
git commit -m "fix(core): harden frame and input validation"
```

### Task 3: Define Windows Backend Contracts and Deterministic Adapters

**Files:**
- Modify: `platform/windows/Cargo.toml`
- Replace: `platform/windows/src/lib.rs`
- Create: `platform/windows/src/error.rs`
- Create: `platform/windows/src/input.rs`
- Create: `platform/windows/src/cursor.rs`
- Create: `platform/windows/tests/backend_contract.rs`

**Interfaces:**
- Produces: `BackendError`, `BackendErrorKind`, and `BackendResult<T>`.
- Produces: `InputInjector::inject(&mut self, &InputEvent) -> BackendResult<usize>`.
- Produces: `CursorSnapshot { visible, x, y, width, height, hotspot_x, hotspot_y, rgba }` with `validate()`.
- Produces internal adapters `NativeInputApi` and `NativeCursorApi`; their Windows implementations are private.

- [ ] **Step 1: Add dependencies needed by contract tests**

Add workspace dependencies on `nexus-capture`, `nexus-codec`, and `nexus-input`.
Add target-specific dependency:

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Foundation",
    "Graphics_Capture",
    "Win32_Foundation",
    "Win32_Graphics_Direct3D11",
    "Win32_Graphics_Dxgi",
    "Win32_Media_MediaFoundation",
    "Win32_System_Com",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 2: Write failing cursor contract tests**

In `platform/windows/tests/backend_contract.rs`, construct a 2x2 cursor with
16 RGBA bytes and assert `validate()` succeeds. Then construct hotspot `(2, 0)`
and a 15-byte payload and assert `HotspotOutOfBounds` and `CursorPayloadLength`.

- [ ] **Step 3: Run cursor tests and observe RED**

Run: `cargo test -p platform-windows --test backend_contract cursor`

Expected: compile failure because `CursorSnapshot` does not exist.

- [ ] **Step 4: Implement cursor validation**

Use checked `width * height * 4`, cap width and height at 256, cap the RGBA
payload at 262,144 bytes, and require hotspot coordinates to be strictly less
than their dimensions for visible non-empty cursors.

- [ ] **Step 5: Run cursor tests and observe GREEN**

Run: `cargo test -p platform-windows --test backend_contract cursor`

Expected: cursor contract tests pass.

- [ ] **Step 6: Write failing input translation tests**

Use a recording `NativeInputApi` adapter and assert:

```text
Key physical_code 0x1e Down -> one scan-code record with key-up false.
Text "é" -> one UTF-16 code-unit down/up pair.
MouseMove at monitor minimum/maximum -> normalized 0/65535 coordinates.
Invalid monitor bounds -> BackendErrorKind::InvalidInput.
```

- [ ] **Step 7: Run input tests and observe RED**

Run: `cargo test -p platform-windows --test backend_contract input`

Expected: compile failure because `InputInjector` and the adapter record types do not exist.

- [ ] **Step 8: Implement deterministic input translation**

Validate the `InputEvent` first, translate into private platform-neutral
`InputRecord` values, then pass them to `NativeInputApi`. Keep monitor-space
normalization in a pure function using `i64` checked subtraction and clamp only
after verifying positive width and height.

- [ ] **Step 9: Add the `cfg(windows)` `SendInput` adapter**

Map private records to `INPUT`, `KEYBDINPUT`, and `MOUSEINPUT`; require
`SendInput` to report the full submitted record count or return
`BackendErrorKind::NativeFailure`. No `unsafe` block may include translation or
validation logic.

- [ ] **Step 10: Verify and commit**

Run: `cargo test -p platform-windows --test backend_contract && cargo clippy -p platform-windows --all-targets -- -D warnings`

Expected: all contract tests and clippy pass on Linux.

```bash
git add platform/windows
git commit -m "feat(windows): add validated input and cursor contracts"
```

### Task 4: Add Capture Lifecycle and Fallback Selection

**Files:**
- Create: `platform/windows/src/capture.rs`
- Modify: `platform/windows/src/lib.rs`
- Create: `platform/windows/tests/capture_contract.rs`
- Create: `platform/windows/tests/windows_capture_smoke.rs`

**Interfaces:**
- Produces: `WindowsCaptureSource::start(config) -> BackendResult<Self>` and implementation of `CaptureSource<Error = BackendError>`.
- Produces: `CaptureConfig { preferred: CaptureApi, allow_dxgi_fallback: bool }`.
- Produces internal `CaptureFactory` and `CaptureSession` adapters for deterministic selection/lifecycle tests.

- [ ] **Step 1: Write failing API-selection tests**

Use a fake factory to assert WGC is attempted first; when it returns
`UnsupportedApi`, DXGI is attempted once; permission denial does not trigger
fallback; disabling fallback returns the WGC error unchanged.

- [ ] **Step 2: Run selection tests and observe RED**

Run: `cargo test -p platform-windows --test capture_contract selection`

Expected: compile failure because capture configuration and factory contracts are absent.

- [ ] **Step 3: Implement minimal selection and lifecycle state**

Implement explicit `Starting`, `Running(CaptureApi)`, `RecoverableLoss`, and
`Stopped` states. Only `UnsupportedApi` and initialization `DeviceLost` permit
the one-time WGC-to-DXGI fallback. `next_frame` validates every frame before
returning it.

- [ ] **Step 4: Run selection tests and observe GREEN**

Run: `cargo test -p platform-windows --test capture_contract selection`

Expected: selection tests pass.

- [ ] **Step 5: Write failing lifecycle tests**

Assert that a frame from the session is returned after validation, malformed
payload becomes `BackendErrorKind::InvalidFrame`, device loss is classified
recoverable, and calls after stop return `BackendErrorKind::Stopped`.

- [ ] **Step 6: Run lifecycle tests and observe RED**

Run: `cargo test -p platform-windows --test capture_contract lifecycle`

Expected: assertions fail until validation and state transitions are wired.

- [ ] **Step 7: Implement lifecycle behavior and native thread ownership**

Create a bounded `sync_channel(1)` command/result boundary. The constructor
spawns one named native thread; that thread initializes COM, owns the session,
and returns frames or structured errors. Shutdown sends one command and joins
the thread. Do not call `join` from a Tokio task in this slice.

- [ ] **Step 8: Add Windows WGC and DXGI session adapters**

Keep WGC frame-pool/session and DXGI duplication objects in private
`cfg(windows)` modules. Convert a acquired texture to the existing BGRA CPU
contract with validated row pitch; copy row-by-row when pitch exceeds
`width * 4`. Return `DeviceLost` for DXGI access loss and WGC closed-session
events.

- [ ] **Step 9: Add an ignored real-desktop smoke test**

`windows_capture_smoke.rs` is `#![cfg(windows)]`, marked `#[ignore = "requires an interactive Windows desktop"]`, requests one frame, validates it, and prints only dimensions/API/timing.

- [ ] **Step 10: Verify and commit**

Run: `cargo test -p platform-windows --test capture_contract && cargo clippy -p platform-windows --all-targets -- -D warnings`

Expected: deterministic tests pass; smoke test is compiled only on Windows.

```bash
git add platform/windows/src platform/windows/tests
git commit -m "feat(windows): add capture lifecycle and fallback"
```

### Task 5: Add Media Foundation H.264 Encoder Lifecycle

**Files:**
- Create: `platform/windows/src/codec.rs`
- Modify: `platform/windows/src/lib.rs`
- Create: `platform/windows/tests/codec_contract.rs`
- Create: `platform/windows/tests/windows_codec_smoke.rs`

**Interfaces:**
- Produces: `WindowsH264Encoder` implementing `VideoEncoder`.
- Produces internal `EncoderTransform` adapter with `configure`, `encode`, `drain`, and `shutdown` operations.
- Consumes: `EncoderConfig`, `CapturedFrame`, and `EncodedFrame` without changing their platform-neutral public shape.

- [ ] **Step 1: Write failing configuration tests**

Use a recording transform to assert invalid configuration never reaches the
adapter, valid H.264 configuration reaches it once, encoding before configure
returns `CodecError::NotConfigured`, and mismatched dimensions return
`CodecError::FrameDimensionsMismatch`.

- [ ] **Step 2: Run configuration tests and observe RED**

Run: `cargo test -p platform-windows --test codec_contract configuration`

Expected: compile failure because `WindowsH264Encoder` is absent.

- [ ] **Step 3: Implement encoder state and mapping**

Store the validated configuration, transform, and `force_next_keyframe` flag.
Map backend initialization/device errors to a new portable
`CodecError::BackendUnavailable` or `CodecError::BackendLost` without placing a
Windows error type in `nexus-codec`.

- [ ] **Step 4: Run configuration tests and observe GREEN**

Run: `cargo test -p platform-windows --test codec_contract configuration`

Expected: configuration tests pass.

- [ ] **Step 5: Write failing reconfiguration/keyframe tests**

Assert `request_keyframe` forces the next output flag, reconfiguration drains
the old transform, dimension changes force a keyframe, and unchanged dimensions
with bitrate-only change preserve normal cadence unless explicitly requested.

- [ ] **Step 6: Run keyframe tests and observe RED**

Run: `cargo test -p platform-windows --test codec_contract keyframe`

Expected: behavioral assertions fail before state transitions are implemented.

- [ ] **Step 7: Implement keyframe and reconfiguration behavior**

Validate before mutating state. Drain, configure, then commit the new config;
on configuration failure keep the encoder unavailable rather than continuing
with ambiguous old state.

- [ ] **Step 8: Add the Windows Media Foundation adapter**

On the encoder thread call `MFStartup`, select an H.264 hardware transform,
configure NV12 input/H.264 output media types, convert BGRA through a D3D11
video processor when available, and use a validated row-copy conversion only
as the Phase 1 fallback. `Drop` drains, releases transform objects, and calls
`MFShutdown` on the owning thread.

- [ ] **Step 9: Add an ignored Windows codec smoke test**

The `cfg(windows)` test encodes a 64x64 deterministic BGRA frame, requires a
non-empty H.264 access unit and keyframe on first output, and is ignored with
the reason `requires Media Foundation H.264 encoder availability`.

- [ ] **Step 10: Verify and commit**

Run: `cargo test -p nexus-codec -p platform-windows && cargo clippy -p nexus-codec -p platform-windows --all-targets -- -D warnings`

Expected: portable and deterministic backend tests pass.

```bash
git add crates/nexus-codec platform/windows
git commit -m "feat(windows): add Media Foundation H264 encoder boundary"
```

### Task 6: Record Verification and Resulting Status

**Files:**
- Modify: `docs/IMPLEMENTATION_STATUS.md`
- Modify: `docs/superpowers/plans/2026-08-27-phase-1-mvp-implementation.md`

**Interfaces:**
- Consumes: actual Linux, cross-target, and Windows smoke-test evidence.
- Produces: an honest distinction between implemented contracts, compiled native code, and real-hardware validation.

- [ ] **Step 1: Run full Linux verification**

Run:

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: every command exits zero.

- [ ] **Step 2: Check installed Windows target and cross-compile if available**

Run: `rustup target list --installed`

If `x86_64-pc-windows-msvc` is installed, run:

```bash
cargo check -p platform-windows --target x86_64-pc-windows-msvc --tests
```

Record the exact result; absence of the target is recorded as not verified.

- [ ] **Step 3: Run native Windows smoke tests on Windows hardware**

Run on an interactive Windows machine:

```powershell
cargo test -p platform-windows --test windows_capture_smoke -- --ignored --nocapture
cargo test -p platform-windows --test windows_codec_smoke -- --ignored --nocapture
```

Record API selected, frame dimensions, capture time, encoder backend, access
unit size, and encode time. Do not record pixels, keys, or input text.

- [ ] **Step 4: Update status only to the evidenced level**

Mark portable/native boundaries implemented when full workspace checks pass.
Mark native WGC/DXGI/Media Foundation backends verified only when the matching
Windows smoke tests pass. Keep Phase 1 `In progress` regardless of this task,
because client/service/full-relay work remains.

- [ ] **Step 5: Verify and commit status**

Run: `git diff --check && git diff -- docs/IMPLEMENTATION_STATUS.md docs/superpowers/plans/2026-08-27-phase-1-mvp-implementation.md`

Expected: claims match command evidence and Phase 1 remains in progress.

```bash
git add docs/IMPLEMENTATION_STATUS.md docs/superpowers/plans/2026-08-27-phase-1-mvp-implementation.md
git commit -m "docs: record Windows backend verification status"
```
