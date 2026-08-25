# Design Note — Windows Agent Architecture & Privilege Boundary

Status: Draft, open for review.
Related spec sections: 18, 39, 45, 46, 58, 61.1, 61.5.
Produced via a system-design review, following on from the session
establishment/authorization/connectivity reviews.

## 1. Requirements

Split a privileged system service (`nexus-agent-service.exe`: identity,
presence, updates, privileged operations, session process spawning) from
an interactive per-session process (`nexus-desktop-host.exe`: capture,
encode, input, cursor, clipboard, audio) across a narrow IPC boundary
(Section 18). Hard constraint: the desktop process must never have an
unrestricted API to install drivers, replace service binaries, modify
arbitrary system files, or change update signing configuration (Section
45). The privileged IPC surface must be allow-listed, authenticated,
versioned, fuzz-tested.

One ambiguity worth flagging: Section 1/42's "Agent idle RAM < 30MB"
doesn't specify whether this covers `nexus-agent-service` alone or the
combined footprint with `nexus-desktop-host`. Since desktop-host isn't
running when there's no active session, the natural reading is that idle
RAM applies to the service alone — but Section 50's acceptance criterion
("Agent idle RAM is measured and documented") should say so explicitly
when the measurement is actually implemented.

## 2. High-level design

The base diagram in Section 18 is correct but doesn't show what happens
across Windows session-state transitions: lock/unlock, UAC Secure Desktop,
fast user switching, sleep/wake. One of these is a hard OS constraint, not
an engineering choice: **neither Windows Graphics Capture nor DXGI Desktop
Duplication can capture the Secure Desktop** (UAC elevation prompts,
Ctrl+Alt+Del screen, lock screen) — Windows blocks this by design. The spec
doesn't yet define what the client shows during this state (a black frame,
or an explicit placeholder). This is a UX decision, not resolved by this
note — flagged as still open in Spec Section 58.

## 3. Deep dive

**IPC authentication needs more than ACL.** Section 61.5 specifies Named
Pipes with a strict ACL. An ACL controls which Windows account can open the
pipe, not which binary — a same-user process (e.g. malware running as the
logged-in user) could still connect if the ACL permits that account.
Decision: the service also verifies the connecting process's code
signature/hash before honoring any privileged request. Recorded as
ADR-020.

**Two distinct privilege contexts for desktop-host, not one.** Section
61.1's short-term recommendation ("spawn dedicated SYSTEM-privileged host
runner in Winlogon desktop") is correct for the pre-login/unattended-access
case — only `SYSTEM` can reach the Winlogon desktop. But Section 45's
minimum-privilege principle requires the in-session capture process (while
a user is logged in) to run as that user, not `SYSTEM` — running the
in-session process as `SYSTEM` would itself violate Section 45. Decision:
`nexus-desktop-host.exe` runs in two contexts — a `SYSTEM` instance scoped
to Winlogon/pre-login only, and a `CreateProcessAsUser` instance for normal
in-session capture. Recorded as ADR-021.

**Unattended-access consent/notification is a product decision, not
resolved unilaterally by this review.** Section 58 left this open. The
spec's two named use cases (Section 1) pull in opposite directions:
remote-work/support (AnyDesk-like) typically has someone at the machine and
expects a visible notification; privileged production access (Teleport-
like) typically has no one at the machine, where a notification is
meaningless and the audit trail (Section 35) substitutes for consent.
Presented as an explicit choice to the product owner (Minh); decided:
**configurable per role/device**, carried as a new `restrictions` field
(`unattended_consent = notify | silent`) on `SessionCapability`, same
mechanism already used for clipboard/file_transfer/recording. Recorded as
ADR-023.

**Desktop-host crash mid-session.** Sections 45/46 don't say what happens
if `nexus-desktop-host` crashes (e.g. a faulty encoder backend). Decision:
the service detects the process exit and respawns desktop-host under the
same `session_id`, reusing the existing reconnect semantics (Section 46)
instead of ending the session. Recorded as ADR-024.

## 4. Scale and reliability

Not scale-sensitive (single host, single agent) — the reliability concern
is entirely about resilience to local process failures and Windows session
transitions, covered above. Worth adding to Appendix C's test matrix:
explicit "fast user switching" and "UAC Secure Desktop during active
session" scenarios alongside the existing lock/unlock and sleep/wake cases.

## 5. Trade-off analysis

| Decision | Option A | Option B | Chosen |
|---|---|---|---|
| IPC authentication | ACL only | ACL + connecting-process signature/hash verification | B (ADR-020) |
| desktop-host privilege | Always `SYSTEM` | `SYSTEM` only for Winlogon/pre-login; as-user for in-session | B (ADR-021) |
| Unattended consent | Always notify | Always silent | Neither — per-role/device policy (ADR-023) |
| desktop-host crash | End session immediately | Respawn, reuse reconnect semantics | B (ADR-024) |

Decisions recorded as ADR-020, ADR-021, ADR-023, ADR-024 in `docs/adr/`.
Still open (not resolved by this note): client-side UX for the Secure
Desktop state, and whether multiple concurrent Windows sessions
(fast user switching) need explicit "which session is active" tracking in
`nexus-agent-service` — flagged for a future review, not blocking Phase 0/1.
