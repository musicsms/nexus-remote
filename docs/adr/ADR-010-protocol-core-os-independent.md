# ADR-010: Protocol Core Remains OS-Independent

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 3.6,
Section 5) and enforced in CLAUDE.md's own project rules; verified against
the actual workspace during this review (`nexus-protocol`, `nexus-session`,
and `nexus-crypto`'s `Cargo.toml` files carry no OS-specific dependencies).
Surfaced as unwritten during an architecture consistency audit (see
`docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 3.6 (Modular platform abstractions) requires core protocol,
session, transport, crypto, and policy crates to not depend on Windows,
macOS, or Linux APIs, with OS-specific functionality living behind narrow
traits. CLAUDE.md already states this as a binding project rule:
"`nexus-protocol`, `nexus-session`, and `nexus-crypto` must never import
`windows-rs` or any other OS-specific crate." This is the structural
precondition ADR-008 (Windows-first sequencing) depends on to stay
non-exclusionary rather than becoming a de facto Windows-only codebase.

## Decision

`nexus-protocol`, `nexus-session`, and `nexus-crypto` (and, by the same
principle, `nexus-common`, `nexus-auth`, `nexus-policy`, `nexus-audit`, and
`nexus-observability`) contain no OS-specific code or dependencies.
OS-specific functionality — screen capture, hardware encode/decode, input
injection — lives behind narrow traits defined in `nexus-capture`,
`nexus-codec`, and `nexus-input`, with concrete platform implementations
under `platform/<os>/` (Section 5). Dependency direction is strictly
one-way: Product layer → Core crates → Platform abstractions → Native
OS/codec APIs.

## Consequences

**Positive**
- These crates can be built, unit-tested, and fuzzed on any CI runner —
  Section 55's CI requirements only need a Windows-specific runner for the
  Windows build/integration-test job, not for the protocol/session/crypto
  test suite.
- Is the structural precondition that keeps ADR-008's Windows-first
  sequencing additive rather than exclusionary: Phase 5's macOS/Linux
  support (Section 48) only requires new `platform/macos/`,
  `platform/linux/` trait implementations, not changes to
  protocol/session/crypto.
- Verified in this review: `nexus-protocol`, `nexus-session`, and
  `nexus-crypto`'s actual `Cargo.toml` files (as scaffolded) carry no
  OS-specific dependencies today, so this ADR formalizes an already-true
  state rather than mandating a future change.

**Negative / follow-up work**
- Not self-enforcing by the type system alone — requires ongoing engineering
  discipline as these crates gain real logic. A concrete enforcement
  mechanism (e.g. a CI job that builds `nexus-protocol`/`nexus-session`/
  `nexus-crypto` on a non-Windows runner as a boundary-violation smoke test)
  should be added under Section 55's CI/CD requirements — not yet specified,
  flagged as follow-up work.

## Alternatives considered

**Allow narrow `#[cfg(windows)]` leaks into core crates for expedience**
(e.g. a Windows-specific timestamp source or convenience helper). Rejected:
even small leaks erode the boundary this ADR exists to protect, break the
ability to build/test these crates on non-Windows CI runners, and undermine
ADR-008's premise that Windows-first is additive rather than a partial
rewrite when Phase 5 arrives.

**A single shared "platform" crate imported everywhere, including
protocol/session/crypto**, rather than narrow per-concern traits. Rejected:
functionally the same dependency-direction violation with an extra layer of
indirection — it still creates a compile-time dependency from OS-independent
core logic onto OS-specific code, just centralized instead of scattered.

## References

Spec Sections 3.6, 5, 55. CLAUDE.md §4. ADR-008.
