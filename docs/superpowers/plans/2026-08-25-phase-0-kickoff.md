# Phase 0 Kickoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a working dev environment (macOS + Windows VM), CI, and Sprint 1's "wire skeleton" — a `nexus-protocol` crate with a versioned control-message type (Protobuf) and a hand-specified video packet header, plus a `nexus-transport` crate that proves synthetic frames and input messages can travel over a real QUIC loopback connection.

**Architecture:** `nexus-protocol` owns two independent wire formats per Spec Section 14/21/33: Protobuf (via `prost`, generated from `proto/nexus.proto`) for reliable-stream control/input messages, and a hand-rolled fixed-layout binary header for datagram-carried video packets. `nexus-transport` wraps `quinn` to set up a self-signed-cert QUIC endpoint pair and demonstrates both wire formats crossing the wire: `MouseMove` over a reliable stream, `VideoPacketHeader` + payload over an unreliable datagram — exactly Section 14's reliable-stream-for-control / datagram-for-video split.

**Tech Stack:** Rust stable, Tokio, Quinn 0.11 (QUIC), rustls 0.23, prost 0.13 (+ prost-build), rcgen 0.13 (self-signed test certs), thiserror, tracing.

## Global Constraints

- Rust stable toolchain (Spec Section 6; ADR-001).
- Tokio async runtime; no blocking I/O on Tokio runtime worker threads (Spec Section 57 rule 2; ADR-002).
- QUIC via Quinn, rustls for TLS 1.3 (Spec Section 6, 14; ADR-003).
- Control messages use Protobuf via Prost; video packets use a compact binary header, not Protobuf (Spec Section 21, 33).
- No unbounded channels in the media path (Spec Section 57 rule 1).
- No `unsafe` without a narrow module boundary and documented invariants (Spec Section 57 rule 3).
- Protocol parsers treat all remote input as hostile — validate, don't trust (Spec Section 57 rule 5).
- Every network message has an explicit maximum size limit (Spec Section 57 rule 6).
- `nexus-protocol` and `nexus-transport` must never import `windows-rs` or other OS-specific crates (CLAUDE.md §4; ADR-010) — everything in this plan is OS-independent by design.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` must pass (CLAUDE.md §9).
- Update `docs/IMPLEMENTATION_STATUS.md` in the same change whenever build status moves (CLAUDE.md §7).

---

### Task 1: Pin Rust toolchain, document setup, verify macOS build

**Files:**
- Create: `rust-toolchain.toml`
- Create: `docs/DEVELOPMENT.md`
- Test: none (environment verification via CLI commands, see Steps)

**Interfaces:**
- Produces: a pinned `channel = "stable"` toolchain every subsequent task and CI job relies on; `docs/DEVELOPMENT.md` is the reference both this task and Task 2 point to.

- [ ] **Step 1: Create `rust-toolchain.toml` at the workspace root**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 2: Create `docs/DEVELOPMENT.md`**

```markdown
# Development Environment Setup

Nexus needs two development environments: macOS (or Linux) for the
OS-independent crates, and a Windows environment for the Windows-specific
crates (`nexus-capture`, the Windows backend of `nexus-codec`,
`nexus-input`, `nexus-desktop-host`, `nexus-client`).

See `docs/adr/ADR-010-protocol-core-os-independent.md` for why the split
exists, and `docs/superpowers/specs/2026-08-25-phase-0-kickoff-design.md`
for why a local Windows VM (no GPU pass-through) is sufficient for capture
work but not for hardware-encoder validation.

## macOS / Linux (OS-independent crates)

Builds: `nexus-common`, `nexus-crypto`, `nexus-protocol`, `nexus-transport`,
`nexus-session`, `nexus-auth`, `nexus-policy`, `nexus-audit`,
`nexus-observability`, and (once scaffolded) `nexusd`, `nexus-relay`,
`nexus-cli`.

1. Install `rustup`:
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
2. Restart your shell, then verify: `rustc --version && cargo --version`.
   The workspace's `rust-toolchain.toml` pins `channel = "stable"` with
   `rustfmt` and `clippy` components — `rustup` installs these
   automatically the first time you run `cargo` inside the repo.
3. From the repo root, verify the workspace builds:
   ```sh
   cargo build --workspace
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   ```

## Windows VM (Windows-specific crates)

Builds: `nexus-capture`, `nexus-codec`, `nexus-input`, `nexus-desktop-host`,
`nexus-client`, `nexus-agent`.

