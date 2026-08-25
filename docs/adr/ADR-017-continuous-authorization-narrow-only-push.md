# ADR-017: Continuous Authorization via Narrow-Only Policy-Snapshot Push

## Status

Accepted.

## Context

Spec Section 8 describes session authorization as a static policy snapshot
the agent enforces locally, re-checked only against an explicit revoke.
Spec Section 61.3 separately describes a long-term target of "Continuous
Zero-Trust Attestation (real-time posture re-evaluation, EDR integration,
instant token revocation push)". Read together, these read as two different
architectures rather than one that evolves — and Section 60's Final
Architecture Statement commits to a design that can evolve toward the
target state "without being rewritten."

This surfaced during a system-design review of the session authorization
model (`docs/protocol/session-authorization-model.md`). The MVP cannot
require the agent to check authorization live with the control plane per
operation — that violates Principle 3.4 (endpoint-verifiable authorization)
and the input-to-photon latency budget in Section 43. Device posture checks
and JIT approval are explicit Non-Goals for MVP (Section 2), so this
decision does not need to build continuous evaluation now — it needs to
make sure the MVP model doesn't have to be discarded when continuous
evaluation is eventually added.

## Decision

1. The agent continues to enforce a locally-held policy snapshot
   (`SessionCapability.restrictions`), exactly as MVP already requires.
2. The control plane may push an updated snapshot to an already-ESTABLISHED
   session at any time, over the same presence channel already used for
   revoke (Section 9), via a new signed message type,
   `session.policy_update`.
3. A `session.policy_update` may only **narrow** `restrictions` (e.g.
   disable clipboard, force a lower `max_resolution`, shorten
   `max_duration`). The agent must reject any update that would grant a
   permission not present in the original signed `permissions[]` from the
   `SessionCapability` the session was established with.
4. Widening a session's access always requires issuing a fresh
   `SessionCapability` and running the identity handshake again — it is
   never done via an in-place push.
5. Full revoke (session termination) remains a distinct operation from a
   narrowing update, using the mechanism already established for revoke
   (push over presence WS, with a heartbeat-interval poll as a fallback
   bound).

This makes "continuous zero-trust attestation" (Section 61.3) a later
extension of the same mechanism MVP already needs — event sources (posture
checks, EDR signals, role changes) feed into the control plane's decision
to push a narrower snapshot or a revoke, rather than requiring a new
live-approval architecture on the data path.

## Consequences

**Positive**
- No architecture change is needed to grow from "static snapshot" toward
  "continuous re-evaluation" — only new event sources feeding an existing
  push mechanism.
- The agent's trusted computing base stays small: it never runs an
  RBAC/ABAC engine, only signature verification and restriction narrowing.
- The cryptographic guarantee of `permissions[]` at issuance is preserved —
  a compromised or buggy control-plane push cannot silently escalate an
  active session's access.

**Negative / follow-up work**
- Agent-side session state must track "current effective restrictions"
  separately from "restrictions as originally issued," and must validate
  every incoming policy-update against the original `permissions[]` before
  applying it — this needs explicit unit tests (an update that tries to
  widen access must be rejected and audited as a suspicious event, not
  silently dropped).
- `nexus-protocol` needs the `session.policy_update` message defined
  (Section 33 Protobuf messages) before Phase 3 policy work begins.
- Spec Sections 8 and 12 have been updated to describe this mechanism and
  cross-reference this ADR.

## Alternatives considered

**Live per-operation approval** (agent asks the control plane before each
privileged action). Rejected: violates Principle 3.4 and the latency
budget in Section 43; also creates a hard availability dependency between
the agent and the control plane for the entire session duration, which the
current design deliberately avoids (see
`docs/protocol/session-establishment-signaling.md`, "nexusd is off the
critical path once CONNECTING starts").

**No extension point; treat MVP snapshot and long-term continuous
attestation as separate future architectures.** Rejected: contradicts the
Final Architecture Statement's explicit non-rewrite commitment (Section
60), and would leave the tension unresolved for whoever implements
Section 37/38 features in Phase 3+.

## References

Spec Sections 2, 3.4, 8, 9, 11, 12, 37, 38, 43, 60, 61.3.
`docs/protocol/session-authorization-model.md`.
