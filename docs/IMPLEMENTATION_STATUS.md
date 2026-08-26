# Nexus Implementation Status

This file tracks how far the actual repository has progressed against the
target architecture defined in `docs/Nexus Remote Desktop Platform - Spec.md`
(Section 5 - Repository Layout, Section 48 - Development Phases, Section 51 -
ADR list, Appendix A - Crate Responsibilities).

The spec is the stable target architecture and should change rarely. This
file changes often — update it whenever a crate/app moves from one status to
the next, a directory gets scaffolded, or a phase exit condition is met.
Keeping the two documents separate means implementation progress never
churns the architecture doc, and the architecture doc never goes stale
relative to what's actually built.

Status legend: **Not started** (path/doc does not exist yet) · **Scaffolded**
(directory/crate exists with stub code only, no real logic) · **In
progress** (real implementation underway, not feature-complete) · **Done**
(meets the relevant Definition of Done / exit condition in the spec).

Last audited: 2026-08-26.

---

## 1. Phase progress (Spec Section 48)

| Phase | Scope | Exit condition | Status |
|---|---|---|---|
| Phase 0 — Foundation | Workspace, CI, protocol crate, QUIC PoC, Windows capture PoC, H.264 encoder PoC | Capture a Windows desktop and stream frames between two local processes | **Done for OS-independent foundation** — protocol, capture/queue, software codec, crypto, fragmentation/reassembly, and QUIC loopback are verified end-to-end in `phase0_e2e_pipeline`; live Windows Graphics Capture/DXGI and hardware H.264 remain Phase 1 platform work |
| Phase 1 — MVP v0.1 | Windows host/client, minimal nexusd, enrollment, relay-only QUIC, H.264 1080p60, input, cursor, reconnect, telemetry overlay | User can enroll a host + client and control it over the Internet through a relay | Not started |
| Phase 2 — v0.2 Connectivity | Candidate discovery, hole punching, P2P QUIC, adaptive bitrate, clipboard text | P2P succeeds on common NATs, falls back to relay | Not started |
| Phase 3 — v0.3 Productization | File transfer, audio, recording, RBAC, audit UI/API, signed updates | — | Not started |
| Phase 4 — v0.5 Enterprise | OIDC, SAML, WebAuthn, access requests, device labels, ABAC | — | Not started |
| Phase 5 — v1.0 | macOS/Linux hosts, HEVC/AV1, enterprise SSO, JIT, recording, device trust | — | Not started |

---

## 2. Repository layout status (Spec Section 5)

Mirrors the target tree 1:1. Anything not listed here does not exist in the
repo yet.

### `crates/` — Phase 0 (initial) members

| Crate | Target responsibility (Appendix A) | Status |
|---|---|---|
| `nexus-common` | IDs, shared errors, time, configuration primitives | **In progress** — strongly typed entity IDs (DeviceId, UserId, NodeId, TenantId, SessionId, ClientId), UnixTimestamp with arithmetic and Serde, Clock/MockClock traits, and common error taxonomy; configuration primitives remain next |
| `nexus-crypto` | Device keys, capability verification, session key derivation | **In progress** — Ed25519 device keypair abstraction, signed-payload envelope, X25519/HKDF-SHA256 session root derivation, ChaCha20-Poly1305 AEAD, fail-closed channel nonce sequencing, and canonical encoded-frame AAD helpers; transport packetizer integration and OS-backed persistence/rotation remain next |
| `nexus-protocol` | Versioned wire/control schema | **In progress** — Protobuf codegen for session and MVP input messages via `proto/nexus.proto`; hand-rolled `VideoPacketHeader` encode/decode (Section 21) with malformed-input tests |
| `nexus-transport` | QUIC connections, streams, datagrams, metrics | **In progress** — self-signed-cert QUIC loopback endpoint helpers, encoded-frame AEAD seal/open integration, bounded datagram frame packetizer and drop-stale reassembler (`VideoFrameReassembler`), and Sprint 1 loopback demo. Metrics and relay integration remain next |
| `nexus-session` | Session state machine, reconnect semantics | **In progress** — explicit lifecycle transitions, stable reconnect-window policy, and deterministic established-session max-duration expiry checks |
| `nexus-auth` | User/device authentication logic | **In progress** — bounded TTL nonce replay cache for signed capability verification; user/device enrollment remains next |
| `nexus-policy` | RBAC/ABAC evaluation | **In progress** — 11 first-class actions (Action/ActionSet), role & device label matching models, PolicyEngine evaluating RBAC/ABAC with ADR-015 concurrent control exclusivity, and ADR-017 dynamic policy narrowing validator; database-backed role persistence remains next |
| `nexus-audit` | Audit event model and sinks | **In progress** — 17 standard audit event types, AuditEvent model with canonical serialization, tamper-evident cryptographic hash chain (BLAKE3) with tamper detection verification, and async AuditSink / MemoryAuditSink / BroadcastAuditSink abstractions; database sink & export adapters remain next |
| `nexus-codec` | Encoder/decoder abstractions | **In progress** — OS-independent `VideoEncoder`, H.264 config, encoded-frame metadata, keyframe/reconfigure contract, and `SoftwareFallbackEncoder` test/fallback encoder; hardware-accelerated OS backends remain next |
| `nexus-capture` | Platform-neutral capture traits | **In progress** — `CaptureSource`/`CapturedFrame` contract, ADR-022 depth-1 latest-frame queue with replacement/drop accounting, and `SyntheticCaptureSource` test capture source; Windows Graphics Capture backend remains next |
| `nexus-input` | Semantic input model | **In progress** — OS-independent keyboard, text, mouse and wheel events with bounded text validation; native Windows injection remains next |
| `nexus-observability` | tracing, metrics, session quality telemetry | Scaffolded — stub only |

