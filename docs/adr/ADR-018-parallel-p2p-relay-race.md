# ADR-018: Race P2P and Relay Connection Setup in Parallel

## Status

Accepted — applies starting v0.2 Connectivity (Phase 2).

## Context

Spec Section 15 defines a connection preference order (LAN > P2P reflexive
> UDP relay > TCP/TLS relay) but does not specify whether candidates are
tried sequentially or in parallel during CONNECTING. Section 1 sets a hard
target of first-frame <1s on LAN and <2s on typical Internet.

This was flagged during a system-design review of connectivity and NAT
traversal (`docs/protocol/connectivity-nat-traversal.md`): a naive
sequential design — attempt P2P hole-punching to completion or timeout,
then fall back to relay — risks missing the first-frame budget whenever
P2P negotiation is slow or ultimately fails (symmetric NAT, blocked UDP,
restrictive firewalls), since the full P2P attempt duration would be added
in front of the relay path on every such case.

## Decision

Starting a session's CONNECTING phase begins relay connection setup and
P2P connectivity checks at the same time, not sequentially. Whichever
candidate completes a successful QUIC handshake first is used. If a P2P
candidate and a relay candidate both succeed within a short grace window
of each other, the P2P candidate is preferred (per the Section 15
preference order) and the relay connection attempt is aborted. If only the
relay candidate succeeds (or succeeds well before any P2P candidate), the
session proceeds on relay without waiting further for P2P.

The exact grace-window duration is an implementation/tuning parameter, not
frozen by this ADR — it should be measured against the first-frame budget
during Phase 2 network-simulation testing (Section 47) rather than guessed
now.

## Consequences

**Positive**
- Protects the first-frame latency target even on networks where P2P is
  slow or fails, since relay is never blocked behind a full P2P attempt.
- No behavior change needed for v0.1 (relay-only): with no P2P candidates
  to race against, the relay candidate simply wins immediately, so this
  ADR is forward-compatible with the current v0.1 implementation.

**Negative / follow-up work**
- Slightly higher resource use during CONNECTING (both paths are attempted
  concurrently instead of one at a time) — bounded by the short duration of
  the establishment phase (Section 12), not a sustained cost.
- Needs an explicit test: a P2P candidate that succeeds just after the
  relay candidate but within the grace window must still cause a switch to
  P2P, not get discarded because relay "won" first. This should be one of
  the Phase 2 integration tests (Section 47).

## Alternatives considered

**Sequential — attempt P2P to completion/timeout, then fall back to
relay.** Rejected: directly risks the first-frame latency target (Section
1) on exactly the networks (symmetric NAT, blocked UDP — see Appendix C
test matrix) where P2P is most likely to fail or take longest, which is
also where users most need the relay fallback to be fast.

**Relay-first, only attempt P2P as a post-connection upgrade.** Considered
but rejected for v0.2: adds a mid-session transport-migration mechanism
(swap from relay to P2P after the session is already ACTIVE) that isn't
otherwise needed yet, and would require careful handling to avoid visible
glitches during the swap. May be revisited later if relay cost/latency at
scale makes it worthwhile — not blocking for v0.2.

## References

Spec Sections 1, 15, 16, 47, 59. `docs/protocol/connectivity-nat-traversal.md`.
