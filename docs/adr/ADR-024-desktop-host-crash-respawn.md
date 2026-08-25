# ADR-024: Desktop-Host Crash Triggers Respawn via Existing Reconnect Semantics

## Status

Accepted.

## Context

Spec Section 46 (Reconnect and Failure Semantics) is written entirely from
the network-loss perspective — transient connectivity loss, reconnect
window, keyframe on reconnect. Sections 18 and 45 don't say what happens if
`nexus-desktop-host.exe` itself crashes mid-session (e.g. a faulty
hardware-encoder backend, a driver issue, an unhandled panic in the capture
pipeline). This gap surfaced during a system-design review of the Windows
agent privilege boundary (`docs/protocol/windows-agent-privilege-boundary.md`).

Without a defined behavior, two independently-built implementations of
`nexus-agent-service` could reasonably diverge: one ending the session
outright on any desktop-host crash, another silently doing nothing (client
just sees the video freeze with no recovery).

## Decision

`nexus-agent-service` monitors its spawned `nexus-desktop-host` process. On
an unexpected exit:

1. The service treats this the same as a transport-level disconnection for
   reconnect purposes (Section 46) — the session's `session_id` remains
   valid within the existing reconnect window (30–120s configurable).
2. The service respawns `nexus-desktop-host` in the same privilege context
   it was running in (ADR-021: `SYSTEM` for Winlogon/pre-login, as-user for
   in-session).
3. The respawned process re-establishes capture and requests a fresh
   keyframe be sent once media resumes, exactly as an ordinary
   post-reconnect keyframe is already required (Section 46).
4. A crash event is recorded as an audit event (Section 35) distinct from
   an intentional session end, so repeated crashes are visible in
   telemetry/monitoring rather than silently retried forever.
5. If the desktop-host keeps crashing on respawn (e.g. 3 failures in a
   short window), the service gives up and transitions the session to
   FAILED rather than looping indefinitely — the exact retry/backoff
   policy is an implementation detail for Epic D, not frozen by this ADR.

## Consequences

**Positive**
- A single flaky encoder backend or transient capture failure doesn't need
  to end an otherwise-healthy session — consistent with the resilience
  goals already established for network loss in Section 46.
- Reuses existing reconnect machinery (session ID stability, keyframe on
  resume) instead of inventing a parallel recovery path.
- Crash-vs-intentional-end is now a distinguishable, auditable event.

**Negative / follow-up work**
- `nexus-agent-service` needs explicit process-monitoring logic (detect
  unexpected exit vs. clean shutdown) and a bounded retry/backoff policy —
  to be defined under Epic D before Phase 1 sign-off, per the reconnect
  test coverage already required in Section 47 ("Agent restart").
- The client side needs to treat "desktop-host crash + respawn" the same
  as a network reconnect visually (a brief interruption, then a fresh
  keyframe) rather than surfacing a different error state, to avoid
  confusing two recoveries that should look the same to the user.

## Alternatives considered

**End the session on any desktop-host crash.** Rejected: unnecessarily
brittle — a transient encoder fault (e.g. a GPU driver hiccup) would force
the user to fully re-authenticate and re-establish the session for a
failure that a simple respawn can usually recover from within the existing
reconnect window.

**Silent retry with no audit trail.** Rejected: violates Engineering Rule
8 (Section 57 — "every privileged operation is auditable") and would make
a host with a genuinely failing encoder backend invisible to monitoring
until a user complains.

## References

Spec Sections 18, 35, 45, 46, 47, 57. `docs/protocol/windows-agent-privilege-boundary.md`.