1. Install Visual Studio Build Tools 2022, with the **"Desktop development
   with C++"** workload selected (this pulls in the MSVC v143 toolset and
   the Windows 10/11 SDK — required for linking against `windows-rs` and
   any native Windows API FFI).
2. Install `rustup` for Windows from https://rustup.rs (the `.exe`
   installer). Accept the default host triple
   (`x86_64-pc-windows-msvc`).
3. Clone/copy the repo into the VM, then from the repo root:
   ```powershell
   cargo build --workspace
   ```
   This should succeed today (all crates are stubs with no
   Windows-specific dependencies yet) — it's a smoke test that the MSVC
   linker and toolchain are wired up correctly before any Windows-specific
   code is written.

## Known limitation

The Windows VM described above (Parallels/UTM/VMware, no GPU
pass-through) can run Windows Graphics Capture / DXGI Desktop Duplication
PoC work, but **cannot** validate hardware H.264 encoding (NVENC/QSV/AMF)
— those need a real GPU exposed to the VM. Until GPU-passthrough access
exists, encoder PoC work uses the software-fallback encoder path
explicitly sanctioned by Spec Section 20.
```

- [ ] **Step 3: Verify macOS toolchain and workspace build**

Run:
```sh
rustc --version
cargo --version
cargo build --workspace
```
Expected: all three succeed; `cargo build --workspace` compiles all twelve
Phase-0 crates and six apps with no errors (they're stubs, this should be
fast).

- [ ] **Step 4: Verify formatting and lint gates pass**

Run:
```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: both exit 0. If `clippy` warns on generated/stub code, fix the
warning (don't silence the lint) before proceeding — these two commands
are the CI gate Task 3 automates, so they must be clean now.

- [ ] **Step 5: Commit**

```bash
git add rust-toolchain.toml docs/DEVELOPMENT.md
git commit -m "chore: pin Rust toolchain, document dev environment setup"
```

---

### Task 2: Verify Windows VM toolchain

**Files:** none created or modified — this task is pure environment
verification, executed inside the Windows VM, following
`docs/DEVELOPMENT.md`'s "Windows VM" section written in Task 1.

**Interfaces:**
- Consumes: `docs/DEVELOPMENT.md` (Task 1).
- Produces: a confirmed-working Windows build environment that Task 3's
  CI Windows job and all future Windows-specific crate work depend on.

- [ ] **Step 1: Follow `docs/DEVELOPMENT.md`'s "Windows VM" section inside the VM**

Install VS Build Tools 2022 (Desktop development with C++ workload) and
`rustup` for Windows as documented.

- [ ] **Step 2: Verify the toolchain**

Run inside the VM (PowerShell):
```powershell
rustc --version
cargo --version
```
Expected: both succeed, host triple reports `x86_64-pc-windows-msvc`.

- [ ] **Step 3: Verify the workspace builds inside the VM**

Copy or clone the repo into the VM (e.g. via a shared folder, or
`git clone` from the GitHub remote created earlier), then from the repo
root:
```powershell
cargo build --workspace
```
Expected: succeeds, same as the macOS build in Task 1 — confirms the MSVC
linker and toolchain are correctly wired up before any Windows-specific
(non-stub) code exists.

- [ ] **Step 4: No commit**

This task produces no repo changes (verification only). If any step
fails, fix `docs/DEVELOPMENT.md`'s instructions in a follow-up commit so
the next person doesn't hit the same problem — but that's Task 1's file
being amended, not a new task.

---

### Task 3: Set up GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`
- Modify: `docs/IMPLEMENTATION_STATUS.md` (Phase 0 row)

**Interfaces:**
- Consumes: `rust-toolchain.toml` (Task 1) — CI installs the same pinned
  toolchain.
- Produces: a CI pipeline every later task's commits run against
  automatically.

- [ ] **Step 1: Create `.github/workflows/ci.yml`**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  fmt-clippy-test:
    name: Format, Clippy, Test (Linux)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: Check formatting
        run: cargo fmt --all -- --check
      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Test
        run: cargo test --workspace

  windows-build:
    name: Build (Windows)
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Build
        run: cargo build --workspace
```

This covers Spec Section 55's "Rust formatting", "Clippy", "Unit tests",
and "Windows build" jobs — the four jobs the
`docs/superpowers/specs/2026-08-25-phase-0-kickoff-design.md` design doc
scoped as this kickoff's CI success criterion. The remaining Section 55
jobs (protocol compatibility tests, fuzz smoke tests, SQL migration tests,
signed-artifact dry run, client→relay→agent integration test) have no
corresponding code yet (no fuzz targets, no DB, no relay/agent talking to
each other) — adding them now would be dead CI configuration. They get
added in the plan that introduces the code they'd be testing.

- [ ] **Step 2: Update `docs/IMPLEMENTATION_STATUS.md`**

Open `docs/IMPLEMENTATION_STATUS.md` and find the Phase 0 row in the
"Phase progress" table (§1). Change its Status cell from:

```
**Not started** — workspace and crate/app skeletons exist (see below), but no CI, no protocol schema, no QUIC PoC, no capture, no encoder
```

to:

```
**In progress** — workspace and crate/app skeletons exist; CI running (fmt/clippy/test on Linux, build on Windows); no protocol schema, no QUIC PoC, no capture, no encoder yet
```

- [ ] **Step 3: Commit and verify on GitHub**

```bash
git add .github/workflows/ci.yml docs/IMPLEMENTATION_STATUS.md
git commit -m "ci: add GitHub Actions workflow (fmt, clippy, test, Windows build)"
git push
```

Then verify the workflow actually runs and passes:
```bash
gh run watch
```
Expected: both jobs (`fmt-clippy-test`, `windows-build`) complete with
success. If either fails, fix the underlying issue (not the workflow) and
push a follow-up commit — do not merge with a red CI run.

---

### Task 4: Protobuf codegen scaffolding + `SessionHello`/`MouseMove` messages

**Files:**
- Create: `proto/nexus.proto`
- Create: `crates/nexus-protocol/build.rs`
- Modify: `crates/nexus-protocol/Cargo.toml`
- Create: `crates/nexus-protocol/src/proto.rs`
- Modify: `crates/nexus-protocol/src/lib.rs`
- Test: `crates/nexus-protocol/tests/proto_roundtrip.rs`

**Interfaces:**
- Produces: `nexus_protocol::proto::SessionHello { protocol_version: u32, session_id: String, device_id: String, capability: Vec<u8>, ephemeral_public_key: Vec<u8> }` and `nexus_protocol::proto::MouseMove { x: i32, y: i32 }`, both implementing `prost::Message`. Task 7 depends on `MouseMove` by this exact path.

- [ ] **Step 1: Add workspace dependencies**

In the root `Cargo.toml`, under `[workspace.dependencies]`, add (alongside
the existing `prost = "0.13"` line):

```toml
prost-build = "0.13"
```

- [ ] **Step 2: Create `proto/nexus.proto`**

```protobuf
syntax = "proto3";
package nexus.protocol.v1;

// Spec Section 33.
message SessionHello {
  uint32 protocol_version = 1;
  string session_id = 2;
  string device_id = 3;
  bytes capability = 4;
  bytes ephemeral_public_key = 5;
}

// Spec Section 33.
message MouseMove {
  sint32 x = 1;
  sint32 y = 2;
}
```

- [ ] **Step 3: Add `build.rs` to `nexus-protocol`**

Create `crates/nexus-protocol/build.rs`:

```rust
fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=../../proto/nexus.proto");
    prost_build::compile_protos(&["../../proto/nexus.proto"], &["../../proto/"])
}
```

- [ ] **Step 4: Update `crates/nexus-protocol/Cargo.toml`**

Replace the file's `[dependencies]` section and add a
`[build-dependencies]` section:

```toml
[package]
name = "nexus-protocol"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
nexus-common = { path = "../nexus-common" }
thiserror.workspace = true
serde.workspace = true
tracing.workspace = true
prost.workspace = true

