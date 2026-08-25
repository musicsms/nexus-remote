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
3. No system Protobuf installation is required. `nexus-protocol` uses the
   `protoc-bin-vendored` build dependency to select a pinned, platform-specific
   `protoc` binary and regenerate Rust control-message types from
   `proto/nexus.proto`. The binary is used only during compilation; it is not
   included in runtime applications. Set `PROTOC` explicitly only when you
   need to override the vendored compiler.
4. From the repo root, verify the workspace builds:
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
