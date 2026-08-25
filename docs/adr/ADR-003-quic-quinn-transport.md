# ADR-003: QUIC via Quinn as the Primary Data-Plane Transport

## Status

Accepted — retroactively frozen. Already implemented (`quinn` is a
workspace dependency in `Cargo.toml`, currently unused pending Phase 0's
QUIC PoC) and stated in the spec (Section 6, Section 14); surfaced as
unwritten during an architecture consistency audit (see
`docs/IMPLEMENTATION_STATUS.md` §3). This ADR directly gates the Phase 0
QUIC proof-of-concept deliverable (Section 48), so it needs to be frozen
before that PoC work proceeds.

## Context

Section 14 makes QUIC the primary data-plane transport: reliable streams
for control/input/clipboard, unreliable datagrams for video/audio/cursor.
Section 6 names Quinn as the QUIC crate and rustls for TLS 1.3. This
directly underlies the transport correction made earlier in this review
(Section 61.2's Packet Loss Recovery row, corrected to match the
datagram-based design in Sections 14/21) and the P2P/relay racing behavior
already frozen in ADR-018.

## Decision

QUIC, via the `quinn` crate (rustls backend), is the transport for all
data-plane connections: client↔agent (direct or via relay) and
endpoint↔relay. Reliable QUIC streams carry session control, keyboard/mouse
input, clipboard, file-control metadata, and diagnostics. Unreliable QUIC
datagrams carry video, audio, and cursor-position packets (Section 14,
Section 21). QUIC's mandatory TLS 1.3 handshake provides transport-level
security; application-level E2E encryption (ADR-006) layers on top so a
relay forwarding QUIC traffic cannot decrypt content (Principle 3.5).

## Consequences

**Positive**
- Built-in connection migration helps the reconnect/roaming scenarios in
  Section 46 and Appendix C's "Wi-Fi roaming/reconnect" test case, without
  Nexus having to build its own connection-migration logic.
- 0-RTT/1-RTT handshake speed directly serves the first-frame latency target
  (Section 1: <1s LAN, <2s typical Internet).
- Datagram support is exactly what Section 14/21's packet design assumes —
  no additional unreliable-transport layer needs to be built on top of a
  reliable protocol.

**Negative / follow-up work**
- QUIC/Quinn is a less battle-tested stack in production than plain TCP/TLS;
  the whole team needs to internalize QUIC's connection/stream/datagram
  model, which is unfamiliar compared to a socket-per-stream mental model.
- UDP is blocked on some restrictive networks (corporate firewalls, some
  mobile carriers), requiring the TCP/TLS relay fallback (Section 15 item 4)
  as a genuinely separate code path that must be maintained and tested
  (Appendix C: "UDP blocked" test case) even though it is not the primary
  path.

## Alternatives considered

**Raw UDP with a custom reliability/congestion-control layer.** Full control
over wire format and loss-recovery behavior. Rejected: reinvents congestion
control, connection establishment, and loss detection that QUIC already
provides as a mature, standardized implementation, and forgoes TLS 1.3
integration "for free" — a strictly worse cost/benefit trade for no
architectural gain.

**WebRTC / SRTP.** Purpose-built for real-time media, wide interop.
Rejected: Section 2 explicitly lists "Full WebRTC compatibility" as a
non-goal for MVP. WebRTC's ICE/SDP/DTLS-SRTP stack is designed for
browser interop that Nexus doesn't need, since Nexus controls both
endpoints (agent and client) directly — adopting it would import
substantial complexity (SDP negotiation, a separate DTLS handshake) to
solve an interop problem this product doesn't have.

**Plain TCP/TLS as the primary transport (not just the fallback).**
Simpler, universally available. Rejected: TCP's head-of-line blocking
across all streams is fundamentally incompatible with Principle 3.2
("interactive freshness over perfect delivery") — a single lost packet
would stall delivery of every subsequent frame, which is precisely the
failure mode Section 14's datagram design exists to avoid. TCP/TLS remains
the last-resort fallback (Section 15 item 4) for UDP-blocked networks only.

## References

Spec Sections 1, 2, 3.2, 6, 14, 15, 21, 46, Appendix C. ADR-002, ADR-006,
ADR-018.
