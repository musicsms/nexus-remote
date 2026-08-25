# ADR-002: Tokio as the Async Runtime

## Status

Accepted — retroactively frozen. Already implemented (`tokio` is a
workspace dependency in `Cargo.toml`) and stated in the spec (Section 6,
Section 57, Section 61.5); surfaced as unwritten during an architecture
consistency audit (see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 6 lists Tokio as the core async runtime. Section 57's Engineering
Rules depend on it directly: rule 1 forbids unbounded channels in the media
path (Tokio's `mpsc::channel` with a fixed capacity is the primary tool for
this), rule 2 forbids blocking I/O on runtime worker threads. Section 61.5
already specifies the split this ADR formalizes: "Dedicated OS Native
Thread (`std::thread`) for capture/encode loop; Tokio async runtime for
networking."

## Decision

Tokio is the async runtime for all networking and control-flow code across
`nexusd`, `nexus-relay`, `nexus-agent`, and the networking layer of
`nexus-client`/`nexus-client-core`. CPU/GPU-bound work — capture, encode,
decode, render — runs on dedicated OS threads (`std::thread`), never as
Tokio tasks, per Section 61.5. Any inherently blocking call inside an async
context (e.g. synchronous file I/O, a blocking library call) must use
`spawn_blocking` or a dedicated thread, never run directly on a runtime
worker thread, per Engineering Rule 2.

## Consequences

**Positive**
- Quinn (QUIC, ADR-003) is built natively for Tokio, so this pairs directly
  with the transport choice with no adapter layer needed.
- `tokio::sync::mpsc` bounded channels are the concrete mechanism satisfying
  Engineering Rule 1 ("no unbounded channels in the media path") and the
  drop-stale backpressure model already decided in ADR-022.
- `tracing`'s async-aware instrumentation (Section 40) integrates natively
  with Tokio tasks, giving per-task/per-connection observability without
  extra plumbing.

**Negative / follow-up work**
- Tokio's cooperative scheduler means CPU-bound work accidentally left on an
  async task stalls every other task sharing that worker thread — this is a
  correctness rule (Engineering Rule 2), not something the type system
  prevents by itself. Needs a concrete enforcement mechanism (e.g. a clippy
  lint, or a documented code-review checklist item) rather than relying on
  discipline alone — flagged as follow-up work under CI/CD requirements
  (Section 55).

## Alternatives considered

**async-std.** Comparable async primitives to Tokio. Rejected: smaller
ecosystem and weaker momentum, and critically, Quinn (ADR-003) is a
Tokio-native crate — pairing it with async-std would require an
integration/compatibility shim with no offsetting benefit.

**A hand-rolled epoll/IOCP event loop.** Full control over scheduling.
Rejected: reinvents a large, well-tested surface (task scheduling, timers,
I/O readiness, cross-platform abstraction) that Tokio already solves;
directly conflicts with Section 3.1's "avoid unnecessary large runtime
dependencies" only in the sense of adding a new large *maintenance*
dependency (a custom runtime) in place of a widely-used, audited one.

## References

Spec Sections 3.1, 6, 40, 42, 43, 55, 57, 61.5. ADR-003, ADR-022.