[build-dependencies]
prost-build.workspace = true
```

- [ ] **Step 5: Create `crates/nexus-protocol/src/proto.rs`**

```rust
//! Generated Protobuf control-message types (Spec Section 33).
//!
//! Source schema: `proto/nexus.proto`. Regenerated on every build by
//! `build.rs` via `prost-build` — do not hand-edit the generated code.
#![allow(clippy::all)]
include!(concat!(env!("OUT_DIR"), "/nexus.protocol.v1.rs"));
```

- [ ] **Step 6: Update `crates/nexus-protocol/src/lib.rs`**

```rust
//! nexus-protocol crate
//! Part of Nexus Remote Desktop Platform

pub mod proto;

pub use proto::{MouseMove, SessionHello};

pub fn init() {
    // Initializer stub for nexus-protocol
}
```

- [ ] **Step 7: Write the failing test**

Create `crates/nexus-protocol/tests/proto_roundtrip.rs`:

```rust
use nexus_protocol::MouseMove;
use prost::Message;

#[test]
fn mouse_move_round_trip() {
    let msg = MouseMove { x: -42, y: 1080 };

    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode should not fail");

    let decoded = MouseMove::decode(buf.as_slice()).expect("decode should not fail");

    assert_eq!(decoded, msg);
}

#[test]
fn session_hello_round_trip() {
    use nexus_protocol::SessionHello;

    let msg = SessionHello {
        protocol_version: 1,
        session_id: "ses_01".to_string(),
        device_id: "dev_01".to_string(),
        capability: vec![1, 2, 3, 4],
        ephemeral_public_key: vec![5, 6, 7, 8],
    };

    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("encode should not fail");

    let decoded = SessionHello::decode(buf.as_slice()).expect("decode should not fail");

    assert_eq!(decoded, msg);
}
```

- [ ] **Step 8: Run the tests to verify they fail (crate doesn't build yet without Steps 1-6)**

Run: `cargo test -p nexus-protocol --test proto_roundtrip`
Expected: FAIL — compile error, `nexus_protocol::MouseMove` doesn't exist,
until Steps 1-6 are actually in place. (If executing steps in order,
Steps 1-6 already exist by the time you reach Step 7 — this step confirms
the test file itself is correct by temporarily commenting out
`pub use proto::{MouseMove, SessionHello};` in `lib.rs`, running the test
to see it fail to compile, then restoring the line.)

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p nexus-protocol --test proto_roundtrip -v`
Expected: both `mouse_move_round_trip` and `session_hello_round_trip` PASS.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml proto/nexus.proto crates/nexus-protocol/build.rs \
  crates/nexus-protocol/Cargo.toml crates/nexus-protocol/src/proto.rs \
  crates/nexus-protocol/src/lib.rs crates/nexus-protocol/tests/proto_roundtrip.rs
