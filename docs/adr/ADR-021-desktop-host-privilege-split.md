# ADR-021: Desktop-Host Runs in Two Privilege Contexts (SYSTEM for Pre-Login, As-User In-Session)

## Status

Accepted.

## Context

Spec Section 61.1 (short-term recommendation) says: "`WM_WTSSESSION_CHANGE`
listener in Service; spawn dedicated `SYSTEM`-privileged host runner in
`Winlogon` desktop." Read on its own, this could be taken to mean
`nexus-desktop-host.exe` always runs as `SYSTEM`. But Section 45 requires
"the media process should use the minimum privilege needed for the active
user desktop" — running the in-session capture/encode/input process as
`SYSTEM` when a normal user is logged in would itself violate that
principle, and unnecessarily widens the blast radius of any
`nexus-desktop-host` vulnerability (it processes hostile-by-default remote
input per Section 57's Engineering Rules).

This surfaced during a system-design review of the Windows agent privilege
boundary (`docs/protocol/windows-agent-privilege-boundary.md`).

## Decision

`nexus-desktop-host.exe` is spawned by `nexus-agent-service` in one of two
distinct privilege contexts, depending on why it's running:

1. **Pre-login / unattended access to the Winlogon desktop.** Spawned as
   `SYSTEM`, scoped specifically to reaching the lock screen / Winlogon
   desktop — this is the only privilege level that can attach to that
   desktop on Windows, and it's used only for this purpose.
2. **Normal in-session capture (a user is logged in).** Spawned via
   `CreateProcessAsUser` under the interactive user's own token — matching
   Section 45's minimum-privilege requirement. This is the common case for
   both the AnyDesk-like and Teleport-like use cases once a session is
   underway.

`nexus-agent-service` tracks which context is active via the existing
`WM_WTSSESSION_CHANGE` listener (Section 61.1) and spawns/tears down the
appropriate desktop-host instance on session-state transitions (login,
logout, lock, unlock).

## Consequences

**Positive**
- Matches Section 45's minimum-privilege principle for the common
  (in-session) case, rather than defaulting to `SYSTEM` everywhere for
  simplicity.
- Limits the blast radius of a `nexus-desktop-host` compromise during a
  normal session — it runs with the logged-in user's privileges, not
  `SYSTEM`.
- Keeps the pre-login/unattended path working exactly as Section 61.1
  already specifies — no change there.

**Negative / follow-up work**
- `nexus-agent-service` now manages two different spawn/teardown code
  paths instead of one, and must handle the transition between them
  correctly (e.g. user logs in during an active pre-login unattended
  session — the `SYSTEM` instance should hand off to an as-user instance,
  or the transition needs explicit design).
- Both instances still cross the same IPC boundary to the service; ADR-020
  (process identity verification) applies equally to both.
- Needs explicit test coverage in Appendix C's matrix: lock → unlock
  transition, and login while an unattended session is already active.

## Alternatives considered

**Always `SYSTEM`.** Rejected: simpler to implement, but directly
contradicts Section 45's minimum-privilege principle and unnecessarily
widens the attack surface for the common in-session case.

**Always as-user, with a separate always-privileged helper only for the
Winlogon screen.** This is effectively the decision made above, phrased
differently — no substantive alternative here; the two contexts are
required by Windows' own security model (only `SYSTEM` can attach to
Winlogon), not a design choice.

## References

Spec Sections 18, 45, 61.1. `docs/protocol/windows-agent-privilege-boundary.md`.
