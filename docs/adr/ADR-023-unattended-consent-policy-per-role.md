# ADR-023: Unattended-Access Consent/Notification Is a Per-Role/Per-Device Policy Setting

## Status

Accepted. Resolves the open question in Spec Section 58:
"Unattended-access consent/notification policy."

## Context

Spec Section 58 left open whether the local user (if present) is notified
when an unattended remote session starts and controls their desktop. This
was raised explicitly during a system-design review of the Windows agent
privilege boundary (`docs/protocol/windows-agent-privilege-boundary.md`)
as a product decision, not a pure engineering trade-off — it has direct
end-user-facing and potentially legal implications (some jurisdictions
require notice when a device is being remotely controlled).

The spec's two named MVP use cases (Section 1) have conflicting
expectations here:

- **Remote support/work (AnyDesk/RustDesk-like).** Typically someone is at
  the machine, or it's a personal device. Users expect a visible
  notification when the desktop is being remotely controlled; silent
  control reads as surveillance and undermines trust in the product.
- **Privileged access to production systems (Teleport-like).** Typically
  no one is at the machine — it's a server or unattended workstation. A
  visible notification has no audience and no purpose; the audit trail
  (Section 35: `session.start`, `session.end`, etc.) is the accountability
  mechanism, not a live prompt.

Presented to the product owner as an explicit choice among: always notify,
always silent, per-role/device configurable, or an AnyDesk-style one-time
unattended-access opt-in with a persistent (non-blocking) banner. Decision:
per-role/device configurable.

## Decision

Consent/notification behavior for unattended access is carried as a new
field in `SessionCapability.restrictions` (Spec Section 12):

```text
unattended_consent = notify | silent
```

- `notify`: the client-controlled desktop shows a visible, non-blocking
  indicator (banner/toast) for the duration of the session, identifying
  that it's under remote control. Default for roles/devices matching the
  remote-work/support use case.
- `silent`: no on-screen indicator. Default for roles/devices matching the
  privileged-access-to-production use case, where the audit trail is the
  accountability mechanism.

The value is set by the policy that issues the `SessionCapability` (Section
11's RBAC/ABAC role definition), not chosen by the client or the agent at
connect time — consistent with how `clipboard`, `file_transfer`, and
`recording` are already modeled as restrictions set by policy.

## Consequences

**Positive**
- Serves both of the spec's named use cases without picking one at the
  expense of the other.
- Reuses the existing `restrictions` mechanism — no new capability
  structure, no new signing/verification path.
- Makes the choice auditable: which policy set `notify` vs. `silent` for a
  given session is recoverable from the policy snapshot already stored per
  session (Section 34, `policy_snapshot_json`).

**Negative / follow-up work**
- `nexus-desktop-host` needs to implement the actual on-screen indicator
  for `notify` mode — not yet designed; this is a client/host UX task, not
  covered by this ADR.
- The default value for roles with no explicit `unattended_consent` set
  needs to be chosen deliberately (fail toward `notify`, the more
  conservative/visible option, rather than silently defaulting to
  `silent`) — record this default explicitly wherever the RBAC role schema
  is implemented (Epic F, Control plane).
- Does not by itself address jurisdictions with specific legal notice
  requirements — if that becomes a concrete requirement, it should be
  handled as a policy default enforced at the control-plane level (e.g.
  disallow `silent` for certain device labels or regions), not by changing
  this mechanism.

## Alternatives considered

**Always notify.** Rejected: meaningless and disruptive for the
privileged-access-to-production use case where no one is present to see
or act on the notification.

**Always silent.** Rejected: undermines user trust for the remote-work/
support use case, where the local user reasonably expects to know when
their desktop is being controlled.

**AnyDesk-style: one-time enrollment opt-in, then a persistent non-blocking
banner on every session.** Considered a reasonable middle ground, but
rejected for MVP: adds both an enrollment-time UI step and a distinct
non-blocking-banner UI, doubling the UX surface to build, when the
per-role/device restriction already achieves the same outcome (visible for
one use case, silent for the other) using infrastructure the MVP already
needs to build for other restrictions.

## References

Spec Sections 1, 11, 12, 34, 35, 58. `docs/protocol/windows-agent-privilege-boundary.md`.
