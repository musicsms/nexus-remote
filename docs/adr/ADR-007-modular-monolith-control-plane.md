# ADR-007: Modular Monolith Control Plane

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 7.1)
and consistent with the Section 2 MVP non-goals (no multi-region
control-plane HA); surfaced as unwritten during an architecture consistency
audit (see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 7.1 states: "`nexusd` is a modular monolith for the first releases
... Do not split into microservices until operational scale or
organizational boundaries justify it." Section 2 excludes "Global
multi-region control-plane HA" from MVP scope. Section 53's minimal
self-hosted deployment blueprint shows a single `nexusd` process alongside
SQLite on one machine.

## Decision

`nexusd` ships as a single deployable binary/process containing
authentication, user identity, device enrollment/registry, policy
evaluation, session brokerage, signaling, relay discovery, and audit
logging as internal modules (Section 7.2) — not as separate services — backed
by SQLite (default) or PostgreSQL (Section 6, ADR-013).

## Consequences

**Positive**
- Matches the "Minimal self-hosted deployment" blueprint (Section 53)
  directly: one binary to deploy, configure, and operate, consistent with
  the self-hosting goal (Section 1).
- Rust's module system already enforces internal boundaries between
  auth/devices/policy/sessions/audit at compile time, without the runtime
  overhead of service-to-service calls — a later split into services, if
  scale ever justifies it, has clear seams to split along.
- Avoids inventing cross-service concerns (service discovery, inter-service
  auth, distributed tracing across process boundaries) the MVP doesn't need.

**Negative / follow-up work**
- No built-in high availability for MVP — a single `nexusd` instance is a
  single point of failure. Explicitly accepted as a Section 2 non-goal;
  Section 53 notes "Control plane can begin as a single-region HA
  deployment later."
- Presence WebSocket connections (Section 9) are held in `nexusd`'s memory;
  a restart drops them all. Already documented as an accepted MVP
  limitation in `docs/protocol/session-establishment-signaling.md` §4 (only
  *new* session requests fail-fast during the restart window; already
  ESTABLISHED/ACTIVE sessions are unaffected per ADR-005's design) — this
  ADR formalizes that existing decision, it does not introduce a new gap.

## Alternatives considered

**Microservices from the start** (separate auth, device, session, policy,
audit services). Rejected: adds deployment and operational complexity
(service discovery, inter-service authentication, distributed tracing)
disproportionate to MVP scale, and directly conflicts with the self-hosting
goal — a self-hoster running Nexus on a single small VM or homelab machine
should not need a service-orchestration platform to run the control plane.

**Serverless/FaaS control plane.** Attractive for scale-to-zero cost
profiles. Rejected: `nexusd` holds long-lived presence WebSocket connections
(Section 9) in memory, which does not map cleanly onto stateless FaaS
execution models without introducing an external pub/sub or connection-relay
layer — infrastructure the self-hosting goal doesn't need at MVP scale.

## References

Spec Sections 1, 2, 6, 7.1, 7.2, 9, 53. ADR-005, ADR-013.
`docs/protocol/session-establishment-signaling.md` §4.
