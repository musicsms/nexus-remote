# ADR-020: IPC Between Agent Service and Desktop Host Verifies Process Identity, Not Just ACL

## Status

Accepted.

## Context

Spec Section 61.5 specifies Named Pipes with a strict ACL security
descriptor as the IPC transport between `nexus-agent-service` (privileged)
and `nexus-desktop-host` (per-user, less privileged). Section 45 requires
this surface to be "allow-listed, authenticated, versioned, and
fuzz-tested."

An ACL on a named pipe restricts *which Windows account* may open a handle
to it — it does not verify *which binary* is doing the connecting. A
process running under the same user account as the legitimate
`nexus-desktop-host.exe` (including malware, or a modified/unsigned copy)
could open the same pipe if the ACL permits that account, and then issue
privileged requests to the service. This was identified during a
system-design review of the Windows agent privilege boundary
(`docs/protocol/windows-agent-privilege-boundary.md`).

## Decision

`nexus-agent-service` verifies the connecting client process's identity
before honoring any privileged IPC request, in addition to the pipe's ACL:

1. On connection, the service resolves the connecting process's image path
   (e.g. via `QueryFullProcessImageName` against the pipe's client PID).
2. The service verifies the binary's Authenticode signature matches
   Nexus's expected signing certificate, and (for defense in depth) that
   its hash matches the version the service expects to be running
   alongside it.
3. A connection that fails this check is rejected and logged as a
   suspicious event (Section 35 audit model) — this is a security-relevant
   event, not a silent failure.
4. The ACL is retained as the first layer (cheap, coarse-grained); process
   identity verification is the second layer (authoritative for privileged
   operations).

## Consequences

**Positive**
- Closes a same-user impersonation gap that ACL alone leaves open.
- Matches the "authenticated" requirement in Section 45 literally (identity
  of the caller, not just its account).
- Failed verification attempts become a concrete audit signal for
  detecting tampering or malware on the host.

**Negative / follow-up work**
- Adds a signature-check step to every IPC connection setup — should be
  measured to confirm it doesn't materially affect session setup latency
  (Section 40 already tracks this metric).
- Requires the desktop-host binary to always be properly signed even in
  development/CI builds used for integration testing, or the check needs
  an explicit test-mode bypass that is clearly excluded from release
  builds.
- The exact IPC message schema and versioning for this handshake needs to
  be defined under Epic D (Agent) before Phase 1 implementation, per the
  Definition of Done in Section 56.

## Alternatives considered

**ACL only, as literally written in Section 61.5.** Rejected: doesn't meet
the "authenticated" bar in Section 45 in any meaningful sense beyond
account-level access control, and leaves a same-user impersonation gap.

**Full mutual TLS over a local TCP loopback instead of Named Pipes.**
Rejected for MVP: Named Pipes with ACL + signature verification achieves
the same authentication goal with less new infrastructure (no local CA/cert
management needed for a same-machine IPC channel); revisit only if a
concrete need for it emerges (e.g. cross-machine agent/desktop-host
separation, which is not part of the current architecture).

## References

Spec Sections 18, 45, 61.5. `docs/protocol/windows-agent-privilege-boundary.md`.
