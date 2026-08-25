# Design Note — Session Authorization Model

Status: Draft, open for review.
Related spec sections: 8, 10, 11, 12, 37, 38, 61.3.
Produced via a system-design review, following on from
`docs/protocol/session-establishment-signaling.md`.

## 1. Requirements

Two distinct concerns are both labeled "authorization" in the spec and need
separating: (a) authentication — proving who a user/device is (Section 10,
already reasonably settled: password+TOTP now, OIDC/SAML/WebAuthn later),
and (b) authorization — deciding what a session may do, and how long that
decision holds without being re-checked.

The tension this note resolves: Section 8 describes a static policy
snapshot the agent enforces locally ("continue to verify already-established
sessions according to the session policy snapshot unless the session is
explicitly revoked"), while Section 61.3 describes a long-term target of
"Continuous Zero-Trust Attestation (real-time posture re-evaluation, EDR
integration, instant token revocation push)". Device posture checks and JIT
approval are explicit Non-Goals for MVP (Section 2), so continuous
evaluation isn't required now — but Section 60's Final Architecture
Statement commits to an architecture that "can later evolve ... without
being rewritten," so the two models need a defined relationship now, not
just at v0.5.

Hard constraint that must not be violated: the agent cannot depend on a
live round-trip to the control plane per operation (Principle 3.4) — that
would blow the input-to-photon latency budget (Section 43).

## 2. High-level design

Keep the MVP point-in-time signed-capability model as-is (agent verifies
locally, doesn't need the control plane reachable mid-session). Add one
extension point: the control plane may push an updated, narrower policy
snapshot to an already-ESTABLISHED session over the same presence channel
used for revoke.

```
Control Plane                                   Agent (holds ACTIVE session)
     |                                                  |
     |--- initial SessionCapability (policy snapshot, T0) --->|  [v0.1: the only grant]
     |                                                  |
     |--- (v0.3+) periodic/event-driven snapshot refresh
     |    (posture change, role change, EDR alert) --------------------------->|
     |                                                  |   agent applies new restrictions
     |                                                  |   to the ALREADY-ESTABLISHED session
     |--- explicit revoke (push over presence WS) ----------------------------->|  ends session
```

"Continuous zero-trust" becomes "periodic narrow-only policy push over the
channel MVP already has to build for revoke" — not a new live-approval
architecture.

## 3. Deep dive

- `SessionCapability.restrictions` (Section 12) is already a policy
  snapshot (clipboard, file_transfer, recording, max_resolution,
  max_duration). A new signed message type, `session.policy_update`, lets
  the control plane replace it on a live session.
- **Narrow-only, enforced by the agent**: an update may only remove or
  tighten restrictions, never grant a permission outside the original
  signed `permissions[]`. Widening access requires a fresh capability and a
  fresh identity handshake — `permissions[]` is part of what the original
  signature covers, and a silent grant would break that guarantee.
- RBAC/ABAC (Section 11) evaluation stays entirely on the control-plane
  side, at issuance time and whenever a policy-update is triggered. The
  agent never runs an ABAC engine — it only verifies signatures and applies
  restriction narrowing locally. This keeps the host-side trusted computing
  base small.
- Device Trust/Posture (Section 37) becomes an ABAC input on the
  control-plane side: a failed posture check triggers either a
  `session.policy_update` (narrow) or a revoke, using the mechanism above —
  no new transport or trust model needed.
- JIT Access (Section 38) fits cleanly: approval only affects what's
  authorized *before* a capability is issued. It doesn't need to touch the
  live-session semantics defined here.

## 4. Scale and reliability

Shares fate and reliability characteristics with the revoke mechanism
already designed in `session-establishment-signaling.md` (push over
presence WS + heartbeat-interval poll as a bound). No new scaling concern
for MVP. Real-time EDR signal ingestion into policy evaluation is a v0.5+
concern (Section 61.3) and is out of scope here.

## 5. Trade-off analysis

| Decision | Option A | Option B | Chosen |
|---|---|---|---|
| Where authorization is (re-)evaluated | Live round-trip per operation | Point-in-time at issuance, agent enforces locally | B — required by the latency budget (Principle 3.4, Section 43) |
| How "continuous" enters later | New live-approval architecture (rewrite) | Periodic narrow-only snapshot push over the existing presence channel | B — no rewrite, reuses the revoke mechanism |
| Can a push widen permissions | Yes | No — widening requires a new capability | B — preserves the cryptographic binding of `permissions[]` |

Decision recorded as ADR-017: `docs/adr/ADR-017-continuous-authorization-narrow-only-push.md`.
