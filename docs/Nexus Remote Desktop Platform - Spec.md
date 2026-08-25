Nexus Remote Desktop Platform

Technical Specification & Implementation Blueprint
Version: 0.1 Draft
Architecture: Greenfield Rust, Teleport-inspired identity/access plane, custom low-latency remote desktop data plane
Primary MVP platform: Windows host + Windows client

────────

1. Executive Summary

Nexus is a new remote desktop platform designed for low latency, low resource usage, self-hosting, and enterprise-grade zero-trust access. The project does not fork Teleport. Instead, it borrows architectural principles from Teleport—short-lived identity, device enrollment, policy-driven access, auditability, and reverse connectivity—while building a new remote desktop engine optimized for native screen capture, hardware video encoding, QUIC transport, NAT traversal, and end-to-end encrypted sessions.

The product should support two long-term use cases:

1. Remote support / remote work similar to AnyDesk or RustDesk.
2. Privileged remote access similar to Teleport Desktop Access, with SSO, MFA, JIT approval, session recording, device trust, and granular policy.

The implementation target is a Rust-first codebase with a modular monolith control plane, stateless relay infrastructure, native host agent, native client, and a versioned protocol shared by all components.

Core product goals

• Agent idle memory target: < 30 MB RAM.
• Agent idle CPU target: < 0.2% average.
• First frame: < 1 second on LAN, < 2 seconds typical Internet.
• LAN input-to-photon target: < 50 ms, stretch target 25–35 ms.
• Direct P2P preferred; relay fallback when P2P is unavailable.
• No inbound port required on the host.
• No permanent shared passwords.
• End-to-end encryption even when traffic traverses a relay.
• Self-hostable control plane and relay.
• Protocol and data plane remain independent from UI and platform-specific code.

────────

2. Non-Goals for the Initial MVP

The first MVP must deliberately avoid scope expansion. The following are not required for v0.1:

• macOS host support.
• Linux host support.
• Browser client.
• Mobile client.
• AV1 or HEVC mandatory support.
• Full WebRTC compatibility.
• SAML/SCIM/LDAP.
• Session recording.
• File transfer.
• Audio forwarding.
• Multi-monitor simultaneous streaming.
• JIT approval workflows.
• Device posture checks.
• Global multi-region control-plane HA.

These features are planned, but they must not block the initial end-to-end remote desktop path.

────────

3. Design Principles

3.1 Lightweight by construction

The host agent must not embed Electron, JVM, or a browser runtime. Native Rust processes should use OS APIs directly. The architecture should avoid unnecessary copies, large runtime dependencies, and unbounded queues.

3.2 Interactive freshness over perfect delivery

Remote desktop is an interactive graphics transport system, not a movie streaming system. The protocol should prefer a fresh frame over retransmission of stale frame data.

3.3 Control plane and data plane separation

The control plane authenticates identities, evaluates policy, issues short-lived session capabilities, performs signaling, and records audit events. It should not carry desktop video in the normal path.

3.4 Endpoint-verifiable authorization

The host agent must independently verify a signed session capability. A compromised relay must not grant access. The data plane should not rely on a central server approval check for every operation.

3.5 Relay blindness

Relay nodes should forward encrypted traffic and should not possess keys required to decrypt desktop contents, clipboard content, files, or input events.

3.6 Modular platform abstractions

Core protocol, session, transport, crypto, and policy crates must not depend on Windows, macOS, or Linux APIs. OS-specific functionality must live behind narrow traits.

────────

4. High-Level Architecture

```text
                        +-------------------------+
                        |      Control Plane      |
                        |                         |
                        | Auth / Identity         |
                        | Device Registry         |
                        | Policy Engine           |
                        | Session Broker          |
                        | Signaling               |
                        | Audit                   |
                        +------------+------------+
                                     |
                              HTTPS / gRPC
                                     |
             +-----------------------+-----------------------+
             |                       |                       |
        Desktop Client          Relay / Edge            Host Agent
             |                                               |
             +--------------- QUIC P2P ----------------------+

Preferred path: Client <==== E2E encrypted QUIC ====> Agent
Fallback path:  Client <==== QUIC ====> Relay <==== QUIC ====> Agent
```

4.1 Main components

|Component           |Responsibility                                                         |
|--------------------|-----------------------------------------------------------------------|
|`nexusd`            |Control plane: auth, devices, policy, sessions, signaling, audit       |
|`nexus-agent`       |Host service: identity, presence, session lifecycle, privilege boundary|
|`nexus-desktop-host`|User-session process: capture, encode, input, clipboard, audio         |
|`nexus-client`      |Native viewer/controller                                               |
|`nexus-relay`       |Stateless encrypted packet relay                                       |
|`nexus-cli`         |Administrative and debugging CLI, later phase                          |

────────

5. Repository Layout

```text
nexus/
├── Cargo.toml
├── crates/
│   ├── nexus-common/
│   ├── nexus-crypto/
│   ├── nexus-protocol/
│   ├── nexus-transport/
│   ├── nexus-session/
│   ├── nexus-auth/
│   ├── nexus-policy/
│   ├── nexus-audit/
│   ├── nexus-codec/
│   ├── nexus-capture/
│   ├── nexus-input/
│   ├── nexus-observability/
│   ├── nexus-audio/            # added Phase 3 / v0.3, see Section 28
│   └── nexus-file-transfer/    # added Phase 3 / v0.3, see Section 27
├── apps/
│   ├── nexusd/
│   ├── nexus-relay/
│   ├── nexus-agent/
│   ├── nexus-desktop-host/
│   ├── nexus-client/
│   └── nexus-cli/
├── platform/
│   ├── windows/
│   ├── macos/
│   └── linux/
├── proto/
├── migrations/
├── deployment/
├── test/
│   ├── integration/
│   ├── network-sim/
│   └── performance/
└── docs/
    ├── adr/
    ├── protocol/
    └── security/
```

Dependency rule

```text
Product layer
    ↓
Core crates
    ↓
Platform abstractions
    ↓
Native OS / codec APIs
```

nexus-protocol, nexus-session, and nexus-crypto must never import windows-rs or other OS-specific bindings.

`apps/` binaries may contain their own private internal library crate(s) (e.g. `nexus-client-core`, Section 29) alongside their `main.rs`. These follow the same dependency-direction rule as everything else, but — unlike the crates in `crates/`, which are cross-app OS-independent core logic tracked individually in Appendix A — an app-internal crate is scoped to that one app and is not itself a top-level workspace concern.

The initial Cargo workspace (Phase 0/1) contains only the twelve crates listed above `nexus-audio`: `nexus-common`, `nexus-crypto`, `nexus-protocol`, `nexus-transport`, `nexus-session`, `nexus-auth`, `nexus-policy`, `nexus-audit`, `nexus-codec`, `nexus-capture`, `nexus-input`, `nexus-observability`. `nexus-audio` and `nexus-file-transfer` are scaffolded in Phase 3 (v0.3) alongside the features in Sections 27–28, not before — this keeps the repository layout and the active workspace in agreement. See Appendix A for the same phase mapping per crate.

The tree above is the target end-state layout, built out incrementally across the phases in Section 48. It is not a snapshot of what exists in the repository today. Current build status against this target — which paths are scaffolded, in progress, or not yet started — is tracked separately in `docs/IMPLEMENTATION_STATUS.md` so that status updates (which change often) don't churn this architecture document (which should stay stable).

────────

6. Recommended Technology Stack

Core runtime

