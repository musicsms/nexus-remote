# Design Note — Session Establishment & Signaling Flow

Status: Draft, open for review.
Related spec sections: 8, 9, 12, 13, 40, 46, 50, 51.
Produced via a system-design review of the flow before Phase 1 implementation.

This note extends Spec Section 13 with explicit handling for the failure
and edge cases the base diagram doesn't cover. It does not replace the
spec — it's the working analysis behind ADR-014 and the other open items
listed at the end.

## 1. Requirements recap

Functional: client requests a session → control plane signals the agent →
control plane issues a signed `SessionCapability` → client connects to the
agent via a candidate/relay → mutual identity proof → E2E encrypted session.
Must degrade correctly when the agent is offline, the capability expires
mid-handshake, the session is revoked while active, or the relay dies.

Non-functional (from the spec, binding): capability establishment TTL
30–120s (Section 12); state transitions idempotent and auditable (Section
13); the agent verifies capabilities independently of the control plane per
operation (Principle 3.4); relay stays blind to content (Principle 3.5);
presence heartbeat 15–30s, <1KB/s idle (Section 9); reconnect window
30–120s (Section 46).

Constraints: v0.1 is relay-only (no P2P yet); `nexusd` is a single-instance
modular monolith (no control-plane HA yet); SQLite is the default DB
(ADR-013).

## 2. High-level flow (extended)

```
Client                    nexusd (Control Plane)                 Agent
  |                              |                                  |
  |--POST /sessions------------->|                                  |
  |                              |--[agent presence WS connected?]  |
  |                              |     NO -> fail-fast DENIED------>| (skip; agent unreachable)
  |                              |     YES                          |
  |                              |--signal: session.request-------->|
  |                              |<--agent: accept/deny-------------|
  |<--signed SessionCapability---|   (or push: capability.revoked   |
  |    + relay candidate         |    at any later point)           |
  |                              |                                  |
  |===== CONNECTING: client uses the capability to reach the agent  =====
  |===== directly — nexusd does not need to stay alive from here on =====
  |----------- QUIC via candidate/relay -------------------------------------------------->|
  |<---------------- agent identity proof (signed with device key) ------------------------|
  |---------------- client identity proof ---------------------------------------------------|
  |========= E2E encrypted session established =========
  |                              |                                  |
  |   (if the relay dies)                                           |
  |<====== reconnect: same session_id, new candidate, fresh keyframe ======>|
  |                              |                                  |
  |   (revoke at any time: push over presence WS, or agent's own    |
  |    periodic policy-snapshot re-check at the next heartbeat)     |
```

The property worth calling out explicitly: once the client holds a
capability and a candidate, `nexusd` is off the critical path to reach
ESTABLISHED. A control-plane restart mid-CONNECTING should not break an
in-flight handshake — this should be a stated requirement, not an implicit
side effect of Principle 3.4.

## 3. Edge cases and resolutions

**(a) Agent offline when the client requests a session.** Check presence
state before entering REQUESTED. If the agent has no open presence WS,
return `DENIED` (reason: agent offline) immediately instead of waiting out
the full establishment TTL.

**(b) Capability expires while still CONNECTING.** Resolved by ADR-014:
`expires_at`/`not_before` govern only the establishment window (agent must
receive `SessionHello` before `expires_at`); once ESTABLISHED, only
`max_duration` (plus revoke/reconnect rules) governs how long the session
runs. Without this split, a long ACTIVE session could read as "capability
expired" under a naive single-field interpretation.

**(c) Revoke while ACTIVE.** Two layers: (1) push — `capability.revoked`
sent over the agent's open presence WS, near-instant; (2) poll — the agent
re-checks its cached policy snapshot every heartbeat (15–30s) regardless of
WS state, as a bound in case the push is lost. Document the resulting SLA
explicitly: best case near-instant, worst case ≤ one heartbeat interval.

**(d) Relay dies mid-session.** The relay heartbeats into `relay_nodes`
(Section 34); `nexusd` excludes a missed-heartbeat relay from candidates for
*new* sessions. For a session already running on that relay, both endpoints
detect the QUIC loss and drive the existing reconnect flow (Section 46):
same `session_id`, re-prove identity, request a new candidate, fresh
keyframe on reconnect. If reconnect doesn't complete inside the reconnect
window, transition to DISCONNECTED → ENDED rather than hanging indefinitely.

**(e) Duplicate session requests** (double-click, client retry after a
network blip). `POST /api/v1/sessions` has no idempotency key today.
Recommend the client send a self-generated `idempotency_key`; `nexusd`
dedupes within a short window before creating a new session row.

**(f) Concurrent sessions to the same target device.** Not specified today.
Recommend, for MVP: `desktop.control` is exclusive per target device (a
second control request is denied while one is ACTIVE); `desktop.view` may
allow multiple viewers. This is a policy decision, not something to leave
implicit in code — worth its own ADR before Epic F (control-plane) lands.

**(g) Protocol-version downgrade during handshake.** Because the agent
already reports its capability set over the presence channel (Section 9),
`nexusd` knows the agent's protocol range at the moment it issues the
`SessionCapability`. That range can be bound into the capability so the
client/agent handshake negotiation is constrained to it, closing the
MITM-forced-downgrade gap in the current mutual-identity-proof step. Also
worth its own ADR (touches ADR-005/ADR-010).

**(h) Clock skew.** Capability validation depends on the agent's local
clock matching `not_before`/`expires_at`. State an explicit tolerance (e.g.
±5s) rather than assuming perfect NTP sync.

## 4. Scale and reliability notes

Single-instance `nexusd` holds presence WS connections in memory — a
restart drops them. Per Section 2, this is unaffected: ESTABLISHED/ACTIVE
sessions don't depend on `nexusd`, so only *new* session requests during the
restart window fail-fast until the agent's WS reconnects. Acceptable for
MVP self-host; multi-instance signaling (pub/sub for revoke, sticky WS) is
explicitly out of scope until HA is revisited (Section 2 Non-Goals).

Metrics worth adding beyond Section 40: signaling round-trip time
(REQUESTED → AUTHORIZED), actual revoke-propagation latency, relay-failover
time, and rate of capability-expired-during-CONNECTING (a signal that the
30–120s TTL may be too tight for real network conditions).

## 5. Open items spun out as their own ADRs

- **ADR-014** (written): split establishment TTL from session duration —
  see `docs/adr/ADR-014-session-capability-ttl-semantics.md`.
- **ADR-015** (written): exclusive-vs-shared concurrent session policy
  (item f above) — see
  `docs/adr/ADR-015-concurrent-session-policy-per-device.md`.
- **ADR-016** (written): binding the agent's advertised protocol range into
  the session capability to close the downgrade gap (item g above) — see
  `docs/adr/ADR-016-bind-agent-protocol-range-into-capability.md`.
