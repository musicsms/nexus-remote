# Nexus

Nexus is a greenfield remote-desktop platform: a Teleport-inspired
identity/access plane paired with a custom, low-latency media/data plane.
It supports two long-term use cases — remote support/work (AnyDesk/RustDesk-like)
and privileged remote access to production systems (Teleport Desktop
Access-like, with SSO, MFA, JIT approval, session recording, device trust,
and granular policy).

Nexus does not fork Teleport. It borrows Teleport's architectural
principles — short-lived identity, device enrollment, policy-driven access,
auditability, reverse connectivity — while building a new desktop engine
optimized for native screen capture, hardware video encoding, QUIC
transport, NAT traversal, and end-to-end encrypted sessions.

```
Control Plane decides
Agent verifies
Endpoints encrypt
Relay forwards
Audit observes
```

## Status

**Phase 0 foundation complete; Phase 1 MVP in progress.** See
[`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) for the
current build status against the target architecture — check it before
assuming any crate has real logic.

All 27 tracked Architecture Decision Records are frozen
(see [`docs/adr/`](docs/adr)); the target design is settled while Phase 1
implementation remains in progress.

## Core goals

- Agent idle: < 30 MB RAM, < 0.2% CPU average.
- First frame: < 1s on LAN, < 2s typical Internet.
- LAN input-to-photon: < 50 ms (stretch: 25–35 ms).
- Direct P2P preferred; relay fallback when P2P is unavailable.
- No inbound port required on the host; no permanent shared passwords.
- End-to-end encryption even when traffic traverses a relay.
- Self-hostable control plane and relay (SQLite by default, zero external
  dependencies).

## Design principles

1. **Lightweight by construction** — no Electron/JVM/browser runtime; native
   OS APIs directly.
2. **Interactive freshness over perfect delivery** — prefer a fresh frame
   over retransmitting a stale one.
3. **Control plane / data plane separation** — the control plane never
   carries desktop video.
4. **Endpoint-verifiable authorization** — the agent verifies signed
   session capabilities locally; no live per-operation approval round-trip.
5. **Relay blindness** — relays forward encrypted traffic and never hold
   the keys to decrypt it.
6. **Modular platform abstractions** — core protocol/session/crypto crates
   never depend on OS-specific APIs.

## Components

| Component | Responsibility |
|---|---|
| `nexusd` | Control plane: auth, devices, policy, sessions, signaling, audit |
| `nexus-agent` | Host service: identity, presence, session lifecycle, privilege boundary |
| `nexus-desktop-host` | User-session process: capture, encode, input, clipboard, audio |
| `nexus-client` | Native viewer/controller |
| `nexus-relay` | Stateless encrypted packet relay |
| `nexus-cli` | Administrative/debugging CLI |

MVP platform target is **Windows host + Windows client**; macOS/Linux host
support is Phase 5 (v1.0).

## Repository layout

```
nexus/
├── Cargo.toml
├── crates/          # OS-independent core logic (protocol, session, crypto, ...)
├── apps/             # Binaries: nexusd, nexus-relay, nexus-agent,
│                      #   nexus-desktop-host, nexus-client, nexus-cli
├── platform/         # OS-specific bindings, behind narrow traits (Phase 0+/5)
├── proto/            # Protobuf schemas (compiled during nexus-protocol build)
├── migrations/       # SQL migrations (not yet created)
├── deployment/        # Deployment manifests (not yet created)
├── test/             # integration / network-sim / performance (not yet created)
└── docs/
    ├── Nexus Remote Desktop Platform - Spec.md   # target architecture
    ├── IMPLEMENTATION_STATUS.md                    # current build status
    ├── adr/                                        # frozen decisions
    ├── protocol/                                    # design notes / edge cases
    └── security/                                    # threat model (Phase 3+)
```

## Documentation map

Read in this order before making non-trivial changes (see
[`CLAUDE.md`](CLAUDE.md) for the full agent/contributor rules):

1. [`docs/Nexus Remote Desktop Platform - Spec.md`](<docs/Nexus Remote Desktop Platform - Spec.md>) —
   target architecture, protocol shape, security model, phase scope. The
   source of truth for design decisions; changes rarely.
2. [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) — what's
   actually built right now versus the spec's target. Changes often.
3. [`docs/adr/`](docs/adr/) — frozen Architecture Decision Records. An ADR
   overrides the spec's prose if the two disagree (more recent, more
   specific). All 27/27 tracked ADRs are written.
4. [`docs/protocol/`](docs/protocol/) — design notes with the reasoning,
   trade-offs, and edge cases behind specific ADRs (session establishment,
   authorization, connectivity/NAT traversal, Windows agent privilege
   boundary, video/media pipeline).

## Building

No CI/task runner exists yet — plain `cargo` workspace:

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Development phases

| Phase | Scope | Exit condition |
|---|---|---|
| 0 — Foundation | Workspace, CI, protocol crate, QUIC/capture/encoder PoCs | Capture a Windows desktop and stream frames between two local processes |
| 1 — MVP v0.1 | Windows host/client, minimal control plane, relay-only QUIC, H.264 1080p60, input, reconnect | Enroll a host + client, control it over the Internet through a relay |
| 2 — v0.2 Connectivity | Candidate discovery, hole punching, P2P QUIC, adaptive bitrate | P2P succeeds on common NATs, falls back to relay |
| 3 — v0.3 Productization | File transfer, audio, recording, RBAC, audit UI, signed updates | — |
| 4 — v0.5 Enterprise | OIDC, SAML, WebAuthn, access requests, ABAC | — |
| 5 — v1.0 | macOS/Linux hosts, HEVC/AV1, enterprise SSO, JIT, device trust | — |

The OS-independent Phase 0 foundation is complete and the native client is in
progress. The client milestone has Linux workspace evidence and a synthetic
QUIC loopback test; the GNU Windows-target check is currently blocked by the
missing MinGW compiler. MSVC/live-Windows smoke and full
host/client/service/relay acceptance are still required before Phase 1 can
exit. See
`docs/IMPLEMENTATION_STATUS.md` for exact per-crate/per-app status.

## Contributing

This project (and any agent working in it, Claude or otherwise) follows the
rules in [`CLAUDE.md`](CLAUDE.md): read the spec/status/ADRs/protocol notes
relevant to your change before writing code, respect the MVP non-goals and
phase scope, keep dependency direction one-way (Product → Core crates →
Platform abstractions → Native OS/codec APIs), and update
`docs/IMPLEMENTATION_STATUS.md` in the same change whenever build status
moves.

## License

Dual-licensed under MIT or Apache-2.0, per `Cargo.toml`.