• Rust stable.
• Tokio async runtime.
• Axum for HTTP APIs.
• Quinn for QUIC.
• rustls for TLS 1.3.
• Prost for Protobuf control messages.
• Serde for persisted/configuration formats.
• SQLx as the async SQL layer, backend-agnostic by design.
• SQLite as the default embedded database for MVP and single-node self-hosting (zero external dependency, matches the "self-hostable" goal in Section 1).
• PostgreSQL as the upgrade path for multi-tenant or horizontally scaled control-plane deployments, using the same SQLx models/queries.
• tracing + OpenTelemetry.
• Prometheus metrics export.

Cryptography

• Ed25519 for long-term device signing identity.
• X25519 for ephemeral session key agreement.
• HKDF-SHA256 for key derivation.
• ChaCha20-Poly1305 or AES-GCM for application-level E2E data protection where required.
• BLAKE3 for file integrity and fast internal hashing.

Windows media stack

• Windows Graphics Capture as primary capture API.
• DXGI Desktop Duplication as fallback.
• D3D11 textures as the primary GPU frame representation.
• Media Foundation and/or vendor encoder backends.
• NVENC for NVIDIA.
• QSV / Media Foundation for Intel.
• AMF / Media Foundation for AMD.
• H.264 mandatory initial codec.

────────

7. Control Plane Specification

7.1 Initial deployment model

nexusd is a modular monolith for the first releases.

```text
                       nexusd
                         |
             +-----------+-----------+
             |           |           |
           HTTP        gRPC      Signaling
             |           |           |
             +-----------+-----------+
                         |
                 Application layer
                         |
      +------------------+------------------+
      |                  |                  |
    Auth              Devices           Sessions
      |                  |                  |
      +------------------+------------------+
                         |
          SQLite (default) / PostgreSQL (scale)
```

Do not split into microservices until operational scale or organizational boundaries justify it.

SQLite is the MVP and default self-hosted backend; a deployment migrates to PostgreSQL (same SQLx schema/query layer) only when it needs concurrent multi-writer access, horizontal scaling, or multi-region control-plane HA — none of which are MVP requirements per Section 2.

7.2 Control-plane modules

• Authentication.
• User identity.
• Organization/tenant management.
• Device enrollment.
• Device registry.
• Policy evaluation.
• Session brokerage.
• Access requests.
• Signaling.
• Relay discovery.
• Audit logging.
• Configuration and update metadata.

────────

8. Device Identity and Enrollment

Each device generates its private key locally during installation. The private key must never be uploaded to the server.

Enrollment flow

```text
Installer
   |
   +-- generate Ed25519 keypair
   |
   +-- POST /api/v1/devices/enroll
   |      enrollment token
   |      device public key
   |      machine metadata
   |
   +-- server validates one-time token
   |
   +-- server issues signed device certificate/credential
```

Device identity fields

```text
DeviceIdentity {
    device_id
    organization_id
    public_key
    device_type
    os
    architecture
    issued_at
    expires_at
    capabilities
}
```

Requirements

• Enrollment tokens are single-use or tightly limited.
• Device credential rotation is supported.
• Revocation can disable new sessions immediately.
• The agent should continue to verify already-established sessions according to the session policy snapshot unless the session is explicitly revoked.

The policy snapshot an already-established session runs under is not frozen forever: the control plane may push a narrower snapshot to the agent at any time (see Section 12 and ADR-017, `docs/adr/ADR-017-continuous-authorization-narrow-only-push.md`). This is how MVP's static-snapshot model extends into the continuous re-evaluation described in Section 61.3 without a rewrite — a push may only restrict an active session, never grant it permissions beyond what `permissions[]` in the original signed capability allowed.

────────

9. Presence and Signaling

The agent maintains a lightweight outbound control connection to nexusd.

Recommended transport for MVP: WebSocket over TLS. QUIC control connection may be introduced later.

Heartbeat target

• Heartbeat interval: 15–30 seconds.
• Average idle network use: < 1 KB/s target.

Presence channel responsibilities

• Online/offline state.
• Agent version.
• Current network candidates.
• Capability changes.
• Session signaling.
• Configuration refresh notifications.
• Update notifications.

Video and audio must never traverse this control connection.

────────

10. User Authentication

MVP

• Email/password.
• TOTP MFA.
• Session cookies or short-lived access tokens for the client UI/API.

Enterprise roadmap

• OIDC.
• SAML.
• WebAuthn/passkeys.
• SCIM.
• LDAP/AD bridge if required.

Authentication proves who the user is. Authorization for a remote session is represented separately by a short-lived signed capability.

────────

11. Authorization Model

Nexus should use an RBAC + ABAC hybrid.

Authorization tuple:

```text
subject + resource + action + context
```

Example:

```yaml
role: production-support
allow:
  device_labels:
    environment: production
  actions:
    - desktop.view
    - desktop.control
conditions:
  require_mfa: true
  managed_client_device: true
  clipboard: false
  file_transfer: false
```

First-class actions

• desktop.view
• desktop.control
• clipboard.read
• clipboard.write
• file.upload
• file.download
• audio.listen
• audio.send
• session.record
• session.request
• session.approve

────────

12. Session Capability

The most important authorization artifact is a signed, short-lived session capability.

Conceptual structure:

```text
SessionCapability {
    version
    issuer
    session_id

    subject_user_id
    client_device_id
    target_device_id

    permissions[]
    restrictions

    not_before
    expires_at
    nonce

    agent_min_protocol
    agent_max_protocol

    client_ephemeral_public_key
    signature
}
```

`agent_min_protocol`/`agent_max_protocol` pin the agent's advertised
protocol range (Section 31) as reported over presence (Section 9) at the
moment the capability is issued. The Section 13 mutual identity-proof
handshake must reject a `negotiated_protocol` outside this signed range —
see ADR-016 for the downgrade attack this closes and why the range is bound
here rather than signing the live negotiated value.

Example restrictions:

```text
clipboard = disabled
file_transfer = disabled
recording = required
max_duration = 30m
max_resolution = 2560x1440
unattended_consent = notify   # notify | silent — see ADR-023
```

Properties

• TTL for establishing a session: typically 30–120 seconds.
• Bound to client device and target device.
• Bound to a session ID and nonce.
• Signed by a control-plane signing key.
• Host agent validates the signature locally.
• Replay attempts must fail.

`expires_at` (with `not_before`) governs only the establishment window: the
agent must receive and validate the initial `SessionHello` carrying this
capability before `expires_at`. Once the session reaches ESTABLISHED,
`expires_at` is no longer checked — ongoing session duration is governed
exclusively by the `max_duration` restriction (see example above) plus
explicit revoke and the reconnect-window rules in Section 46. See
ADR-014 (`docs/adr/ADR-014-session-capability-ttl-semantics.md`) for the
rationale; this keeps the wire schema unchanged while removing the
ambiguity between a short establishment TTL and a long-running active
session.

`restrictions` may be updated in place on an ESTABLISHED session via a
signed `session.policy_update` message over the presence channel (Section
9). The agent applies the new `restrictions` only if it narrows the
existing ones; it must reject any update that would grant a permission not
present in the original `permissions[]`, since that field is part of what
the initial signature covers. Widening access always requires a fresh
capability and a fresh identity handshake. See ADR-017.

────────

13. Session Establishment

```text
Client                         Control Plane                    Agent
  |                                  |                           |
  |--- request session ------------->|                           |
  |                                  |--- signal request ------->|
  |                                  |                           |
  |<-- signed capability ------------|                           |
  |                                  |                           |
  |----------- connect candidate / relay ----------------------->|
  |                                                              |
  |<---------------- agent identity proof -----------------------|
  |---------------- client identity proof ---------------------->|
  |                                                              |
  |=========== E2E encrypted session established ================|
```