git commit -m "feat(nexus-protocol): add Protobuf codegen, SessionHello and MouseMove messages"
```

---

### Task 5: Video packet header (Spec Section 21)

**Files:**
- Create: `crates/nexus-protocol/src/video_packet.rs`
- Modify: `crates/nexus-protocol/src/lib.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `nexus_protocol::VideoPacketHeader { version: u8, flags: u8, stream_id: u16, frame_id: u32, packet_id: u16, packet_count: u16, timestamp_us: u64, payload_len: u16 }`, `nexus_protocol::VideoPacketHeader::encode(&self, payload: &[u8], out: &mut Vec<u8>)`, `nexus_protocol::VideoPacketHeader::decode(buf: &[u8]) -> Result<(VideoPacketHeader, &[u8]), VideoPacketError>`, `nexus_protocol::video_packet::flags::{KEYFRAME, FRAME_START, FRAME_END, FEC, CONFIG}` (all `u8` constants), `nexus_protocol::video_packet::CURRENT_VERSION: u8`. Task 7 depends on `VideoPacketHeader`, `encode`, `decode`, and the `flags` constants by these exact names.

- [ ] **Step 1: Write the failing test (as part of the new module)**

Create `crates/nexus-protocol/src/video_packet.rs`:

```rust
//! Video packet header (Spec Section 21).
//!
//! Byte layout is fixed network order (big-endian) for every multi-byte
//! field — Section 21 requires "all fields and byte order must be
//! formally specified before compatibility is promised"; this module is
//! that specification for the Phase 0 PoC. It does not by itself satisfy
//! the full Definition of Done in Section 56 (fuzzing, security review,
//! backward-compat docs come later, once this header is used on a real
//! network path rather than a loopback PoC).

use thiserror::Error;

/// Total header length in bytes: 1+1+2+4+2+2+8+2.
pub const HEADER_LEN: usize = 22;
pub const CURRENT_VERSION: u8 = 1;

pub mod flags {
    pub const KEYFRAME: u8 = 0b0000_0001;
    pub const FRAME_START: u8 = 0b0000_0010;
    pub const FRAME_END: u8 = 0b0000_0100;
    pub const FEC: u8 = 0b0000_1000;
    pub const CONFIG: u8 = 0b0001_0000;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPacketHeader {
    pub version: u8,
    pub flags: u8,
    pub stream_id: u16,
    pub frame_id: u32,
    pub packet_id: u16,
    pub packet_count: u16,
    pub timestamp_us: u64,
    pub payload_len: u16,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoPacketError {
    #[error("buffer too short for header: need {HEADER_LEN} bytes, got {got}")]
    HeaderTooShort { got: usize },
    #[error("buffer too short for payload: header declares {declared} bytes, got {got}")]
    PayloadTooShort { declared: usize, got: usize },
}

impl VideoPacketHeader {
    /// Encodes the header followed by `payload` into `out`. Does not
    /// validate that `payload.len()` matches `self.payload_len` — callers
    /// are expected to set `payload_len` to `payload.len()` before calling.
    pub fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        out.reserve(HEADER_LEN + payload.len());
        out.push(self.version);
        out.push(self.flags);
        out.extend_from_slice(&self.stream_id.to_be_bytes());
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.packet_id.to_be_bytes());
        out.extend_from_slice(&self.packet_count.to_be_bytes());
        out.extend_from_slice(&self.timestamp_us.to_be_bytes());
        out.extend_from_slice(&self.payload_len.to_be_bytes());
        out.extend_from_slice(payload);
    }

    /// Decodes a header and its payload slice from `buf`. Rejects
    /// truncated input rather than panicking or reading out of bounds —
    /// Spec Section 57 rule 5 ("protocol parsers treat all remote input
    /// as hostile").
    pub fn decode(buf: &[u8]) -> Result<(VideoPacketHeader, &[u8]), VideoPacketError> {
        if buf.len() < HEADER_LEN {
            return Err(VideoPacketError::HeaderTooShort { got: buf.len() });
        }

        let version = buf[0];
        let flags = buf[1];
        let stream_id = u16::from_be_bytes([buf[2], buf[3]]);
        let frame_id = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let packet_id = u16::from_be_bytes([buf[8], buf[9]]);
        let packet_count = u16::from_be_bytes([buf[10], buf[11]]);
        let timestamp_us = u64::from_be_bytes([
            buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
        ]);
        let payload_len = u16::from_be_bytes([buf[20], buf[21]]);

        let payload_start = HEADER_LEN;
        let payload_end = payload_start + payload_len as usize;
        if buf.len() < payload_end {
            return Err(VideoPacketError::PayloadTooShort {
                declared: payload_len as usize,
                got: buf.len() - payload_start,
            });
        }

        Ok((
            VideoPacketHeader {
                version,
                flags,
                stream_id,
                frame_id,
                packet_id,
                packet_count,
                timestamp_us,
                payload_len,
            },
            &buf[payload_start..payload_end],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encode_decode() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: flags::KEYFRAME | flags::FRAME_START | flags::FRAME_END,
            stream_id: 1,
            frame_id: 42,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 1_234_567,
            payload_len: 4,
        };
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];

        let mut buf = Vec::new();
        header.encode(&payload, &mut buf);

        let (decoded, decoded_payload) = VideoPacketHeader::decode(&buf).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded_payload, &payload);
    }

    #[test]
    fn decode_rejects_truncated_header() {
        let buf = [0u8; 10];
        let err = VideoPacketHeader::decode(&buf).unwrap_err();
        assert_eq!(err, VideoPacketError::HeaderTooShort { got: 10 });
    }

    #[test]
    fn decode_rejects_truncated_payload() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 0,
            frame_id: 0,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            payload_len: 100,
        };
        let mut buf = Vec::new();
        header.encode(&[], &mut buf); // declares 100 bytes but encodes 0

        let err = VideoPacketHeader::decode(&buf).unwrap_err();
        assert_eq!(
            err,
            VideoPacketError::PayloadTooShort {
                declared: 100,
                got: 0
            }
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they don't run yet**

Run: `cargo test -p nexus-protocol video_packet`
Expected: `running 0 tests` — the filter matches nothing because
`video_packet.rs` isn't declared as a module in `lib.rs` yet, so Rust
never compiles it. (This exits 0, not a failure — a test-name filter
matching zero tests is not itself an error. The real "red" checkpoint is
that 0 of the 3 tests you just wrote ran, not a nonzero exit code.)

- [ ] **Step 3: Wire the module into `lib.rs`**

Update `crates/nexus-protocol/src/lib.rs`:

```rust
//! nexus-protocol crate
//! Part of Nexus Remote Desktop Platform

