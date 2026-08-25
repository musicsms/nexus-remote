# ADR-008: Windows-First Host/Client MVP

## Status

Accepted — retroactively frozen. Already stated in the spec (title line,
Section 2, Section 48) and implemented as the only platform target in
Phases 0–4; surfaced as unwritten during an architecture consistency audit
(see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

The spec's own title states "Primary MVP platform: Windows host + Windows
client." Section 2 explicitly excludes macOS and Linux host support from
MVP. Section 48 places macOS/Linux host support in Phase 5 (v1.0), the last
phase. Section 59's "Recommended Immediate Implementation Order" starts
with Windows capture, not a cross-platform abstraction layer.

## Decision

Phase 1 (MVP v0.1) through Phase 4 (v0.5) target Windows only for both the
host agent (`nexus-agent`/`nexus-desktop-host`) and the client
(`nexus-client`). macOS and Linux host support, and their respective media
stacks (VideoToolbox/Metal, VAAPI/Vulkan per Section 30), are Phase 5 work.

## Consequences

**Positive**
- `nexus-capture`, `nexus-codec`, and `nexus-input` (Section 5) already
  isolate platform-specific logic behind narrow traits (ADR-010), so
  Windows-first is a sequencing decision, not a platform-exclusivity
  decision — adding macOS/Linux backends later doesn't require an
  architecture change, only new implementations of existing traits.
- Concentrates Phase 0/1 engineering effort on proving the hardest,
  most product-critical path first — Windows capture/encode/input, per
  Section 59's explicit risk-ordering rationale — rather than spreading
  effort thin across three platforms before any of them work end-to-end.
- Windows has the most fragmented hardware-encoder landscape (NVENC, QSV,
  AMF) of the three target platforms; solving it first means the hardest
  case is validated early rather than deferred.

**Negative / follow-up work**
- `platform/macos/` and `platform/linux/` (Section 5) stay empty and
  unvalidated until Phase 5 — any accidental Windows-specific assumption
  leaking into a supposedly OS-independent trait (ADR-010) won't surface
  until those platforms are actually built against it.
- Delays market reach to non-Windows-host users (e.g. macOS/Linux
  workstations as remote-support targets) until v1.0.

## Alternatives considered

**Cross-platform-first** (build capture/encode/input abstractions for
Windows, macOS, and Linux simultaneously from the start). Rejected: triples
the native-API surface that must be designed and validated before any
end-to-end path works, directly conflicting with Section 59's risk-ordering
guidance to attack the highest-risk part (the interactive media pipeline)
on one platform before generalizing.

**macOS-first.** Common choice for developer/creative-tool remote-desktop
products. Rejected: the two named use cases (Section 1 — remote
support/work, and privileged access to production systems) both skew toward
Windows desktop/workstation prevalence in the target enterprise and
prosumer environments, and Windows is the harder capture/encode problem to
solve, making it the better platform to prove the architecture against
first.

## References

Spec Sections 1, 2, 5, 30, 48, 59. ADR-010.