Session state machine

```text
REQUESTED
  -> AUTHORIZED
  -> CONNECTING
  -> ESTABLISHED
  -> ACTIVE
  -> DISCONNECTED
  -> ENDED

Failure states:
DENIED
EXPIRED
FAILED
REVOKED
```

State transitions must be idempotent and auditable.

A session request for `desktop.control` on a target device that already has
an ACTIVE `desktop.control` session is denied (`DENIED`, reason: device
already under control); `desktop.view` may be granted concurrently
regardless of an active `desktop.control` session on the same device. See
ADR-015 for the full policy and the race-safety requirement on the
control-plane check.

────────

14. Data-Plane Transport

QUIC is the primary transport.

QUIC logical layout

```text
QUIC connection
|
+-- reliable stream: session control
+-- reliable stream: keyboard / mouse
+-- reliable stream: clipboard
+-- reliable stream: file-control metadata
+-- reliable stream: diagnostics
|
+-- QUIC datagrams: video packets
+-- QUIC datagrams: audio packets
+-- QUIC datagrams: cursor position updates
```

Rationale

Reliable streams are appropriate for control and state transitions. Video should use datagrams so loss of an old packet does not block a newer frame.

Mandatory transport behavior

• Datagram sequence numbers.
• Frame IDs.
• Stream IDs for monitor/video source.
• Loss tracking.
• Keyframe request mechanism.
• RTT and jitter estimation.
• Bounded send queues.
• Backpressure.
• Congestion-aware bitrate adaptation.

────────

15. Connectivity and NAT Traversal

Connection preference order:

1. Same-LAN direct candidate.
2. Internet P2P UDP hole-punched candidate.
3. UDP relay.
4. TCP/TLS relay fallback.

Candidate model

```text
Candidate {
    type: LAN | REFLEXIVE | RELAY
    address
    port
    protocol
    priority
    expires_at
}
```

MVP connectivity

v0.1 may intentionally support relay-only connectivity. This reduces risk while the media pipeline is stabilized.

v0.2 NAT traversal

Implement an ICE-like candidate exchange without requiring full WebRTC.
Reflexive-address discovery is a custom endpoint on the existing
authenticated control-plane channel rather than a standalone STUN server
(ADR-019, `docs/adr/ADR-019-custom-reflexive-discovery.md`) — this avoids
exposing an additional unauthenticated service to the Internet and reuses
trust already established for signaling. A STUN-compatible mode can be
added later for interop if needed. Symmetric NAT and blocked UDP fall back
to relay.

Relay connection setup and P2P connectivity checks run in parallel from the
start of CONNECTING, not sequentially (ADR-018,
`docs/adr/ADR-018-parallel-p2p-relay-race.md`) — whichever candidate
succeeds first is used, with a short grace window to prefer a successful
P2P candidate over a relay candidate that also succeeded. A sequential
"try P2P, then fall back to relay" design would risk missing the <1–2s
first-frame target (Section 1) on networks where P2P negotiation is slow.

The client must always treat the `candidates` array returned by
`POST /api/v1/sessions` (Section 32) as a list to iterate, even in v0.1
where it contains exactly one relay candidate. This keeps the v0.1 → v0.2
transition additive (more candidate types appended to the same list)
instead of requiring a client rewrite once P2P candidates are introduced.

────────

16. Relay Architecture

The relay should be deliberately dumb and horizontally scalable.

Relay knows

• Session ID.
• Endpoint connection IDs.
• Relay token claims.
• Byte counts.
• Timing and network metrics.

Relay must not know

• Desktop pixels.
• Clipboard contents.
• File contents.
• Keyboard values.
• E2E session encryption keys.

Relay authentication token

```text
RelayToken {
    session_id
    relay_id
    client_device_id
    target_device_id
    expires_at
    signature
}
```

A relay must validate this token without a database lookup for every packet.

────────

17. End-to-End Encryption

Transport-level QUIC TLS is required, but relay traversal should also support application-level E2E encryption so the relay cannot decrypt content.

Suggested model:

1. Client and agent authenticate device/session identities.
2. Exchange ephemeral X25519 keys.
3. Derive session keys via HKDF-SHA256.
4. Encrypt application payloads with ChaCha20-Poly1305 or AES-GCM.
5. Derive separate directional/channel keys.

Never invent custom cryptographic primitives.

────────

18. Windows Agent Architecture

Windows requires separation between the system service and the interactive desktop process.

```text
+-------------------------------+
| nexus-agent-service.exe       |
|                               |
| Identity                      |
| Network / Presence            |
| Updates                       |
| Privileged operations         |
| Session process spawning      |
+---------------+---------------+
                |
         narrow IPC boundary
                |
+---------------v---------------+
| nexus-desktop-host.exe        |
|                               |
| Screen capture                |
| Hardware encoding             |
| Input injection               |
| Cursor                        |
| Clipboard                     |
| Audio                         |
+-------------------------------+
```

The privileged service must expose only a minimal IPC API to the desktop process.

`nexus-desktop-host.exe` runs in two distinct privilege contexts, not one:
a `SYSTEM`-privileged instance spawned into the `Winlogon` desktop, used only
to reach the pre-login/lock screen for unattended access; and an
instance spawned via `CreateProcessAsUser` running as the interactive user,
used for normal in-session capture. Running the in-session capture process
as `SYSTEM` would violate the minimum-privilege principle in Section 45.
See ADR-021 (`docs/adr/ADR-021-desktop-host-privilege-split.md`).

Note: neither Windows Graphics Capture nor DXGI Desktop Duplication can
capture the Secure Desktop (UAC elevation prompts, Ctrl+Alt+Del screen) —
this is a Windows security boundary, not an implementation gap. The client
UX for this state (e.g. a placeholder overlay while the remote desktop is
on the Secure Desktop) is not yet decided; see Section 58.

If the desktop-host process crashes mid-session, the service detects the
process exit and respawns it, reusing the existing session's reconnect
semantics (Section 46) rather than ending the session outright — see
ADR-024 (`docs/adr/ADR-024-desktop-host-crash-respawn.md`).

────────

19. Screen Capture

Capture priority on Windows:

1. Windows Graphics Capture.
2. DXGI Desktop Duplication fallback.
3. GDI only for diagnostics/fallback, not normal production use.

Frame representation

Primary frame representation should be a GPU-resident D3D11 texture.

Desired path:

```text
Desktop compositor
      -> D3D11 texture (RGBA8)
      -> D3D11 Video Processor color conversion (RGBA8 -> NV12, on VRAM)
      -> hardware encoder
      -> encoded bitstream
      -> packetizer
      -> QUIC datagrams
```

The color-conversion step is not optional: hardware H.264 encoders
(NVENC/QSV/AMF) take NV12 input, not the compositor's native RGBA8/BGRA8
texture format, so this step exists on the normal path even though it was
previously implicit rather than shown (see Section 61.1). The normal path
should avoid GPU-to-CPU framebuffer copies — the conversion above stays on
VRAM.

────────

20. Video Encoding

Codec policy

• H.264: mandatory MVP codec.
• HEVC: optional post-MVP.
• AV1: later optimization for capable hardware.

Encoder abstraction

```rust
pub trait VideoEncoder {
    fn configure(&mut self, config: EncoderConfig) -> Result<()>;
    fn encode(&mut self, frame: VideoFrame) -> Result<EncodedFrame>;
    fn request_keyframe(&mut self) -> Result<()>;
    fn reconfigure(&mut self, config: EncoderConfig) -> Result<()>;
}
```

Backends