### `crates/` — Phase 3 members (not yet due)

| Crate | Target responsibility | Status |
|---|---|---|
| `nexus-audio` | Audio model and Opus pipeline | Not started — correctly absent; add when Phase 3 begins (Section 28) |
| `nexus-file-transfer` | Chunking, resume, integrity | Not started — correctly absent; add when Phase 3 begins (Section 27) |

### `apps/`

| App | Target responsibility | Status |
|---|---|---|
| `nexusd` | Control plane: auth, devices, policy, sessions, signaling, audit | Scaffolded — binary boots, initializes `tracing_subscriber`, logs a startup line. No HTTP/gRPC server, no DB, no auth, no session broker |
| `nexus-relay` | Stateless encrypted packet relay | Scaffolded — stub binary only |
| `nexus-agent` | Host service: identity, presence, session lifecycle, privilege boundary | Scaffolded — stub binary only |
| `nexus-desktop-host` | User-session process: capture, encode, input, clipboard, audio | Scaffolded — stub binary only |
| `nexus-client` | Native viewer/controller | Scaffolded — stub binary only |
| `nexus-cli` | Administrative/debugging CLI | Scaffolded — stub binary only |

### Everything else in the target tree

| Path | Purpose (Spec) | Status |
|---|---|---|
| `platform/windows/` | Windows-specific OS/codec bindings | Not started |
| `platform/macos/` | macOS-specific bindings (Phase 5) | Not started |
| `platform/linux/` | Linux-specific bindings (Phase 5) | Not started |
| `proto/` | Protobuf schemas (Section 33) | Scaffolded — `proto/nexus.proto` defines the Phase 0 session, input, monitor, cursor, and capability messages (package `nexus.protocol.v1`), compiled into `nexus-protocol` by `build.rs` via `prost-build` |
| `migrations/` | SQL migrations (SQLite-first, Section 34) | Not started |
| `deployment/` | Deployment manifests (Section 53) | Not started |
| `test/integration/` | Client → relay → agent integration tests | Not started |
| `test/network-sim/` | Network simulation profiles (Section 47) | Not started |
| `test/performance/` | Performance regression tests | Not started |
| `docs/adr/` | Frozen ADRs (Section 51) | **Done** — ADR-001 through ADR-025 are recorded; ADR-025 freezes encoded-frame AEAD granularity and nonce/AAD boundaries. |
| `docs/protocol/` | Protocol documentation | In progress — 5 design notes added: `session-establishment-signaling.md`, `session-authorization-model.md`, `connectivity-nat-traversal.md`, `windows-agent-privilege-boundary.md`, `video-media-pipeline.md` |
| `docs/security/` | Threat model, security docs (Section 44) | Not started |

---

## 3. ADR status (Spec Section 51, plus ADRs discovered during design review)

All 25 of the tracked ADRs have now been written as documents in
`docs/adr/`. ADR-001 through ADR-013 were the 13 foundational decisions
Section 51 called for "before heavy implementation" — they were already
implemented in practice (e.g. the crates in `Cargo.toml` already encoded
ADR-001/002/003/008/010/012/013) and stated in the spec's prose, but were
not yet frozen as standalone records until this pass; they are backfill
ADRs formalizing existing decisions, not new architectural choices.
ADR-014 through ADR-024 arose from active system-design review ahead of
Phase 1 and are documented in `docs/protocol/`.

