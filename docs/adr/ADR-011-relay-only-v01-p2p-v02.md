# ADR-011: Relay-Only v0.1, P2P Connectivity in v0.2

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 15,
Section 48, Section 59) and the explicit prerequisite ADR-018 and ADR-019
already build on; surfaced as unwritten during an architecture consistency
audit (see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 15 states: "v0.1 may intentionally support relay-only connectivity.
This reduces risk while the media pipeline is stabilized." Section 48
scopes Phase 1 (v0.1) to relay-only QUIC and Phase 2 (v0.2) to candidate
discovery, hole punching, and P2P QUIC. Section 59's recommended
implementation order places relay (#6) well before device
identity/enrollment and the session broker, and NAT traversal is not in the
first-ten-items list at all — it's implicitly v0.2 work. ADR-018 (parallel
P2P/relay race) and ADR-019 (custom reflexive discovery) already assume and
build on this v0.1/v0.2 split.

## Decision

Phase 1 (v0.1) ships with connectivity restricted to relay-only: the dumb,
stateless, horizontally-scalable relay described in Section 16 is the only
connection path. Direct P2P connectivity (same-LAN direct candidates,
Internet UDP hole-punching) is deferred to Phase 2 (v0.2), where it is
implemented per ADR-018 (parallel racing against relay, not sequential) and
ADR-019 (reflexive discovery over the authenticated control-plane channel,
not a standalone STUN server).

## Consequences

**Positive**
- De-risks the highest-uncertainty parts of the project independently: media
  pipeline correctness first (Phase 0/1 — capture, encode, transport,
  decode), connectivity sophistication second (Phase 2) — directly matching
  Section 59's stated risk-ordering rationale.
- The `Candidate` model and the `candidates` array returned by
  `POST /api/v1/sessions` (Section 15, Section 32) are already
  forward-compatible: a v0.1 client that iterates the array (even when it
  holds exactly one relay candidate) requires no rewrite when v0.2 adds
  LAN/reflexive candidate types — this is a concrete implementation
  guideline already captured in
  `docs/protocol/connectivity-nat-traversal.md`.
- Phase 0's exit condition ("capture a Windows desktop and stream frames
  between two local processes") and Phase 1's MVP don't need NAT traversal
  to work at all, keeping the first end-to-end demo unblocked by the
  hardest connectivity problem.

**Negative / follow-up work**
- v0.1 carries 100% of session traffic over the relay, including
  same-LAN/same-network sessions that P2P would otherwise handle directly —
  the worst case for the low-latency goal (Section 1), and higher relay
  bandwidth cost than v0.2 will have. Explicitly accepted as a deliberate,
  temporary trade-off.
- Relay capacity planning (`max_sessions`, `bandwidth_limit_mbps`, Section
  54) matters more during the v0.1-only period than after v0.2 ships P2P
  offload, per the scale notes in
  `docs/protocol/connectivity-nat-traversal.md`.

## Alternatives considered

**Build P2P/NAT traversal first, relay as a fallback added later.**
Rejected: directly contradicts Section 59's build order, and NAT traversal
correctness (reflexive discovery, hole-punching across symmetric NATs,
Appendix C's NAT test matrix) is a substantial, separately-testable problem
that would block the very first end-to-end demo unnecessarily — Phase 0's
exit condition doesn't need networking beyond loopback, let alone NAT
traversal.

**Ship both relay and P2P simultaneously in v0.1.** Rejected: doubles the
connectivity surface (relay protocol + P2P candidate exchange +
hole-punching) that must be validated before the media pipeline itself
(capture/encode/transport/decode) is proven, increasing the risk that Phase
1 slips while two hard problems are debugged at once instead of one.

## References

Spec Sections 1, 15, 16, 32, 48, 54, 59, Appendix C. ADR-018, ADR-019.
`docs/protocol/connectivity-nat-traversal.md`.
