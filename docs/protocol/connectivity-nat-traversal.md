# Design Note — Connectivity & NAT Traversal

Status: Draft, open for review.
Related spec sections: 1, 2, 15, 16, 32, 46, 58, 59, 61.2, Appendix C.
Produced via a system-design review, following on from
`docs/protocol/session-establishment-signaling.md` and
`docs/protocol/session-authorization-model.md`.

## 1. Requirements

Connection preference order is fixed by the spec: same-LAN direct > P2P
UDP hole-punch > UDP relay > TCP/TLS relay (Section 15). v0.1 is
deliberately relay-only to de-risk the media pipeline first (Section 59:
relay comes before NAT traversal in the build order). Hard constraints:
"no inbound port required on the host" (Section 1) must hold even in
relay-only v0.1; first-frame targets are <1s LAN / <2s typical Internet
(Section 1), which bounds how long candidate selection is allowed to take;
relay must stay blind to content even when P2P falls back to it
(Principle 3.5).

## 2. High-level design

Section 15 defines the `Candidate` struct but not the actual v0.2 exchange
sequence. This note makes it explicit — a standard ICE-lite pattern:

```
Client                     nexusd (candidate exchange channel)               Agent
  |--- request session ---------------->|                                        |
  |                                    |--- signal: gather candidates ---------->|
  |                                    |<-- agent candidates (LAN, reflexive) ---|
  |<-- client gathers its own candidates (LAN, reflexive) ----------------------|
  |--- client candidates ------------->|                                        |
  |                                    |--- forward candidates both ways -------->|
  |===== both sides now hold each other's candidate list =====
  |--- try candidates in parallel, priority order; first success wins --------->|
  |========= QUIC connection established on the winning candidate ============|
```

## 3. Deep dive

**Reflexive-address discovery (resolves the open question in Section 58).**
Decision: a custom endpoint co-located with `nexusd`, reachable only over
the already-authenticated control-plane channel, rather than standing up a
separate unauthenticated STUN server for v0.2. This avoids adding a new
public attack surface and reuses trust already established for signaling.
A STUN-compatible mode can be layered in later for interop with existing
infrastructure if that becomes necessary. Recorded as ADR-019.

**Parallel racing, not sequential.** A "try P2P fully, then fall back to
relay" design risks blowing the <1–2s first-frame budget on networks where
P2P negotiation is slow or fails outright (symmetric NAT, blocked UDP).
Decision: start relay connection setup and P2P connectivity checks at the
same time from the beginning of CONNECTING; use whichever candidate
succeeds first, with a short grace window to prefer a P2P candidate over a
relay candidate that also succeeded around the same time. Recorded as
ADR-018.

**v0.1 → v0.2 must be additive, not a rewrite.** `POST /api/v1/sessions`
already returns a `candidates` array (Section 32); in v0.1 it holds exactly
one relay candidate. If the v0.1 client is written to always iterate this
array (rather than hardcoding "connect to the one relay candidate"), adding
LAN/REFLEXIVE candidate types in v0.2 is purely additive on the wire and on
the client. This is a concrete implementation guideline for Epic E
(Client) rather than a standalone ADR — flagged here so it's decided before
Phase 1 client code is written, not discovered during the v0.2 migration.

**TCP/TLS relay fallback** (Section 15 item 4, Appendix B) activates only
when UDP relay connection itself fails/times out at session start — not in
response to mid-session packet loss over an already-working UDP relay,
which is handled by adaptive quality (Section 22), not a transport swap.

**Reconnect after a candidate change** (network handoff mid-session,
Section 46): re-run a lightweight candidate exchange, reusing the existing
capability if `max_duration` hasn't elapsed and the session hasn't been
revoked. This is a direct, positive validation of ADR-014 (establishment
TTL separated from session duration): a reconnect long after `expires_at`
has passed is not a problem, because only `max_duration` governs an
already-ESTABLISHED session.

## 4. Scale and reliability

In v0.1 relay-only, 100% of session traffic rides the relay — relay
capacity planning (`max_sessions`, `bandwidth_limit_mbps`, Section 54)
matters more during the v0.1-only period than it will once P2P offloads
LAN/home-NAT traffic in v0.2. Recommend adding a candidate-type breakdown
(LAN / reflexive / relay-fallback) to the existing "P2P success ratio"
metric (Section 40) to verify empirically, post-v0.2, that P2P is actually
being preferred in practice.

## 5. Trade-off analysis

| Decision | Option A | Option B | Chosen |
|---|---|---|---|
| Reflexive discovery | Standard standalone STUN server | Custom endpoint over the authenticated control-plane channel | B (ADR-019) |
| P2P vs relay setup | Sequential (P2P first, then relay) | Parallel race, first success wins within a grace window | B (ADR-018) — protects the first-frame latency budget |
| v0.1 client candidate handling | Hardcode single relay connect | Always iterate a candidate list, even length 1 | B — implementation guideline, not a formal ADR |
| UDP-blocked fallback trigger | Detect and swap transport at any point | Detect only at session start | B — mid-session loss is an adaptive-quality problem, not a transport-swap problem |

Decisions recorded as ADR-018
(`docs/adr/ADR-018-parallel-p2p-relay-race.md`) and ADR-019
(`docs/adr/ADR-019-custom-reflexive-discovery.md`).