| ADR | Decision | Status |
|---|---|---|
| ADR-001 | Rust as primary implementation language | **Done** — `docs/adr/ADR-001-rust-primary-language.md` |
| ADR-002 | Tokio runtime | **Done** — `docs/adr/ADR-002-tokio-runtime.md` |
| ADR-003 | QUIC/Quinn primary data-plane transport | **Done** — `docs/adr/ADR-003-quic-quinn-transport.md` |
| ADR-004 | H.264 mandatory MVP codec | **Done** — `docs/adr/ADR-004-h264-mandatory-mvp-codec.md` |
| ADR-005 | Signed capability-based session authorization | **Done** — `docs/adr/ADR-005-signed-capability-authorization.md` |
| ADR-006 | Application E2E encryption through relay | **Done** — `docs/adr/ADR-006-application-e2e-encryption-through-relay.md` |
| ADR-007 | Modular monolith control plane | **Done** — `docs/adr/ADR-007-modular-monolith-control-plane.md` |
| ADR-008 | Windows-first host/client MVP | **Done** — `docs/adr/ADR-008-windows-first-mvp.md` |
| ADR-009 | Windows service + per-user desktop-host process separation | **Done** — `docs/adr/ADR-009-windows-service-desktop-host-separation.md` |
| ADR-010 | Protocol core remains OS-independent | **Done** — `docs/adr/ADR-010-protocol-core-os-independent.md` |
| ADR-011 | Relay-only v0.1, P2P in v0.2 | **Done** — `docs/adr/ADR-011-relay-only-v01-p2p-v02.md` |
| ADR-012 | Native client; no Electron | **Done** — `docs/adr/ADR-012-native-client-no-electron.md` |
| ADR-013 | SQLite default for MVP/self-host, PostgreSQL upgrade path | **Done** — `docs/adr/ADR-013-sqlite-default-postgres-upgrade.md` |
| ADR-014 | Separate session-establishment TTL from session-duration limit in `SessionCapability` | **Done** — `docs/adr/ADR-014-session-capability-ttl-semantics.md` |
| ADR-015 | Concurrent-session policy per target device (`desktop.control` exclusive, `desktop.view` shared) | **Done** — `docs/adr/ADR-015-concurrent-session-policy-per-device.md` |
| ADR-016 | Bind agent's advertised protocol range into the session capability to prevent handshake downgrade | **Done** — `docs/adr/ADR-016-bind-agent-protocol-range-into-capability.md` |
| ADR-017 | Continuous authorization via narrow-only policy-snapshot push, not live per-operation approval | **Done** — `docs/adr/ADR-017-continuous-authorization-narrow-only-push.md` |
| ADR-018 | Race P2P and relay connection setup in parallel (not sequentially) during CONNECTING | **Done** — `docs/adr/ADR-018-parallel-p2p-relay-race.md` |
| ADR-019 | Custom reflexive-address discovery over the control-plane channel instead of a standalone STUN server (resolves the Section 58 open question) | **Done** — `docs/adr/ADR-019-custom-reflexive-discovery.md` |
| ADR-020 | IPC between agent service and desktop-host verifies connecting process identity/signature, not ACL alone | **Done** — `docs/adr/ADR-020-ipc-process-authentication.md` |
| ADR-021 | Desktop-host runs in two privilege contexts: `SYSTEM` for Winlogon/pre-login only, as-user for in-session capture | **Done** — `docs/adr/ADR-021-desktop-host-privilege-split.md` |
| ADR-022 | Capture/encode queue backpressure is drop-stale (bounded depth-1, replace not block), extending Principle 3.2 to the capture stage | **Done** — `docs/adr/ADR-022-capture-encode-backpressure-drop-stale.md` |
| ADR-023 | Unattended-access consent/notification is a per-role/per-device policy setting (resolves the Section 58 open question) | **Done** — `docs/adr/ADR-023-unattended-consent-policy-per-role.md` |
| ADR-024 | Desktop-host process crash triggers automatic respawn reusing existing session reconnect semantics, not session termination | **Done** — `docs/adr/ADR-024-desktop-host-crash-respawn.md` |
| ADR-025 | Encoded-frame payload AEAD granularity, directional nonce domains, and stable AAD metadata | **Done** — `docs/adr/ADR-025-encoded-frame-aead-framing.md` |

---

## How to keep this current

- When a crate/app gains real logic beyond the stub, move it from
  **Scaffolded** to **In progress** and note what's implemented.
- When a directory in Section 2 of this file gets created, flip it to
  **Scaffolded** or **In progress** as appropriate.
- When a Phase's exit condition (Section 48 of the spec) is actually
  demonstrated, flip that phase to **Done** and start tracking the next one.
- When an ADR gets written to `docs/adr/`, link it here and mark it **Done**.
