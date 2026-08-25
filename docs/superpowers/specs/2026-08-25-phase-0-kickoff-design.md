# Phase 0 Kickoff: Environment & Sequencing

Status: Approved by user (Minh), 2026-08-25.

## Context

The architecture layer is complete: the spec is internally consistent, all
24 tracked ADRs are frozen (`docs/adr/`), and `docs/IMPLEMENTATION_STATUS.md`
accurately tracks build status. Phase 0 (Section 48 of the spec) has not
started. Before writing any code, we need a concrete plan for *how* to
start, given the actual development environment available right now — not
the idealized environment the spec's Section 59 build order assumes.

**Constraints discovered before this decision:**

- No Rust toolchain installed anywhere yet (`rustc`/`cargo`/`rustup` absent
  on the primary dev machine).
- No CI configured (`.github/` doesn't exist). CI is one of Phase 0's seven
  required deliverables (Section 48).
- Primary dev machine is macOS (Apple Silicon). MVP target platform is
  Windows host + Windows client (ADR-008, unchanged — see "Out of scope"
  below).
- A Windows environment is available, but only as a **local VM** (Parallels/
  UTM/VMware on the Mac), with no GPU pass-through. This means:
  - Windows Graphics Capture / DXGI Desktop Duplication (Section 19) work
    fine in the VM — they capture the desktop compositor's output, not raw
    GPU hardware.
  - Hardware H.264 encode (NVENC/QSV/AMF, Section 20) does **not** work in
    this VM — none of the three vendor encode engines are exposed without
    GPU pass-through.

Section 59's "Recommended Immediate Implementation Order" (Windows capture
→ H.264 hardware encode → QUIC → client decode → ...) assumes hardware
encode is reachable from day one. It isn't, in the current environment.
This design resolves how to sequence Phase 0 work given that gap, without
blocking on acquiring GPU-passthrough infrastructure first.

## Out of scope

This is a **sequencing and environment-setup decision only**. It does not
change:
- ADR-008 (Windows-first host/client MVP) — Windows remains the MVP target.
- Section 2's Non-Goals (no macOS/Linux host support for MVP).
- Any other frozen ADR or spec content.

A Linux-host/macOS-client direction was considered and explicitly declined
by the user during this discussion — the plan below stays inside the
existing Windows-first architecture.

## Decision

### 1. Two development environments, not one

- **macOS (primary machine):** install `rustup` + stable toolchain. Used
  for every crate that Section 3.6/ADR-010 already require to be
  OS-independent: `nexus-common`, `nexus-protocol`, `nexus-transport`,
  `nexus-session`, `nexus-crypto`, `nexus-observability`. Also hosts CI
  workflow authoring and general workspace tooling (`cargo fmt`, `clippy`,
  `cargo test --workspace` for the OS-independent crates).
- **Windows VM:** install `rustup` (MSVC target) + Visual Studio Build
  Tools (required for `windows-rs` / WGC / DXGI FFI linking). Used for
  `nexus-capture`, the Windows backend of `nexus-codec`, `nexus-input`,
  `nexus-desktop-host`, and the Windows build of `nexus-client`.

Cross-compiling the Windows-API-heavy crates from macOS was considered and
rejected: it requires vendoring Windows SDK headers and import libraries
for a linking setup that gains nothing over developing directly in the VM,
where the APIs are natively available and testable.

### 2. CI provider: GitHub Actions

Chosen because it has both Linux and native Windows (`windows-latest`)
runners built in, directly satisfying Section 55's "Windows build" CI job
requirement without standing up a self-hosted runner. Requires the repo to
have a GitHub remote — **confirm before Sprint 1 lands** whether this repo
is already pushed to GitHub or needs to be.

### 3. Work sequencing: adapted from Section 59, not identical to it

1. **Sprint 1 — Wire skeleton** (Section 52), on macOS, first: Cargo
   workspace CI, `nexus-protocol` schema skeleton, `nexus-transport` QUIC
   PoC via Quinn, `tracing` wired up, a loopback sender/receiver
   demonstrating synthetic frames and input messages over QUIC locally.
   Entirely OS-independent — no VM dependency, fastest path to a working
   CI pipeline and a first demonstrable milestone.
2. **Windows capture PoC**, in the VM: Windows Graphics Capture primary,
   DXGI Desktop Duplication fallback (Section 19). Feasible now, no GPU
   pass-through needed.
3. **H.264 encode PoC, using the software encoder fallback** explicitly
   sanctioned by Section 20 ("Software encoder only as fallback and for
   CI/testing"). This validates the encode pipeline's architecture
   (`VideoEncoder` trait, keyframe requests, reconfiguration) end-to-end,
   but does **not** validate real hardware-encoder performance
   (NVENC/QSV/AMF) or the 1080p60 / latency-budget targets in Sections 42–43
   — those require GPU-passthrough access this environment doesn't have
   yet.

This still attacks the media pipeline's architectural risk early (Section
59's underlying rationale), just with the hardware-performance validation
step explicitly deferred rather than silently skipped.

### 4. Tracking the deferred item

Once Sprint work begins, `docs/IMPLEMENTATION_STATUS.md` must record the
software-fallback-only status of the encoder PoC as an explicit, named
limitation (not leave it implicit) — e.g. under Phase 0's row: "H.264
encode PoC — software fallback only; hardware backend (NVENC/QSV/AMF)
validation blocked on GPU-passthrough environment access." This keeps the
status file honest about what Phase 0's exit condition ("capture a Windows
desktop and stream frames between two local processes") has actually
demonstrated versus what remains unvalidated.

## Success criteria for this kickoff

- `rustup`/`cargo` functional on macOS; `cargo build --workspace` succeeds
  for the twelve Phase-0 crates.
- `rustup`/`cargo` + VS Build Tools functional in the Windows VM.
- GitHub Actions CI running at least: Rust formatting, clippy, unit tests
  (Linux runner) and a Windows build job (`windows-latest`), per Section 55.
- Sprint 1's demo achieved: synthetic frames and input messages transmitted
  over QUIC in a loopback test.

Detailed step-by-step tasks for reaching these are the subject of the
implementation plan (next step: `writing-plans`), not this design doc.