• NVIDIA NVENC.
• Intel QSV / Media Foundation.
• AMD AMF / Media Foundation.
• Software encoder only as fallback and for CI/testing.

Default MVP operating point

• 1920×1080.
• Up to 60 fps.
• Hardware H.264.
• Adaptive bitrate.

────────

21. Video Packet Format

Control messages can use Protobuf. Video packets should use a compact binary header.

Conceptual header:

```text
version        u8
flags          u8
stream_id      u16
frame_id       u32
packet_id      u16
packet_count   u16
timestamp_us   u64
payload_len    u16
payload        bytes
```

Flags may include:

• KEYFRAME.
• FRAME_START.
• FRAME_END.
• FEC.
• CONFIG.

All fields and byte order must be formally specified before compatibility is promised.

────────

22. Adaptive Quality Controller

Create a dedicated QualityController rather than scattering adaptation logic across the encoder and transport.

Inputs

• RTT.
• Packet loss.
• Jitter.
• Estimated available bandwidth.
• Send queue depth.
• Encode latency.
• Decode latency.
• Render queue depth.

Outputs

• Bitrate.
• FPS cap.
• Resolution scale.
• Quantizer target.
• Keyframe interval.
• Optional FEC rate.

Control rule

Degrade quickly when the network worsens; increase quality slowly when conditions improve. Tune the specific "quickly"/"slowly" constants against the network-simulation profiles already defined in Section 47, rather than as free parameters — the profiles exist precisely to make this measurable.

A resolution-scale change is not a free adjustment: it breaks inter-frame prediction, so the encoder must issue a forced keyframe immediately on any resolution-scale change, in addition to the existing keyframe triggers (packet-loss recovery, post-reconnect per Section 46).

Backpressure at the capture/encode boundary follows Principle 3.2
(interactive freshness): the queue between capture and encode is bounded to
depth 1 with replace-not-block semantics — a new frame replaces a still-queued
older one rather than the capture thread blocking on a slow encoder. See
ADR-022 (`docs/adr/ADR-022-capture-encode-backpressure-drop-stale.md`).

────────

23. Cursor Handling

The cursor should normally be sent separately from the video stream.

Agent sends:

```text
CursorShape {
    id
    width
    height
    hotspot_x
    hotspot_y
    pixel_format
    data
}

CursorPosition {
    x
    y
    visible
    shape_id
}
```

The client renders the cursor locally. This avoids cursor latency being coupled to capture + encoding + decoding latency.

────────

24. Input Protocol

Do not send Windows virtual-key codes as the universal wire format.

Wire-level input types:

• KeyDown
• KeyUp
• TextInput
• MouseMove
• MouseButton
• MouseWheel
• Touch
• Pen later

Keyboard event should preserve physical key, logical key, Unicode text where appropriate, and modifier state. This is required for international keyboard layouts and IME support.

────────

25. Multi-Monitor and Coordinate System

Agent advertises:

```text
MonitorInfo {
    monitor_id
    origin_x
    origin_y
    width_px
    height_px
    scale_factor
    refresh_hz
    primary
}
```

Use a stable global logical desktop coordinate system on the wire. The host performs the final transform into physical coordinates for each monitor.

Do not make the client responsible for Windows DPI internals.

MVP can support one selected monitor at a time; multi-stream monitor output is a later feature.

────────

26. Clipboard

Clipboard runs on its own reliable channel.

Supported initial MIME types:

• text/plain
• text/html later
• image/png later
• file-list later

Policy modes:

• disabled.
• read-only.
• write-only.
• bidirectional.

Enforce payload size and rate limits. Recommended text clipboard MVP max size: 1–10 MB configurable.

────────

27. File Transfer

Post-MVP feature.

Protocol operations:

```text
FileTransferInit
FileMetadata
FileChunk
FileAck
FileCancel
FileComplete
```

Required behavior:

• Resumable transfers.
• BLAKE3 integrity verification.
• Per-session rate limits.
• Policy checks.
• Transfer audit events.
• Explicit destination-path rules.

────────

28. Audio

Post-MVP feature.

Windows audio capture: WASAPI loopback.

Codec: Opus.

Transport: QUIC datagrams.

Target end-to-end audio latency: < 100 ms typical.

────────

29. Client Architecture

Avoid Electron.

Recommended split:

```text
nexus-client
|
+-- UI layer
|
+-- nexus-client-core
      +-- session
      +-- transport
      +-- crypto
      +-- decoder
      +-- renderer
      +-- input
```

Potential UI frameworks:

• Slint: preferred candidate for polished native UI.
• iced.
• egui for early developer tooling.

The remote-display surface should be native/GPU rendered rather than drawn through a browser canvas.

`nexus-client-core` is a library crate private to `apps/nexus-client/` (see Section 5), not a member of the top-level `crates/` directory — its submodules (session, transport, crypto, decoder, renderer, input) are client-specific wiring, not cross-app shared logic, so it is not tracked in Appendix A alongside the OS-independent core crates.

────────

30. Video Decoding and Rendering

Desired Windows client path:

```text
QUIC
  -> compressed H.264 frame
  -> hardware decoder
  -> GPU texture
  -> D3D11 renderer
```

Avoid CPU image conversion unless required by fallback paths.

Future platforms:

• macOS: VideoToolbox + Metal.
• Linux: VAAPI + Vulkan/OpenGL/wgpu.

────────

31. Protocol Versioning

Protocol compatibility must be explicit from the first release.

Handshake example:

```text
client_min_protocol = 1
client_max_protocol = 3
agent_min_protocol  = 2
agent_max_protocol  = 4
negotiated_protocol = 3
```

Feature negotiation is separate from protocol version.

Capabilities may include:

• H264.
• HEVC.
• AV1.
• MULTI_MONITOR.
• AUDIO.
• CLIPBOARD_IMAGE.
• FILE_TRANSFER.
• HDR.
• TOUCH.

Never infer capabilities only from agent version.

────────

32. Control API

Initial REST-style public API:

```text
/api/v1/auth/*
/api/v1/users
/api/v1/devices
/api/v1/sessions
/api/v1/access-requests
/api/v1/roles
/api/v1/audit
/api/v1/relays
```

Create session

POST /api/v1/sessions

Request:

```json
{
  "target_device_id": "dev_01",
  "permissions": [
    "desktop.view",
    "desktop.control"
  ]
}
```

Response:

```json
{
  "session_id": "ses_01",
  "capability": "<signed-binary-or-token>",
  "candidates": [
    {"type": "relay", "region": "sgp1"}
  ]
}
```

The exact token encoding must be specified as a separate protocol ADR.

────────

33. Protobuf Control Messages

Initial protocol examples:

```protobuf
message SessionHello {
  uint32 protocol_version = 1;
  string session_id = 2;
  string device_id = 3;
  bytes capability = 4;
  bytes ephemeral_public_key = 5;
}

message MouseMove {
  sint32 x = 1;
  sint32 y = 2;
}

message MonitorInfo {
  uint32 id = 1;
  sint32 origin_x = 2;
  sint32 origin_y = 3;
  uint32 width = 4;
  uint32 height = 5;
  float scale = 6;
}
```

Protocol schemas belong in proto/ and are generated during build. Breaking field reuse is forbidden.

────────

34. Database Model

Initial database tables (SQLite by default for MVP/self-host; the same schema runs on PostgreSQL once a deployment upgrades — see Section 6):

• organizations
• users
• user_identities
• devices
• device_labels
• device_credentials
• roles
• role_bindings
• sessions
• session_events
• access_requests
• audit_events
• relay_nodes

Device table fields

```text
id
organization_id
hostname
os
os_version
architecture
agent_version
public_key
last_seen_at
status
capabilities_json
created_at
revoked_at
```

