# ADR-013: SQLite as the Default MVP/Self-Host Database, PostgreSQL as the Upgrade Path

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 6,
Section 7.1, Section 34, Section 53, Section 54) and cited by number
("ADR-013") from multiple other documents in this repository
(`docs/protocol/session-establishment-signaling.md`, CLAUDE.md §5) before
this file existed; surfaced as unwritten during an architecture consistency
audit (see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 6 names SQLite as the default embedded database for MVP and
single-node self-hosting, and PostgreSQL as the upgrade path for
multi-tenant or horizontally-scaled deployments, both via the same SQLx
async layer. Section 7.1 restates this for the control plane specifically.
Section 34 requires schema/migrations to stay portable between the two
backends. Section 53's deployment blueprint shows SQLite as a single file
on local disk in the minimal self-hosted case. Section 54's configuration
model selects the backend via `database.driver` (`sqlite` | `postgres`).
The self-hosting goal (Section 1) is the primary driver: a zero-external-
dependency default.

## Decision

SQLite (via SQLx) is the default database for MVP and single-node
self-hosted deployments. PostgreSQL is a drop-in upgrade path using the
same schema and queries, selected via the `database.driver` configuration
value (Section 54), for deployments that need concurrent multi-writer
access, horizontal scaling, or multi-region control-plane HA. Migrations
(Section 34) must avoid backend-specific types or features to keep this
portable.

## Consequences

**Positive**
- MVP/self-host deployment is genuinely zero-external-dependency (Section
  1's self-hostability goal) — no separate database server to run, secure,
  or back up for a single small VPS or homelab deployment.
- The upgrade path is a configuration change, not a schema rewrite (Section
  53) — self-hosters can start on SQLite and graduate only when they
  actually need multi-writer/horizontal scale, lowering switching cost.
- SQLx's compile-time-checked queries work against both backends from the
  same codebase, so this doesn't fork the data-access layer.

**Negative / follow-up work**
- Schema and migrations must be written defensively against the
  lowest-common-denominator of SQLite/PostgreSQL types and features (Section
  34) — a standing engineering discipline for the life of the project, not
  a one-time cost. This constrains schema choices, e.g. avoiding
  PostgreSQL-specific array columns or SQLite-specific dynamic-typing
  shortcuts.
- SQLite's single-writer model means `nexusd`'s own concurrency model (how
  many concurrent DB writers it spawns, e.g. via connection-pool sizing)
  must be designed around that constraint from the start, not discovered as
  a bottleneck later.

## Alternatives considered

**PostgreSQL-only from the start.** Simpler to support one backend.
Rejected: conflicts directly with the self-hosting goal (Section 1) — the
"Minimal self-hosted deployment" blueprint (Section 53) needs `nexusd` plus
a single local file, not a separately operated database server, for the
smallest self-hosted deployments to be genuinely simple.

**A custom embedded key-value store** (e.g. `sled`, `redb`) instead of a
relational database. Rejected: the data model (Section 34 — organizations,
users, devices, roles, sessions, audit events, and their relationships) is
inherently relational, with ad hoc query needs (audit search, session
history) that a KV store would require reimplementing on top of. SQLite
already provides durable, transactional, embedded relational storage at
effectively the same "zero external dependency" property, with a vastly
larger and better-understood surface.

**MySQL/MariaDB as the scale-up target instead of PostgreSQL.** Also a
mature, widely-deployed RDBMS. Rejected: PostgreSQL has stronger support for
features Nexus is likely to need as it scales — e.g. JSONB for the
`policy_snapshot_json`/`capabilities_json` fields already in Section 34's
schema, richer indexing options — and SQLx's PostgreSQL support is more
mature and idiomatic within the Rust ecosystem than its MySQL support.

## References

Spec Sections 1, 6, 7.1, 34, 53, 54. ADR-007.
`docs/protocol/session-establishment-signaling.md`. CLAUDE.md §5.
