# Phase 1 Full Acceptance Guide

This guide is the runbook for moving Phase 1 from **In progress** to
**Done**. Linux tests and synthetic loopback are necessary checks, but they
are not evidence of the real Windows host/client/relay exit condition.

## 1. Required test environment

Use two Windows 10/11 machines (or a Windows VM plus a physical Windows
machine) and one reachable relay/control-plane host:

- Visual Studio Build Tools 2022, Desktop development with C++, MSVC v143,
  Windows 10/11 SDK, and Rust `x86_64-pc-windows-msvc`.
- A GPU/driver capable of the selected WGC/DXGI path and Media Foundation
  H.264 transform. Record GPU, driver, OS build, CPU, RAM, and display size.
- A network path that does not require inbound ports on the host.
- A clean database for enrollment and durable audit verification.

From the repository root, run on every Windows machine:

```powershell
rustup show
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Run the ignored native tests explicitly and retain their logs:

```powershell
cargo test -p platform-windows --test windows_capture_smoke -- --ignored --nocapture
cargo test -p platform-windows --test windows_codec_smoke -- --ignored --nocapture
cargo test -p platform-windows --test windows_input_cursor_smoke -- --ignored --nocapture
cargo test -p nexus-client --test windows_decoder_smoke -- --ignored --nocapture
cargo test -p nexus-client --test windows_client_smoke -- --ignored --nocapture
```

Do not mark a smoke test successful when it was skipped, ignored, or run on
Linux. If a native prerequisite is unavailable, record the exact reason.

## 2. Enrollment and authorization

1. Start `nexusd` with a fresh SQLite database and durable audit storage.
2. Start `nexus-relay` with its signing identity and reachable listener.
3. Enroll a fresh Windows host through the control plane. Verify that the
   credential is persisted locally and that no private key reaches `nexusd`.
4. Enroll a fresh Windows client and verify that it lists only authorized
   hosts.
5. Request a session and capture the signed capability and relay token
   metadata (never secrets or private keys).
6. Verify both agent and client reject expired, wrong-device, invalid,
   replayed, and permission-insufficient capabilities.
7. Verify `desktop.view` cannot emit input and `desktop.control` is exclusive
   per target device.

Evidence: enrollment IDs, session ID, capability expiry, permission set,
authorization decisions, and audit rows for login/enrollment/authorization.

## 3. Relay session and native media path

Run the complete path:

```text
Windows client → nexus-relay → Windows agent/service
                                  → desktop-host
```

Verify, in order:

1. The service launches the per-user desktop-host and authenticates its IPC
   peer identity/signature.
2. WGC is selected when available; DXGI fallback is used only for the
   documented recoverable initialization failures.
3. The desktop-host captures, encodes H.264, seals the access unit, and sends
   bounded datagrams through the relay.
4. The client authenticates, decodes, and presents the first frame through
   the Win32/D3D11 path.
5. Keyboard, mouse, wheel, text, and cursor updates reach the host and are
   audited. Malformed cursor/frame packets are rejected.
6. Kill the network path, relay endpoint, and desktop-host independently.
   Reconnect must preserve the session ID, respect the reconnect window, and
   never create a second control session.

Capture relay logs, endpoint state transitions, first-frame evidence, and
one complete audit trail from session start through session end.

## 4. Performance and reliability evidence

Run both LAN and relay profiles, including latency, loss, reordering, and
disconnect/reconnect. Record:

- First-frame latency and input-to-host latency (p50/p95).
- Capture, encode, transport, decode, and present timings.
- Frame rate, bitrate, frame age, stale-frame drops, and reconnect count.
- Agent idle RAM and one-hour soak-test memory delta.
- 1080p30 on minimum hardware and 1080p60 on recommended hardware.

Acceptance requires no unbounded media queue, deterministic agent restart/
crash respawn, and no memory growth beyond the agreed tolerance. Store raw
results under the project’s acceptance artifact location and link them from
`docs/IMPLEMENTATION_STATUS.md`.

## 5. Security and operational review

Before sign-off, complete `docs/security/phase1-threat-model.md` covering:

- Windows capture and desktop isolation.
- Agent/service IPC spoofing and process identity checks.
- Input injection and capability theft.
- Relay blindness to application payloads.
- Crash recovery, reconnect races, and audit durability.

Confirm that logs and telemetry contain correlation IDs and metrics only;
never frame plaintext, cryptographic keys, or sensitive text input.

## 6. Sign-off checklist

The Phase 1 owner should attach evidence for every item below:

- [ ] Fresh Windows host enrollment and persisted credential.
- [ ] Fresh client enrollment, authorized-host listing, and session request.
- [ ] Agent/client capability verification and permission enforcement.
- [ ] Relay-only encrypted session with relay unable to decrypt payloads.
- [ ] Native capture, H.264 encode, client decode, and first-frame present.
- [ ] Keyboard/mouse/text/cursor control and durable privileged-input audit.
- [ ] Reconnect, expiry, duplicate nonce, agent restart, and crash respawn.
- [ ] LAN/relay 1080p30 and 1080p60 measurements.
- [ ] One-hour soak and network-loss simulation results.
- [ ] Threat-model/security review and clean Windows build/test logs.

Only after every box has attached evidence should the status table change
Phase 1 from **In progress** to **Done**. If any item is unavailable, leave
the status unchanged and record the blocker and next test date.