Session fields

```text
id
organization_id
user_id
client_device_id
target_device_id
status
connection_mode
relay_id
created_at
started_at
ended_at
policy_snapshot_json
termination_reason
```

Schema and migrations must stay portable between SQLite and PostgreSQL (avoid backend-specific types/features) so the SQLite-to-PostgreSQL upgrade path stays a configuration change, not a rewrite.

────────

35. Audit Model

```text
AuditEvent {
    event_id
    timestamp
    organization_id
    user_id?
    device_id?
    session_id?
    event_type
    metadata
}
```

Initial event types:

• user.login
• user.mfa
• device.enroll
• device.revoke
• session.request
• session.authorize
• session.deny
• session.start
• session.disconnect
• session.end
• clipboard.read
• clipboard.write
• file.upload
• file.download
• access.request
• access.approve

Audit events should optionally form a tamper-evident hash chain:

```text
hash_n = HASH(serialized_event_n || hash_n-1)
```

────────

36. Session Recording

Post-MVP.

Do not perform a second screen capture for recording. Reuse the encoded stream when possible.

Conceptual output:

```text
session/<session-id>/
├── metadata.json
├── video.mkv
└── events.bin
```

Storage backends:

• Local disk for development.
• S3-compatible object storage.
• MinIO.
• Cloud object storage adapters later.

Recording policy must be included in the signed session capability so the host cannot silently downgrade a mandatory recording requirement.

────────

37. Device Trust / Posture

Future policy signals:

• Secure Boot enabled.
• Disk encryption enabled.
• OS patch level.
• Agent signature/health.
• Corporate certificate present.
• EDR presence.

Example policy:

```yaml
allow:
  device_labels:
    environment: production
conditions:
  client_posture:
    secure_boot: true
    disk_encrypted: true
```

────────

38. Just-in-Time Access

Future enterprise flow:

```text
User requests production workstation
          |
          v
AccessRequest
          |
Approver reviews reason + duration
          |
          v
Temporary policy grant
          |
          v
Short-lived session capability
```

Audit must include requester, approver, reason, target device, approved duration, and resulting session IDs.

────────

39. Update System

Agent update security is critical.

Signed update manifest:

```text
UpdateManifest {
    version
    channel
    platform
    architecture
    sha256
    artifact_url
    signature
}
```

Channels:

• stable.
• beta.
• nightly.

Enterprise deployments may pin or disable automatic updates.

Rollback strategy must be defined before auto-update becomes default.

────────

40. Observability

Use tracing throughout all Rust components.

Expose OpenTelemetry traces and Prometheus metrics.

Core operational metrics

• Active sessions.
• Session setup success rate.
• Session setup latency.
• P2P success ratio.
• Relay bandwidth.
• Relay concurrent connections.
• Agent online count.
• Authentication failures.
• Policy denials.

Per-session quality metrics

• FPS.
• Bitrate.
• RTT.
• Jitter.
• Packet loss.
• Encode latency.
• Decode latency.
• Render latency.
• Send queue depth.
• Frame drops.

────────

41. Built-In Latency Instrumentation

Every encoded frame should carry timing metadata in debug/telemetry mode:

```text
capture_ts
encode_start_ts
encode_end_ts
send_ts
receive_ts
decode_start_ts
decode_end_ts
render_ts
```

Client debug overlay example:

```text
FPS          59.7
RTT          21 ms
Bitrate      8.4 Mbps
Encode       3.2 ms
Decode       2.8 ms
Frame Queue  1
Loss         0.4%
```

Do this from the beginning. Latency problems are extremely hard to optimize without stage-level measurements.

────────

42. Performance Budgets

Agent idle

• RAM: < 30 MB target.
• CPU: < 0.2% average target.
• Background network: < 1 KB/s average target.

Active 1080p60 session with hardware encoding

• Agent CPU: < 10% on a modern 4+ core machine, excluding pathological desktop content.
• Agent RAM: < 150 MB target.
• Encoder queue: ideally <= 1 frame.
• Decoder/render queue: ideally <= 1 frame.

Typical bandwidth

• Office productivity: 1–8 Mbps.
• High-motion content: 5–20 Mbps.

These numbers are targets, not guarantees, and must be validated on defined test hardware.

────────

43. Latency Budget

LAN aspirational budget:

```text
capture        5 ms
encode         5 ms
network        3 ms
decode         5 ms
render         8 ms
-------------------
approx.       26 ms
```

Typical Internet with ~20 ms RTT should aim for roughly 40–60 ms perceived interaction latency depending on frame timing.

The system should optimize queue depth before visual fidelity. A beautiful frame delivered 200 ms late is a bad remote desktop frame.

────────

44. Security Threat Model

|Threat                     |Required mitigation                                           |
|---------------------------|--------------------------------------------------------------|
|Stolen server DB           |Device private keys remain endpoint-local                     |
|Rogue relay                |Application E2E encryption                                    |
|Session token theft        |Short TTL, target/client binding, nonce                       |
|Token replay               |Session-bound nonce + single-use establishment semantics      |
|MITM                       |Mutual identity verification and signed capabilities          |
|Compromised desktop process|Minimal privileged service API                                |
|Clipboard exfiltration     |Capability/policy controls + audit                            |
|Unauthorized file transfer |Explicit action permission + audit                            |
|Recording tampering        |Signed/hash-chained metadata                                  |
|Lost device                |Device revocation                                             |
|Control-plane compromise   |Key rotation, audit, separation of signing keys where feasible|

A formal threat-model document should be produced before v0.3 enterprise features.

────────

45. Secure Privilege Boundary

On Windows, nexus-agent-service may run with elevated privilege, but the media process should use the minimum privilege needed for the active user desktop — concretely, `SYSTEM` only when reaching the pre-login `Winlogon` desktop for unattended access, and the interactive user's own privilege for normal in-session capture (Section 18, ADR-021).

The privileged IPC surface should be allow-listed, authenticated, versioned, and fuzz-tested. "Authenticated" means verifying the connecting process's identity (code signature/hash), not only the Windows ACL on the pipe — an ACL restricts which account can connect, not which binary (ADR-020).

The desktop process must not have unrestricted APIs to:

• install drivers.
• replace service binaries.
• modify arbitrary system files.
• change update signing configuration.

────────

46. Reconnect and Failure Semantics

Remote desktop must tolerate transient connectivity loss.

Required state

• Session ID remains stable during reconnect window.
• New transport connection must prove the same session/device identity.
• Encoder should issue a fresh keyframe after media reconnection.
• Input events must not be replayed accidentally.
• Clipboard and file transfer state must be independently resumable or explicitly aborted.

This section is written from the network-loss perspective, but the same
semantics apply when `nexus-desktop-host` itself crashes mid-session (e.g. a
faulty encoder backend): the service detects the process exit and respawns
the desktop-host under the same `session_id`, treating it as a reconnect
rather than ending the session (ADR-024).

Recommended default reconnect window: 30–120 seconds configurable.

────────

47. Testing Strategy

Unit tests

• Capability validation.
• Policy evaluation.
• Packet serialization.
• Sequence rollover.
• Replay prevention.
• Codec abstraction behavior.

Integration tests

• Client -> relay -> agent session.
• Invalid capability rejection.
• Expired capability rejection.
• Reconnect.
• Agent restart.
• Relay restart/failover.
• Device revocation.

Network simulation

Automated test profiles:

• 20 ms RTT, 0% loss.
• 80 ms RTT, 1% loss.
• 150 ms RTT, 3% loss.
• 100 ms RTT, 5% burst loss.
• Bandwidth drops from 20 Mbps to 2 Mbps.
• Temporary 3-second network outage.

