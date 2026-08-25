# ADR-014: Separate Session-Establishment TTL from Session-Duration Limit

## Status

Accepted — frozen before Phase 1 implementation, per Spec Section 51.

## Context

Spec Section 12 defines `SessionCapability` with a single `expires_at`
field (paired with `not_before`), described under "Properties" as the "TTL
for establishing a session: typically 30–120 seconds." The same section's
example `restrictions` block separately lists `max_duration = 30m`, which
describes how long the session itself may run once active.

These are two different temporal semantics that were not clearly separated:
does an ESTABLISHED/ACTIVE session get torn down when `expires_at` passes,
even though establishment already succeeded well inside the 30–120s window?
As written, an agent and a control plane implemented independently could
reasonably arrive at different interpretations.

This ambiguity surfaced during a system-design review of the session
establishment and signaling flow (see
`docs/protocol/session-establishment-signaling.md`, item b), done ahead of
Phase 1 so `nexus-session` (state machine) and `nexus-crypto` (capability
verification) are built against one agreed semantic instead of two guesses
that later have to be reconciled.

## Decision

1. `SessionCapability.expires_at` (with `not_before`) governs **only the
   establishment window**: the agent must receive and successfully
   validate the initial `SessionHello` carrying this capability before
   `expires_at`. Once the mutual identity proof completes and the session
   reaches ESTABLISHED, `expires_at` is no longer checked.
2. Session duration once ESTABLISHED is governed **exclusively** by the
   `max_duration` restriction (already present in the `restrictions`
   example in Section 12), together with explicit revoke (push over the
   presence channel, or the agent's periodic policy-snapshot re-check) and
   the reconnect-window rules in Section 46. The agent enforces
   `max_duration` from the ESTABLISHED timestamp, independent of
   `expires_at`.
3. Recommended defaults are unchanged: `not_before`..`expires_at` window of
   30–120 seconds for establishment; `max_duration` remains a separate,
   typically much longer, per-role/per-policy value set by the issuing
   policy.
4. No new wire field is introduced. `nexus-protocol`'s `SessionCapability`
   schema keeps `expires_at` and `restrictions.max_duration` as they are —
   this is a semantic clarification, not a schema change. The verifier
   implementation in `nexus-crypto` / `nexus-session` must implement the
   two-phase check explicitly rather than treating `expires_at` as a
   rolling session deadline.

## Consequences

**Positive**
- Removes the ambiguity that could cause a correctly-established,
  long-running session to be killed by a short establishment TTL.
- No breaking change to the `SessionCapability` wire schema.
- Establishment-TTL tuning (a network/reliability concern) and
  session-duration policy (a business/compliance concern) can now evolve
  independently.

**Negative / follow-up work**
- The agent-side capability verifier has two distinct temporal checks
  instead of one, adding a small amount of state-machine complexity.
- Must be covered under the Definition of Done for a protocol feature
  (Spec Section 56): version/capability interaction documented (this ADR),
  plus unit tests for the boundary cases — establishing right at
  `expires_at`, and session teardown right at `max_duration` after a
  reconnect.
- Spec Section 12 has been updated to state this two-phase semantic
  explicitly and cross-reference this ADR.

## Alternatives considered

**Single field reinterpreted as session end-time** (control plane re-signs
or extends `expires_at` at connect time to cover the whole session).
Rejected: it would require `expires_at` to become effectively long-lived
after establishment, undermining the short-lived, single-use establishment
property that the nonce/replay protection in Section 44 depends on.

**New dedicated field** (e.g. `establishment_expires_at`, leaving
`expires_at` to mean session end). Rejected for MVP: unnecessary wire-schema
churn when reinterpreting the two existing fields (`expires_at` for
establishment, `restrictions.max_duration` for session length) already
covers the requirement without touching `SessionHello`/`SessionCapability`
structure.

## References

Spec Sections 8, 12, 13, 44, 46. `docs/protocol/session-establishment-signaling.md`.