pub mod proto;
pub mod video_packet;

pub use proto::{MouseMove, SessionHello};
pub use video_packet::{VideoPacketError, VideoPacketHeader};

pub fn init() {
    // Initializer stub for nexus-protocol
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nexus-protocol video_packet -v`
Expected: all three tests (`round_trip_encode_decode`,
`decode_rejects_truncated_header`, `decode_rejects_truncated_payload`)
PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nexus-protocol/src/video_packet.rs crates/nexus-protocol/src/lib.rs
git commit -m "feat(nexus-protocol): add Section 21 video packet header encode/decode"
```

---

### Task 6: QUIC loopback connection helpers in `nexus-transport`

**Files:**
- Modify: `crates/nexus-transport/Cargo.toml`
- Create: `crates/nexus-transport/src/quic.rs`
- Modify: `crates/nexus-transport/src/lib.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `nexus_transport::quic::ServerEndpoint { endpoint: quinn::Endpoint, cert_der: rustls::pki_types::CertificateDer<'static> }`, `nexus_transport::quic::make_server_endpoint(bind_addr: std::net::SocketAddr) -> anyhow::Result<ServerEndpoint>`, `nexus_transport::quic::make_client_endpoint(bind_addr: std::net::SocketAddr, server_cert: &rustls::pki_types::CertificateDer<'static>) -> anyhow::Result<quinn::Endpoint>`. Task 7 depends on all three by these exact names.

- [ ] **Step 1: Add dependencies to workspace and crate**

In the root `Cargo.toml`, under `[workspace.dependencies]`, add:

```toml
rcgen = "0.13"
```

Update `crates/nexus-transport/Cargo.toml`:

```toml
[package]
name = "nexus-transport"
version.workspace = true
edition.workspace = true
authors.workspace = true
license.workspace = true

[dependencies]
nexus-common = { path = "../nexus-common" }
thiserror.workspace = true
serde.workspace = true
tracing.workspace = true
tokio.workspace = true
quinn.workspace = true
rustls.workspace = true
anyhow.workspace = true
rcgen.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

- [ ] **Step 2: Write the failing test**

Create `crates/nexus-transport/src/quic.rs`:

```rust
//! QUIC endpoint setup for local loopback (Spec Section 6, 14; ADR-003).
//!
//! Uses a self-signed certificate (`rcgen`) so client and server can
//! establish a TLS 1.3-secured QUIC connection without a real CA. This
//! is a Phase 0 PoC helper for same-process loopback testing — it is NOT
//! the production certificate model (real deployments use control-plane-
//! issued or CA-issued certs).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

pub struct ServerEndpoint {
    pub endpoint: Endpoint,
    pub cert_der: CertificateDer<'static>,
}

fn transport_config() -> TransportConfig {
    let mut config = TransportConfig::default();
    config.max_idle_timeout(Some(Duration::from_secs(30).try_into().expect("fits in VarInt")));
    // Explicit, non-default receive buffer for video-style datagram
    // traffic (Spec Section 57 rule 1: no unbounded channels/queues).
    config.datagram_receive_buffer_size(Some(64 * 1024));
    config
}

/// Binds a QUIC server endpoint on `bind_addr` with a freshly generated
/// self-signed certificate for `localhost`.
pub fn make_server_endpoint(bind_addr: SocketAddr) -> anyhow::Result<ServerEndpoint> {
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der = CertificateDer::from(generated.cert);
    let key_der = PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der());

    let mut server_config =
        ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())?;
    server_config.transport_config(Arc::new(transport_config()));

    let endpoint = Endpoint::server(server_config, bind_addr)?;

    Ok(ServerEndpoint { endpoint, cert_der })
}

/// Binds a QUIC client endpoint on `bind_addr` that trusts exactly
/// `server_cert` (the certificate returned by `make_server_endpoint`).
pub fn make_client_endpoint(
    bind_addr: SocketAddr,
    server_cert: &CertificateDer<'static>,
) -> anyhow::Result<Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(server_cert.clone())?;

    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?;
    let mut client_config = ClientConfig::new(Arc::new(quic_client_config));
    client_config.transport_config(Arc::new(transport_config()));

    let mut endpoint = Endpoint::client(bind_addr)?;
    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn client_connects_to_server() {
        let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server.endpoint.local_addr().unwrap();
        let cert_der = server.cert_der.clone();

        let server_task = tokio::spawn(async move {
            let incoming = server.endpoint.accept().await.expect("no incoming connection");
            let connection = incoming.await.expect("handshake failed");
            connection.remote_address()
        });

        let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
        let connection = client
            .connect(server_addr, "localhost")
            .expect("connect() setup failed")
            .await
            .expect("handshake failed");

        assert_eq!(connection.remote_address().port(), server_addr.port());

        let server_saw = server_task.await.unwrap();
        assert_eq!(server_saw.ip(), connection.local_ip().unwrap_or(server_saw.ip()));
    }
}
```

- [ ] **Step 3: Run test to verify it doesn't run yet**

Run: `cargo test -p nexus-transport quic`
Expected: `running 0 tests` — `quic.rs` isn't declared as a module in
`lib.rs` yet, so it isn't compiled and the filter matches nothing. (Exits
0; the checkpoint is that `client_connects_to_server` did not run, not a
nonzero exit code — see the same note in Task 5, Step 2.)