Performance regression tests

Track:

• Encode latency.
• Decode latency.
• Memory use.
• Connection setup time.
• First-frame time.
• FPS stability.

────────

48. Development Phases

Phase 0 - Foundation

Deliverables:

• Rust workspace.
• CI.
• logging/tracing.
• protocol crate.
• QUIC proof of concept.
• Windows capture proof of concept.
• H.264 hardware encoder proof of concept.

Exit condition: capture a Windows desktop and stream frames between two local processes.

Phase 1 - MVP v0.1

Scope:

• Windows host.
• Windows client.
• nexusd minimal auth/device/session API.
• Device enrollment.
• Relay-only QUIC.
• H.264 1080p up to 60 fps.
• Keyboard/mouse.
• Local cursor rendering.
• Basic reconnect.
• Telemetry overlay.

Exit condition: a user can enroll a host and client, request an authorized session, and control the host over the Internet through a relay.

Phase 2 - v0.2 Connectivity

Scope:

• Candidate discovery.
• UDP hole punching.
• P2P QUIC.
• Relay fallback.
• Adaptive bitrate.
• Clipboard text.
• Single-monitor switching / improved multi-monitor metadata.
• Reconnect hardening.

Exit condition: direct P2P succeeds on common residential NATs and automatically falls back to relay.

Phase 3 - v0.3 Productization

Scope:

• File transfer.
• Audio.
• Session recording.
• RBAC.
• Audit UI/API.
• Signed updates.
• Installer polish.

Phase 4 - v0.5 Enterprise

Scope:

• OIDC.
• SAML.
• WebAuthn.
• Access requests/approval.
• Device labels.
• ABAC policy conditions.
• Multi-region relay selection.

Phase 5 - v1.0

Scope targets:

• Windows/macOS/Linux host support.
• H.264 plus optional HEVC/AV1.
• P2P + relay.
• Enterprise SSO.
• JIT access.
• Recording.
• Audit.
• Device trust.
• Self-hosted and managed-cloud deployment patterns.

────────

49. MVP Backlog Breakdown

Epic A - Core protocol

• Define protocol version negotiation.
• Define SessionHello.
• Define capability encoding.
• Define control messages.
• Define input events.
• Define cursor messages.
• Define video datagram header.
• Implement encode/decode tests.
• Add fuzz targets for packet parsing.

Epic B - Windows capture/encode

• Windows Graphics Capture wrapper.
• DXGI fallback.
• D3D11 frame abstraction.
• H.264 NVENC backend.
• Intel backend.
• AMD backend.
• Software fallback for test.
• Keyframe request.
• Encoder reconfiguration.

Epic C - Transport

• Quinn connection manager.
• Reliable control stream.
• Input stream.
• Datagram video transport.
• Packet sequencing.
• RTT/loss metrics.
• Bounded queues.
• Relay protocol.

Epic D - Agent

• Device key generation.
• Enrollment.
• Presence channel.
• Session request handler.
• Session capability verification.
• Desktop-host spawning.
• IPC security.
• Update hooks.

Epic E - Client

• Authentication.
• Device list.
• Session request.
• QUIC transport.
• H.264 hardware decoding.
• Native renderer.
• Keyboard/mouse capture.
• Local cursor.
• Debug overlay.

Epic F - Control plane

• Organization/user schema.
• Login/MFA.
• Device enrollment API.
• Device registry.
• Session creation API.
• Capability signing.
• Agent signaling.
• Relay registry.
• Audit events.

────────

50. MVP Acceptance Criteria

The MVP is considered deployable for internal pilot only when all of the following are true:

Functional

• A new Windows host can be enrolled without opening inbound ports.
• A Windows client can authenticate and list authorized hosts.
• Client can request a session and receive a short-lived capability.
• Agent independently rejects expired, invalid, wrong-device, and replayed capabilities.
• Client can view 1080p desktop output.
• Keyboard and mouse control work reliably.
• Cursor is rendered separately from video.
• Temporary network loss can reconnect without creating a second user session.

Security

• No permanent remote desktop password exists.
• Device private keys are never stored server-side.
• Relay cannot decrypt desktop payloads.
• All control-plane traffic uses TLS.
• Session establishment is authenticated at both endpoints.
• Audit records exist for login, enrollment, authorization, session start, and session end.

Performance

• Agent idle RAM is measured and documented.
• First-frame latency is measured.
• Encode/decode stage timings are exposed.
• 1080p30 works on defined minimum test hardware.
• 1080p60 works on defined recommended hardware.
• No unbounded media queue exists.

Reliability

• 1-hour session soak test passes without memory growth beyond agreed tolerance.
• Relay reconnect test passes.
• Agent restart behavior is deterministic.
• Network loss simulation is part of CI/nightly testing.

────────

51. Initial ADRs to Freeze Before Heavy Implementation

Create Architecture Decision Records for at least these decisions:

1. ADR-001: Rust as primary implementation language.
2. ADR-002: Tokio runtime.
3. ADR-003: QUIC/Quinn primary data-plane transport.
4. ADR-004: H.264 mandatory MVP codec.
5. ADR-005: Signed capability-based session authorization.
6. ADR-006: Application E2E encryption through relay.
7. ADR-007: Modular monolith control plane.
8. ADR-008: Windows-first host/client MVP.
9. ADR-009: Windows service + per-user desktop-host process separation.
10. ADR-010: Protocol core remains OS-independent.
11. ADR-011: Relay-only v0.1, P2P in v0.2.
12. ADR-012: Native client; no Electron.
13. ADR-013: SQLite as the default MVP/self-host database, with PostgreSQL as the drop-in upgrade path for scaled/multi-tenant deployments.

These ADRs prevent architectural drift during the first implementation cycle.

────────

52. Suggested First 6 Implementation Sprints

Sprint 1 - Wire skeleton

• Workspace and CI.
• nexus-protocol.
• nexus-transport QUIC POC.
• Basic tracing.
• Loopback sender/receiver.

Demo: transmit synthetic frames and input messages over QUIC locally.

Sprint 2 - Windows video path

• Windows Graphics Capture.
• D3D11 texture abstraction.
• H.264 hardware encode.
• H.264 client decode.
• Native render surface.

Demo: view a local Windows desktop from another process.

Sprint 3 - Relay

• nexus-relay.
• Relay token.
• Encrypted tunnel.
• Metrics.
• Bandwidth limits.

Demo: desktop stream through a remote relay.

Sprint 4 - Identity and enrollment

• Device key generation.
• nexusd.
• Database schema (SQLite default, PostgreSQL-compatible).
• Device enrollment.
• Presence.
• Minimal user auth.

Demo: enrolled devices appear online in client.

Sprint 5 - Authorized sessions

• Session API.
• Policy stub.
• Signed capability.
• Agent verification.
• Mutual endpoint handshake.
• Audit events.

Demo: authorized user connects; unauthorized client is rejected by the agent.

Sprint 6 - Hardening

• Reconnect.
• Adaptive bitrate v1.
• Local cursor.
• Network simulation.
• Performance metrics.
• Soak tests.
• Installer/service lifecycle.

Demo: stable one-hour remote session under simulated network degradation.

────────

53. Deployment Blueprint

Minimal self-hosted deployment

```text
Internet
   |
+--+-------------------------------+
| reverse proxy / TLS              |
+--+-------------------------------+
   |
   +--> nexusd
   |       |
   |       +--> SQLite (default, single file on local disk)
   |
   +--> nexus-relay
```

Suggested production separation:

