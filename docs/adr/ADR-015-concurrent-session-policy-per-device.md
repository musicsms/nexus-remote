# ADR-015: Concurrent-Session Policy Per Target Device

## Status

Accepted.

## Context

Spec Section 11 defines `desktop.view` and `desktop.control` as separate
first-class authorization actions, but neither Section 13 (session
establishment/state machine) nor Section 34 (database model) specifies what
happens when a second session request targets a device that already has an
ACTIVE session. Two independently-built control-plane implementations could
reasonably diverge: one rejecting any second request outright, another
allowing unlimited concurrent controllers with undefined simultaneous input
behavior.

This gap was identified during a system-design review of the session
establishment and signaling flow
(`docs/protocol/session-establishment-signaling.md`, item f) and is a real
dependency for the video/media pipeline review
(`docs/protocol/video-media-pipeline.md`), which notes that true multi-viewer
fan-out (one capture+encode pipeline serving N viewer sessions) has "a real
architectural fork" blocked on this policy decision.

## Decision

1. `desktop.control` is exclusive per target device. While a session holding
   `desktop.control` is ACTIVE for a given `target_device_id`, a new session
   request for `desktop.control` on the same device is denied
   (`DENIED`, reason: device already under control) until the existing
   session ends (ENDED/DISCONNECTED past the reconnect window, Section 46) or
   is explicitly revoked.
2. `desktop.view` may be granted concurrently to multiple sessions on the
   same target device, independent of whether a `desktop.control` session is
   also active. This supports the Teleport-like review/observe use case
   (Section 1, Section 38's JIT approval flow) without requiring exclusive
   access.
3. Enforcement is a control-plane check, at session-request time, before a
   `SessionCapability` is issued — not something the agent enforces. This is
   consistent with ADR-017: the agent never runs RBAC/ABAC evaluation, only
   signature verification and restriction narrowing.
4. The check-and-create must be race-safe: two near-simultaneous
   `desktop.control` requests for the same device must not both succeed. This
   requires a DB-level constraint or an equivalent transactional check (the
   exact mechanism is an implementation detail for Epic F, to be chosen once
   the `sessions` table's representation of granted permissions is finalized
   — Section 34 does not yet show a directly-indexable `desktop.control`
   column — and must stay portable between SQLite and PostgreSQL per
   CLAUDE.md §5), not an application-level check-then-act.
5. This ADR does not mandate a specific video-pipeline fan-out architecture.
   MVP/early multi-viewer support may start with one independent
   capture+encode pipeline per viewer session (simpler, correct, but not
   GPU-efficient) and revisit true fan-out as a performance optimization once
   `desktop.view`-only sessions are actually used in practice (Phase 2+) —
   that remains an open implementation question for Epic B/C, not frozen
   here.

## Consequences

**Positive**
- Closes an authorization gap that is relevant even to Phase 1 MVP (a second
  control request must be rejected deterministically), not just to future
  multi-viewer scenarios.
- Unblocks the video-pipeline note in
  `docs/protocol/video-media-pipeline.md`: the authorization policy is now
  defined even though the fan-out implementation is deliberately left open.
- Matches the existing pattern in Section 11's example policy (separate
  `desktop.view`/`desktop.control` actions) rather than introducing a new
  authorization primitive.

**Negative / follow-up work**
- The control plane needs an explicit, tested race-safe check for concurrent
  `desktop.control` requests — must be covered by an integration test (two
  simultaneous control requests for one device; exactly one succeeds) under
  Epic F, per the Definition of Done (Section 56) and Section 47's
  integration-test list.
- `session_events`/`audit_events` (Section 35) should record a denied
  concurrent-control attempt as a distinct event type
  (`session.deny` with reason `device_already_controlled`) so contention is
  visible in audit logs, not just a generic denial.
- Spec Section 13 has been updated to state the exclusivity/DENIED rule
  directly and cross-reference this ADR.

## Alternatives considered

**Exclusive for both `desktop.control` and `desktop.view` (one session of
any kind at a time).** Rejected: unnecessarily restrictive for the
Teleport-like use case, where an approver or auditor reasonably wants to
observe a session without taking control (Section 38's JIT flow implies
review scenarios that need concurrent view access).

**No exclusivity check at all; rely on simultaneous input injection being
self-evidently confusing to users.** Rejected: silently allowing two
concurrent `desktop.control` sessions creates an accountability gap
(Section 35 — unclear which session's actions are whose) and undefined
behavior at the input-injection layer, not just a UX problem.

## References

Spec Sections 11, 13, 34, 35, 38, 50.
`docs/protocol/session-establishment-signaling.md` (item f),
`docs/protocol/video-media-pipeline.md` (multi-viewer fan-out note).
ADR-017.