- [ ] **Step 4: Wire the module into `lib.rs`**

Update `crates/nexus-transport/src/lib.rs`:

```rust
//! nexus-transport crate
//! Part of Nexus Remote Desktop Platform

pub mod quic;

pub fn init() {
    // Initializer stub for nexus-transport
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p nexus-transport quic -v`
Expected: `client_connects_to_server` PASSes.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/nexus-transport/Cargo.toml crates/nexus-transport/src/quic.rs crates/nexus-transport/src/lib.rs
git commit -m "feat(nexus-transport): add self-signed-cert QUIC loopback endpoint helpers"
```

---

### Task 7: Sprint 1 demo — synthetic frame + input message over QUIC loopback

**Files:**
- Create: `crates/nexus-transport/tests/loopback_demo.rs`
- Modify: `docs/IMPLEMENTATION_STATUS.md`

**Interfaces:**
- Consumes: `nexus_protocol::{MouseMove, VideoPacketHeader, video_packet::flags, video_packet::CURRENT_VERSION}` (Tasks 4-5); `nexus_transport::quic::{make_server_endpoint, make_client_endpoint}` (Task 6).
- Produces: nothing further downstream — this is Sprint 1's exit demo (Spec Section 52: "transmit synthetic frames and input messages over QUIC locally").

- [ ] **Step 1: Add `nexus-protocol` and `prost` as dev-dependencies of `nexus-transport`**

Update `crates/nexus-transport/Cargo.toml`, adding to `[dev-dependencies]`:

```toml
nexus-protocol = { path = "../nexus-protocol" }
prost.workspace = true
```

(Keep the existing `tokio = { workspace = true, features = [...] }` dev-dependency line from Task 6.)

- [ ] **Step 2: Write the failing test**

Create `crates/nexus-transport/tests/loopback_demo.rs`:

```rust
//! Sprint 1 demo (Spec Section 52): transmit a synthetic input message
//! over a reliable QUIC stream, and a synthetic video packet over an
//! unreliable QUIC datagram, between a loopback client and server —
//! proving Section 14's reliable-stream-for-control /
//! datagram-for-video split actually works end to end.