```text
Control plane network:
  nexusd
  SQLite (default) or PostgreSQL (once multi-writer/scale is needed)
  object storage

Edge network:
  relay-sgp1
  relay-hkg1
  relay-fra1
```

Relay nodes should be replaceable/stateless. Control plane can begin as a single-region HA deployment later. Moving from SQLite to PostgreSQL is a `database.driver` configuration change (Section 54), not a schema rewrite, so self-hosters can start on SQLite and graduate only when they actually need it.

────────

54. Configuration Model

Example nexusd configuration:

```yaml
server:
  listen: 0.0.0.0:8443

database:
  driver: sqlite   # sqlite (MVP/self-host default) | postgres (scaled/multi-tenant)
  url: ${DATABASE_URL}   # e.g. sqlite:///var/lib/nexus/nexus.db or postgres://...

identity:
  signing_key: /var/lib/nexus/keys/session-signing.key

auth:
  password_login: true
  totp: true

relay:
  registration_secret_file: /var/lib/nexus/relay-secret

audit:
  backend: sqlite   # inherits database.driver; postgres once the control plane scales
```

Example relay configuration:

```yaml
relay:
  id: sgp1-01
  region: sgp1
  listen_udp: 0.0.0.0:4433
  control_plane: https://control.example.com
  max_sessions: 5000
  bandwidth_limit_mbps: 10000
```

Example agent configuration:

```yaml
control_plane: https://control.example.com
state_dir: C:\ProgramData\Nexus
update_channel: stable
relay_preference: auto
```

Secrets must not be stored directly in normal configuration files when platform secure storage is available.

────────

55. CI/CD Requirements

Required CI jobs:

• Rust formatting.
• Clippy.
• Unit tests.
• Protocol compatibility tests.
• Fuzz smoke tests.
• SQL migration tests.
• Windows build.
• Signed-artifact dry run.
• Integration test: client -> relay -> agent.

Nightly jobs:

• Network simulation matrix.
• One-hour soak test.
• Memory regression.
• Performance benchmark.
• Dependency vulnerability scan.

Release pipeline must produce signed binaries and signed update manifests.

────────

56. Definition of Done for a Protocol Feature

A protocol feature is not complete until:

• Schema or binary layout is documented.
• Version/capability interaction is specified.
• Happy-path unit tests exist.
• Malformed input tests exist.
• Backward compatibility behavior is documented.
• Metrics/logging exist.
• Security impact is reviewed.
• At least one end-to-end integration test exists.

────────

57. Engineering Rules

1. No unbounded channels in the media path.
2. No blocking I/O on Tokio runtime worker threads.
3. No unsafe without a narrow module boundary and documented invariants.
4. OS/codec FFI must be wrapped in safe abstractions.
5. Protocol parsers treat all remote input as hostile.
6. Every network message has maximum size limits.
7. Every session has explicit lifecycle and timeout semantics.
8. Every privileged operation is auditable.
9. Media queues optimize for freshness.
10. Performance measurements are part of feature completion, not an afterthought.

────────

58. Open Questions to Resolve During Phase 0

These questions should become ADRs or experiments rather than informal assumptions:

