# ADR-005: Signed Capability-Based Session Authorization

## Status

Accepted — retroactively frozen. Already deeply implemented in the spec's
design (Section 12, refined further by ADR-014/015/016/017) and stated as
Principle 3.4; surfaced as unwritten during an architecture consistency
audit (see `docs/IMPLEMENTATION_STATUS.md` §3). Recorded now specifically
because ADR-016 already cites it as a decision it builds on.

## Context

Principle 3.4 (endpoint-verifiable authorization) requires the host agent to
independently verify a signed session capability; a compromised relay must
not grant access, and the data plane must not rely on a central server
approval check for every operation. Section 12 defines the
`SessionCapability` structure this principle is implemented through. Section
60's Final Architecture Statement summarizes the resulting model in one
line: "Control Plane decides / Agent verifies / Endpoints encrypt / Relay
forwards / Audit observes."

## Decision

Session authorization is represented as a short-lived, signed
`SessionCapability` (Section 12) issued once by the control plane at session
request time and verified independently by the agent using the control
plane's signing public key — never as a live, per-operation approval
round-trip to `nexusd`. The capability binds `subject_user_id`,
`client_device_id`, `target_device_id`, `permissions[]`, `restrictions`, a
validity window, and a nonce, and the agent rejects any capability that
fails signature verification, is expired, targets the wrong device, or
replays a previously-seen nonce.

## Consequences

**Positive**
- Keeps the agent's trusted computing base small: it verifies a signature
  and applies restrictions, it never runs an RBAC/ABAC engine itself — this
  is the property ADR-017 later builds "continuous authorization" on top of
  without an architecture rewrite.
- Works even if `nexusd` becomes transiently unreachable after the
  capability is issued (`docs/protocol/session-establishment-signaling.md`:
  "nexusd is off the critical path once CONNECTING starts") — sessions don't
  have a hard runtime dependency on control-plane availability.
- A compromised or malicious relay cannot forge authorization, since it
  never holds the control plane's signing key and the agent verifies
  locally (Principle 3.5, relay blindness).

**Negative / follow-up work**
- Revocation before a capability's natural expiry is not instantaneous by
  construction — it requires an active push over the presence channel or the
  agent's periodic policy-snapshot re-check (already documented in
  `docs/protocol/session-establishment-signaling.md`'s revoke SLA
  discussion: near-instant best case, one heartbeat interval worst case).
- The control-plane signing key becomes a critical, high-value security
  dependency — key rotation and, where feasible, separation of signing keys
  from other control-plane secrets is required (Section 44 threat model:
  "Control-plane compromise").

## Alternatives considered

**Live per-operation approval** (agent calls back to `nexusd` before every
privileged action). Rejected: violates Principle 3.4 directly and the
input-to-photon latency budget (Section 43), and creates a hard availability
dependency between the agent and control plane for the entire session
duration — the same reasoning ADR-017 later restates when it extends this
model toward continuous authorization.

**Opaque bearer/session tokens validated by control-plane lookup**, like a
traditional session cookie. Rejected: requires the agent to reach `nexusd`
synchronously to validate every session, reintroducing the live-dependency
problem above, and doesn't naturally express a specific, cryptographically
bound authorization decision (permissions, target device, TTL) the way a
signed capability does.

**Mutual TLS client certificates alone**, with no separate capability
structure. Rejected: a client certificate authenticates a device/identity —
it says nothing about what that identity is authorized to do in this
specific session, conflating authentication (Section 10) with authorization
(Section 11) into a single artifact.

## References

Spec Sections 10, 11, 12, 43, 44, 60. ADR-014, ADR-015, ADR-016, ADR-017.
