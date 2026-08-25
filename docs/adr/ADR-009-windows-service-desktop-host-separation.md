# ADR-009: Windows Service + Per-User Desktop-Host Process Separation

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 18,
Section 45) and the structural basis ADR-020 and ADR-021 both already build
on; surfaced as unwritten during an architecture consistency audit (see
`docs/IMPLEMENTATION_STATUS.md` §3). Recorded now because those two ADRs
already cite this decision as a prerequisite.

## Context

Section 18 defines the Windows agent architecture as two processes:
`nexus-agent-service.exe` (identity, network/presence, updates, privileged
operations, session spawning) and `nexus-desktop-host.exe` (capture,
hardware encoding, input injection, cursor, clipboard, audio), separated by
a narrow IPC boundary. Section 45 (Secure Privilege Boundary) requires the
media process to use minimum privilege for the active user desktop, while
the service may run elevated.

## Decision

On Windows, Nexus splits into two processes rather than one:
`nexus-agent-service.exe`, a privileged Windows service handling identity,
presence, updates, and session-process spawning; and
`nexus-desktop-host.exe`, an interactive per-session process handling
capture, encode, input, cursor, clipboard, and audio, communicating with the
service across a narrow, authenticated IPC boundary (ADR-020). The two
processes are never combined into one.

## Consequences

**Positive**
- Matches Windows' own security model directly: only a service/`SYSTEM`
  context can reach the pre-login Winlogon desktop, and only a per-user
  context should touch the interactive user's desktop — fighting this model
  with a single combined process would work against the platform, not with
  it.
- Is the load-bearing structure ADR-020 (IPC process-identity
  authentication) and ADR-021 (two desktop-host privilege sub-contexts)
  both depend on — neither of those decisions is meaningful without this
  process split existing first.
- Limits the blast radius of a `nexus-desktop-host` compromise: it is the
  process parsing hostile-by-default remote input (Engineering Rule 5), so
  keeping it out of the privileged service is a direct application of
  minimum privilege (Section 45).

**Negative / follow-up work**
- Cross-process IPC adds a design and maintenance surface not present in a
  single-process architecture: message schema, versioning, and
  authentication (Section 45's "allow-listed, authenticated, versioned,
  fuzz-tested" requirement, met by ADR-020).
- Session start incurs a small added latency for the spawn-plus-IPC-handshake
  sequence, which must be measured against the first-frame target (Section
  1) as part of Section 40's observability metrics — not assumed
  negligible.

## Alternatives considered

**Single combined process running as `SYSTEM` at all times.** Simpler:
no IPC boundary to design. Rejected outright: directly violates Section
45's minimum-privilege principle — a compromised capture/encode process
(which processes hostile-by-default remote input per Engineering Rule 5)
would immediately hold `SYSTEM` privilege, unacceptably widening blast
radius. This is the same reasoning ADR-021 later applies one level deeper,
to the two privilege sub-contexts within `nexus-desktop-host` itself.

**Single process running always as the interactive user, no privileged
service.** Avoids needing elevated privilege management entirely. Rejected:
cannot reach the pre-login Winlogon desktop for unattended access, which
Section 1's remote-support use case explicitly requires (controlling a
locked or logged-out machine), and cannot perform genuinely privileged
operations (signed update installation, service lifecycle management)
without an interactive UAC prompt interrupting what must be an unattended,
automated agent.

## References

Spec Sections 1, 18, 40, 45, 57. ADR-008, ADR-020, ADR-021.