• Exact session capability encoding: compact binary, COSE, PASETO-like, or project-specific signed structure.
• Whether application E2E encryption is applied per QUIC datagram or at a higher framed layer. Section 61.2 states one candidate answer (encoded-frame-payload level, i.e. one AEAD operation per encoded frame before Section 21's packetizer fragments it), but Section 61 is a later, informally-reconciled addition to this spec — see the correction made to its Packet Loss Recovery row for a concrete case where a 61.x entry contradicted Sections 14/21/58. Do not treat the 61.2 entry as authoritative for this question until it is either cross-checked against Section 17 and frozen as an ADR, or Section 17 is updated directly. Natural home for that ADR: Section 17 (End-to-End Encryption).
• Native decoder API choice on Windows.
• Primary UI framework: Slint versus another native option.
• ~~Whether a STUN-compatible server is sufficient or a custom connectivity service is needed.~~ Resolved — ADR-019: custom reflexive discovery over the control-plane channel for v0.2, STUN-compatible interop deferred.
• Whether to use wgpu in the main client renderer or direct platform APIs initially.
• Exact H.264 packetization strategy and maximum datagram payload.
• Whether FEC belongs in v0.2 or later.
• Secure Desktop/UAC support model on Windows. Partially resolved: the privilege-context split (SYSTEM for Winlogon/pre-login, as-user in-session) is settled by ADR-021. Still open: client-side UX while the remote desktop is on the Secure Desktop, which cannot be captured at all (Section 19).
• ~~Unattended-access consent/notification policy.~~ Resolved — ADR-023: configurable per role/device via `SessionCapability.restrictions`, not one fixed global behavior. Chosen specifically because the spec's two named use cases (Section 1) have conflicting expectations here.

────────

59. Recommended Immediate Implementation Order

Do not start with the control plane UI or enterprise IAM.

Build in this order:

```text
1. Windows capture
        ↓
2. H.264 hardware encode
        ↓
3. QUIC video transport
        ↓
4. Client hardware decode + render
        ↓
5. Keyboard/mouse
        ↓
6. Relay
        ↓
7. Device identity/enrollment
        ↓
8. Signed session capability
        ↓
9. Control plane session broker
        ↓
10. Adaptive quality + reconnect
```

This sequence attacks the highest technical risk first: the interactive media pipeline.

────────

60. Final Architecture Statement

Nexus should be implemented as a new remote-access platform, not as a Teleport fork.

The guiding architecture is:

```text
Control Plane decides
Agent verifies
Endpoints encrypt
Relay forwards
Audit observes
```

Teleport-inspired ideas belong primarily in identity, access control, device enrollment, short-lived authorization, and auditing. The actual desktop engine—capture, encoding, input, QUIC transport, P2P, relay fallback, adaptive quality, and native rendering—should be owned and designed specifically for Nexus.

The first product milestone should prove one thing exceptionally well:

> A lightweight Windows agent and native Windows client can establish an authenticated, encrypted session and deliver responsive 1080p remote control through a relay, with a protocol and architecture that can later evolve to P2P and enterprise zero-trust access without being rewritten.

────────

Appendix A - Crate Responsibilities and Workspace Phase

|Crate                |Responsibility                                              |Introduced        |
|---------------------|------------------------------------------------------------|-------------------|
|`nexus-common`       |IDs, shared errors, time, configuration primitives          |Phase 0 (initial) |
|`nexus-crypto`       |Device keys, capability verification, session key derivation|Phase 0 (initial) |
|`nexus-protocol`     |Versioned wire/control schema                               |Phase 0 (initial) |
|`nexus-transport`    |QUIC connections, streams, datagrams, metrics               |Phase 0 (initial) |
|`nexus-session`      |Session state machine, reconnect semantics                  |Phase 0 (initial) |
|`nexus-auth`         |User/device authentication logic                            |Phase 0 (initial) |
|`nexus-policy`       |RBAC/ABAC evaluation                                        |Phase 0 (initial) |
|`nexus-audit`        |Audit event model and sinks                                 |Phase 0 (initial) |
|`nexus-codec`        |Encoder/decoder abstractions                                |Phase 0 (initial) |
|`nexus-capture`      |Platform-neutral capture traits                             |Phase 0 (initial) |
|`nexus-input`        |Semantic input model                                        |Phase 0 (initial) |
|`nexus-observability`|tracing, metrics, session quality telemetry                 |Phase 0 (initial) |
|`nexus-audio`        |Audio model and Opus pipeline                               |Phase 3 (v0.3)    |
|`nexus-file-transfer`|Chunking, resume, integrity                                 |Phase 3 (v0.3)    |

"Phase 0 (initial)" crates are the twelve members of the Cargo workspace from the start of implementation. `nexus-audio` and `nexus-file-transfer` are added to `Cargo.toml` when Phase 3 work begins, matching Sections 5, 27, and 28.

────────

Appendix B - Initial Process Ports

Example defaults; final values must be configurable.

|Service               |Port|Protocol                    |Purpose                    |
|----------------------|---:|----------------------------|---------------------------|
|`nexusd`              |443 |HTTPS/WSS                   |API, auth, signaling       |
|`nexus-relay`         |4433|UDP/QUIC                    |Relay data plane           |
|`nexus-relay` fallback|443 |TCP/TLS/QUIC where supported|Restricted-network fallback|
|STUN/connectivity (optional, deferred)|3478|UDP|Not used by the v0.2 baseline — reflexive-address discovery runs over the authenticated control-plane channel instead (ADR-019). Reserved only for a possible future STUN-compatible interop mode; do not open this port as part of the v0.2 build.|

────────

Appendix C - Minimum Pilot Test Matrix

Windows host OS

• Windows 10 22H2.
• Windows 11 current supported releases.
• Intel integrated GPU.
• NVIDIA discrete GPU.
• AMD discrete/integrated GPU where available.

Network

• Same LAN.
• Home NAT.
• Corporate NAT.
• UDP blocked.
• High latency.
• Packet loss.
• Wi-Fi roaming/reconnect.

Desktop scenarios

• Static office application.
• Scrolling web page.
• Video playback.
• Multi-monitor host.
• 100%, 125%, 150%, 200% DPI.
• US and Vietnamese keyboard layouts.
• Lock/unlock.
• Sleep/wake.
• UAC prompt behavior.

────────

61. Architecture Enhancements Roadmap: Short-Term vs. Long-Term

To bridge the gap between initial MVP execution speed and ultimate long-term technical excellence, the architecture defines two distinct implementation tiers: Short-Term (MVP Recommendations) and Long-Term (Target State / Best-in-Class Architecture).

61.1 Media & Graphics Pipeline

| Feature Category | Short-Term (MVP Recommendation) | Long-Term (Target State / Best-in-Class) |
|---|---|---|
| Color Space Conversion | D3D11 Video Processor (`ID3D11VideoProcessor`) RGBA8 -> NV12 conversion directly on VRAM. Zero-CPU copy. | Custom D3D11/D3D12 Compute Shader pipeline supporting NV12, P010 (10-bit), and RGB444 color formats. |
| Desktop Capture API | Primary Windows Graphics Capture (WGC); DXGI Desktop Duplication fallback (matches Sections 6 and 19 — corrected here for consistency). | Dynamic API switching + Custom Indirect Display Driver (IddCx) for headless virtual display creation up to 240Hz/4K. |
| UAC & Winlogon Handling | `WM_WTSSESSION_CHANGE` listener in Service; spawn dedicated `SYSTEM`-privileged host runner in `Winlogon` desktop for pre-login/unattended capture only — the in-session desktop-host runs as the interactive user (ADR-021). | Seamless Desktop Handle Migration with shared DXGI GPU context across desktop boundaries. |
| Color Depth & Display | 8-bit SDR (1080p / 4K @ 60fps). | 10-bit HDR (HDR10/Dolby Vision) with wide color gamut (Rec.2020) and dynamic client-side GPU upscaling (DirectML / WebGPU NIS/FSR). |

61.2 Transport, Network & Loss Recovery

| Feature Category | Short-Term (MVP Recommendation) | Long-Term (Target State / Best-in-Class) |
|---|---|---|
| Packet Loss Recovery | Unreliable QUIC datagrams per Sections 14/21 — no retransmission; a lost fragment simply drops, and the encoder/agent issues a keyframe on request (Section 20/22) rather than blocking newer frames on redelivery of an old one (corrected here for consistency with Sections 14, 21, 58 — a prior draft of this row proposed reliable Stream-per-Frame with `STREAM_RESET`/`CANCEL_STREAM`, which contradicts the datagram-based design used everywhere else in the spec). | Adaptive Fountain Codes / RaptorQ FEC with real-time loss model prediction (0ms retransmission latency overhead) — FEC is meaningful specifically because the underlying transport stays unreliable/datagram-based; it would be redundant on top of a reliable stream. |
| Application E2E Crypto | ChaCha20-Poly1305 / AES-GCM applied at the Encoded Frame Payload level (60 AEAD ops/sec). | Per-frame AEAD with hardware-accelerated AES-NI / ARM Crypto extensions + zero-copy payload slicing. |
| Network Traversal | ICE-inspired UDP P2P hole punching (parallel-raced against relay setup, ADR-018) with fallback to QUIC Relay nodes; reflexive discovery via ADR-019. | Multipath QUIC (MP-QUIC) across Wi-Fi + 5G/LTE simultaneous links + Anycast Blind Relay Mesh with sub-10ms failover. |

61.3 Security, Identity & Device Trust

| Feature Category | Short-Term (MVP Recommendation) | Long-Term (Target State / Best-in-Class) |
|---|---|---|
| Key Storage & Identity | Software Ed25519/X25519 keypairs stored in OS secure keychains (Windows Credential Manager). | TPM 2.0 / Secure Enclave hardware-bound keys with Platform Configuration Register (PCR) remote attestation. |
| Cryptographic Primitives | Classical TLS 1.3 + Ed25519 + X25519 + ChaCha20-Poly1305. | Quantum-Resistant Hybrid Crypto (X25519 + ML-KEM-768 / Kyber for KEM, Ed25519 + ML-DSA / Dilithium for Signatures). |
| Session Authorization | Short-lived signed capability tokens validated locally by Agent, narrowable in place via signed policy-update push (ADR-017) — the evolution path to the target state, not a separate architecture. | Continuous Zero-Trust Attestation (real-time posture re-evaluation, EDR integration, instant token revocation push). |

61.4 Input, Peripherals & Audio

| Feature Category | Short-Term (MVP Recommendation) | Long-Term (Target State / Best-in-Class) |
|---|---|---|
| Keyboard Injection | USB HID Scancode mapping via `SendInput(KEYEVENTF_SCANCODE)`. Unicode `WM_CHAR` fallback for IME/Text. | Kernel-level input filter driver / Virtual HID device for game anti-cheat compatibility and low-level system key interception. |
| Peripherals & Passthrough | Local relative/absolute cursor, clipboard text synchronization. | Virtual Bus Drivers (ViGEm/USB-over-IP) for Gamepads (haptics), Wacom pressure tablets, and YubiKey/PKCS#11 smart cards. |
| Audio Pipeline | WASAPI Loopback capture -> Opus codec -> QUIC datagrams (< 100ms latency). | WebRTC Audio Processing (AEC/NS) + 5.1/7.1 Surround Spatial Audio streaming. |

61.5 Concurrency Architecture & Rust System Design

| Feature Category | Short-Term (MVP Recommendation) | Long-Term (Target State / Best-in-Class) |
|---|---|---|
| Media Concurrency | Dedicated OS Native Thread (`std::thread`) for capture/encode loop; Tokio async runtime for networking. | Zero-allocation lock-free SPSC queues (`crossbeam`) + NUMA-aware thread pinning for GPU/Network IO. |
| Process IPC | Named Pipes with strict ACL security descriptor, plus verification of the connecting process's code signature/hash (ADR-020) — ACL alone does not stop a same-user process from opening the pipe. | Zero-copy Shared Memory Ring Buffers (`shm`) with event signal synchronization. |

────────

End of specification.