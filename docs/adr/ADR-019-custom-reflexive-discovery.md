# ADR-019: Custom Reflexive-Address Discovery Over the Control-Plane Channel

## Status

Accepted — applies starting v0.2 Connectivity (Phase 2). Resolves the open
question in Spec Section 58: "Whether a STUN-compatible server is
sufficient or a custom connectivity service is needed."

## Context

v0.2 NAT traversal (Spec Section 15) needs a way for a client or agent to
learn its own reflexive (public-facing) address and port as observed from
outside its NAT, which is a prerequisite for P2P candidate exchange
(Section 15, ICE-like candidate model). The spec left open whether to run a
standard STUN server or something custom (Section 58).

This was resolved during a system-design review of connectivity and NAT
traversal (`docs/protocol/connectivity-nat-traversal.md`). A standalone
STUN server is a new, typically unauthenticated, public UDP service — extra
attack surface and extra infrastructure to run and secure, for a capability
that only ever needs to be used by devices already authenticated to
`nexusd` over the presence channel (Section 9).

## Decision

For v0.2, reflexive-address discovery is a request/response exchanged over
the same authenticated control-plane channel already used for presence and
signaling — the client or agent asks `nexusd` (or a component behind it)
what address/port the request was observed from, reusing the existing
transport and authentication rather than a separate STUN listener.

A standard STUN-compatible mode is not ruled out permanently — it may be
added later if interop with existing STUN infrastructure becomes valuable
(e.g. for third-party client compatibility) — but is not required for v0.2
and is not part of this decision's scope.

## Consequences

**Positive**
- No new unauthenticated public service to expose, harden, and operate.
- Reuses the trust and transport already established for presence/signaling
  (Section 9) — one fewer moving part in the v0.2 connectivity stack.
- Keeps candidate gathering (Section 15's `Candidate` model) entirely
  within the control-plane API surface already being built for session
  signaling.

**Negative / follow-up work**
- Not directly interoperable with third-party STUN clients/tooling out of
  the box; if that's ever needed, a STUN-compatible listener would be added
  alongside this mechanism, not instead of it.
- Reflexive discovery now depends on the control-plane connection being up,
  same as the rest of signaling — no new availability dependency beyond
  what session establishment already requires.

## Alternatives considered

**Standalone STUN server (RFC 5389-compatible).** Rejected for v0.2:
standard and interoperable, but adds a new unauthenticated network service
for a capability only Nexus's own authenticated endpoints need right now.
Revisit if third-party interop becomes a real requirement.

**Third-party public STUN servers.** Rejected: acceptable for
prototyping, but unsuitable for a self-hosted product where an external
dependency for a core connectivity function conflicts with the
self-hostability goal (Section 1).

## References

Spec Sections 1, 9, 15, 58. `docs/protocol/connectivity-nat-traversal.md`.