use nexus_protocol::{video_packet, MouseMove, VideoPacketHeader};
use nexus_transport::quic::{make_client_endpoint, make_server_endpoint};
use prost::Message;

#[tokio::test]
async fn synthetic_frame_and_input_travel_over_quic_loopback() {
    tracing_subscriber::fmt::try_init().ok();

    let server = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.endpoint.local_addr().unwrap();
    let cert_der = server.cert_der.clone();

    let server_task = tokio::spawn(async move {
        let incoming = server.endpoint.accept().await.expect("no incoming connection");
        let connection = incoming.await.expect("handshake failed");
        tracing::info!(remote = %connection.remote_address(), "server: connection established");

        // Reliable stream: input message (Section 14).
        let (_send, mut recv) = connection.accept_bi().await.expect("no incoming stream");
        let stream_bytes = recv
            .read_to_end(1024)
            .await
            .expect("failed to read input stream");
        let received_mouse_move =
            MouseMove::decode(stream_bytes.as_slice()).expect("failed to decode MouseMove");
        tracing::info!(?received_mouse_move, "server: decoded input message");

        // Unreliable datagram: video packet (Section 14).
        let datagram = connection
            .read_datagram()
            .await
            .expect("failed to read video datagram");
        let (received_header, received_payload) =
            VideoPacketHeader::decode(&datagram).expect("failed to decode video packet header");
        tracing::info!(?received_header, payload_len = received_payload.len(), "server: decoded video packet");

        (received_mouse_move, received_header, received_payload.to_vec())
    });

    let client = make_client_endpoint("127.0.0.1:0".parse().unwrap(), &cert_der).unwrap();
    let connection = client
        .connect(server_addr, "localhost")
        .expect("connect() setup failed")
        .await
        .expect("handshake failed");
    tracing::info!(remote = %connection.remote_address(), "client: connection established");

    // Send the input message over a reliable bidirectional stream.
    let sent_mouse_move = MouseMove { x: 640, y: 360 };
    let mut mouse_move_buf = Vec::new();
    sent_mouse_move.encode(&mut mouse_move_buf).unwrap();

    let (mut send, _recv) = connection.open_bi().await.expect("failed to open stream");
    send.write_all(&mouse_move_buf).await.expect("failed to write input stream");
    send.finish().expect("failed to finish stream");

    // Send the synthetic video packet over an unreliable datagram.
    let sent_header = VideoPacketHeader {
        version: video_packet::CURRENT_VERSION,
        flags: video_packet::flags::KEYFRAME | video_packet::flags::FRAME_START | video_packet::flags::FRAME_END,
        stream_id: 0,
        frame_id: 1,
        packet_id: 0,
        packet_count: 1,
        timestamp_us: 42,
        payload_len: 4,
    };
    let sent_payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
    let mut datagram_buf = Vec::new();
    sent_header.encode(&sent_payload, &mut datagram_buf);
    connection
        .send_datagram(datagram_buf.into())
        .expect("failed to send video datagram");

    let (received_mouse_move, received_header, received_payload) =
        server_task.await.expect("server task panicked");

    assert_eq!(received_mouse_move, sent_mouse_move);
    assert_eq!(received_header, sent_header);
    assert_eq!(received_payload, sent_payload);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p nexus-transport --test loopback_demo`
Expected: FAIL — compile error until Step 1's dev-dependency addition
lands (`nexus_protocol` not resolvable from the test target).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nexus-transport --test loopback_demo -v -- --nocapture`
Expected: `synthetic_frame_and_input_travel_over_quic_loopback` PASSes;
`--nocapture` shows the `tracing::info!` lines confirming both the
reliable-stream input message and the datagram video packet were
received and decoded correctly.

- [ ] **Step 5: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test across `nexus-protocol` (Tasks 4-5) and
`nexus-transport` (Tasks 6-7) passes — this is what CI (Task 3) will run
on push.

- [ ] **Step 6: Update `docs/IMPLEMENTATION_STATUS.md`**

Two edits:

First, in the "Phase progress" table (§1), change the Phase 0 row's Status
cell (already edited once in Task 3) to:

```
**In progress** — workspace/CI done; nexus-protocol has SessionHello/MouseMove (Protobuf) + Section 21 video packet header; nexus-transport proves both over a real QUIC loopback connection (Sprint 1 demo, Section 52). No Windows capture or hardware encoder PoC yet.
```

Second, in the "`crates/` — Phase 0 (initial) members" table (§2), update
two rows:

```
| `nexus-protocol` | Versioned wire/control schema | In progress — Protobuf codegen (`SessionHello`, `MouseMove`) via prost-build from `proto/nexus.proto`; hand-rolled `VideoPacketHeader` encode/decode (Section 21) with malformed-input tests |
```

```
| `nexus-transport` | QUIC connections, streams, datagrams, metrics | In progress — self-signed-cert QUIC loopback endpoint helpers (`make_server_endpoint`/`make_client_endpoint`); Sprint 1 demo proves reliable-stream input + unreliable-datagram video both work end to end. No metrics, no relay integration yet |
```

Also update the `proto/` row in "Everything else in the target tree"
(§2) from "Not started" to:

```
**Scaffolded** — `proto/nexus.proto` with `SessionHello`, `MouseMove`; generated via `prost-build` in `nexus-protocol`'s `build.rs`
```

- [ ] **Step 7: Commit**

```bash
git add crates/nexus-transport/Cargo.toml crates/nexus-transport/tests/loopback_demo.rs docs/IMPLEMENTATION_STATUS.md
git commit -m "feat: Sprint 1 demo — synthetic frame + input message over QUIC loopback"
git push
```

- [ ] **Step 8: Verify CI passes on the pushed commit**

Run: `gh run watch`
Expected: both CI jobs (`fmt-clippy-test`, `windows-build`) succeed on
this commit. This is Phase 0 Sprint 1's exit demonstration, now also
continuously verified by CI going forward.

---

## Post-plan state

After Task 7: macOS and Windows VM environments are both verified
working; GitHub Actions CI runs on every push; `nexus-protocol` has a
real (if minimal) Protobuf control-message schema and a formally
byte-specified video packet header, both with passing tests including
malformed-input rejection; `nexus-transport` proves a real QUIC
connection can carry both wire formats over loopback, matching Section
14's reliable-stream/datagram split. `docs/IMPLEMENTATION_STATUS.md`
reflects all of this accurately.

**Not covered by this plan** (per the design doc's scope boundary — these
are separate future plans): Windows capture PoC in the VM, H.264 encoder
PoC (software-fallback), client-side hardware decode/render, and every
later Sprint 2-6 item.
