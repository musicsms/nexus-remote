# Nexus — Agent Rules

Nexus is a greenfield Rust remote-desktop platform (Teleport-inspired
identity/access plane + custom low-latency media/data plane). The project is
has completed the Phase 0 foundation and is implementing the Phase 1 MVP:
the Windows platform scaffold and core persistence slices exist, while the
Phase 1 acceptance condition remains unverified—a real Windows host and
client must control a host through the relay. Any agent (Claude or otherwise)
working in this repo must follow the rules below.

## 1. Read before you write

In this order, every time, before making non-trivial changes:

1. `docs/Nexus Remote Desktop Platform - Spec.md` — the target architecture.
   This is the source of truth for design decisions, protocol shape,
   security model, and phase scope. Read the section relevant to your task
   (e.g. Section 12 before touching session capabilities, Section 21 before
   touching the video packet header).
2. `docs/IMPLEMENTATION_STATUS.md` — what's actually built right now versus
   that target. Don't assume a crate/app has real logic just because it
   exists; check its status here first.
3. `docs/adr/` — frozen Architecture Decision Records. An ADR overrides the
   spec's prose if the two ever disagree (the ADR is the more recent, more
   specific decision). `docs/IMPLEMENTATION_STATUS.md` §3 is the index of
   which ADRs exist and what each one decided — check it before assuming an
   area is undecided.
4. `docs/protocol/` — design notes with the reasoning behind specific ADRs
   (session establishment/signaling, session authorization, connectivity/
   NAT traversal, Windows agent privilege boundary, video/media pipeline).
   Read the one covering your area before touching that code — it has the
   edge cases and trade-offs the spec's prose doesn't spell out.

The spec (file 1) is the stable architecture document. It changes rarely,
and only when the target design itself changes — not when implementation
progress changes. Do not add "current status" notes, TODOs, or progress
markers into the spec; that belongs exclusively in file 2.

## 2. Scope discipline

- Section 2 of the spec ("Non-Goals for the Initial MVP") is binding. Do not
  implement macOS/Linux host support, browser/mobile client, AV1/HEVC,
  SAML/SCIM/LDAP, session recording, file transfer, audio, multi-monitor
  streaming, JIT approval, device posture, or multi-region HA unless the
  user explicitly asks for that specific post-MVP feature.
- Work phase-by-phase per Section 48. Check `IMPLEMENTATION_STATUS.md` for
  the current phase before starting a task; don't build Phase 2+ features
  while Phase 0/1 exit conditions are still unmet, unless asked to.
- `nexus-audio` and `nexus-file-transfer` are Phase 3 crates. Do not create
  them or add them to `Cargo.toml` before Phase 3 starts (Spec Section 5,
  Appendix A).

## 3. Engineering rules (Spec Section 57 — non-negotiable)

- No unbounded channels anywhere in the media path.
- No blocking I/O on Tokio runtime worker threads.
- No `unsafe` without a narrow module boundary and documented invariants.
- All OS/codec FFI must be wrapped in safe abstractions.
- Protocol parsers treat all remote input as hostile — validate, don't trust.
- Every network message has an explicit maximum size limit.
- Every session has explicit lifecycle and timeout semantics.
- Every privileged operation is auditable.
- Media queues optimize for freshness (drop stale frames, don't buffer them).
- Performance measurements are part of "done," not a follow-up task.

## 4. Workspace and crate rules

- Dependency direction is one-way: Product layer → Core crates → Platform
  abstractions → Native OS/codec APIs (Spec Section 5). Never add a reverse
  dependency.
- `nexus-protocol`, `nexus-session`, and `nexus-crypto` must never import
  `windows-rs` or any other OS-specific crate. OS-specific code lives behind
  narrow traits in `nexus-capture`, `nexus-codec`, `nexus-input`, or under
  `platform/<os>/`.
- New crates/apps go through `crates/*` or `apps/*` (the workspace globs in
  the root `Cargo.toml` already pick them up); add the new crate to
  `[workspace.dependencies]` explicitly so other crates can depend on it via
  `workspace = true`.

## 5. Database

- SQLite is the default backend for MVP and self-hosted single-node
  deployments (ADR-013, Spec Sections 6/34/53/54). PostgreSQL is the
  upgrade path for multi-tenant/scaled deployments, via the same SQLx
  layer.
- Keep schema and migrations portable between SQLite and PostgreSQL — avoid
  backend-specific types or features. Migrations live in `migrations/`.
- Config selects the backend via `database.driver` (`sqlite` | `postgres`),
  not a hardcoded choice in code.

## 6. Undecided things — freeze an ADR, don't silently choose

Spec Section 51 and Section 58 are the canonical lists of decisions that
need an ADR before heavy implementation — don't copy that list here, it
will just go stale as ADRs get written; read those sections directly, and
cross-check against `docs/IMPLEMENTATION_STATUS.md` §3 to see which are
already resolved (as of this writing: all 24 tracked ADRs — 001–024 — have
been written; further open items remain only outside this tracked set, per
Spec Section 58).

If your task depends on one of these and no ADR exists yet in `docs/adr/`:
write a short ADR there first (context, decision, consequences), or — if
the decision is non-obvious or has product implications — stop and ask the
user instead of guessing. Do not make an irreversible architectural choice
inline in a PR with no record of why.

## 7. Mandatory bookkeeping: keep IMPLEMENTATION_STATUS.md current

Every time you do one of the following, update
`docs/IMPLEMENTATION_STATUS.md` in the same change:

- Scaffold a new crate, app, or target directory (`platform/`, `proto/`,
  `migrations/`, `deployment/`, `test/*`, `docs/adr|protocol|security`) →
  mark it **Scaffolded**.
- Add real logic to something that was a stub → mark it **In progress**,
  with a one-line note on what's implemented.
- Finish something to the point it meets its spec section's intent → mark
  it **Done**.
- Meet a phase's exit condition (Spec Section 48) → mark that phase
  **Done** and note the date.
- Write a new ADR to `docs/adr/` → mark that ADR row **Done** and link the
  file.

This file is the only place progress is tracked. If it drifts from the
repo, the next agent (or the user) has no reliable way to know what's
already built — treat an out-of-date status file as a bug in your own
change, not a separate cleanup task.

## 8. Definition of done for a protocol feature (Spec Section 56)

Before calling any protocol-level change complete:

- Binary/schema layout documented.
- Version/capability interaction specified.
- Happy-path unit tests exist.
- Malformed-input tests exist (parsers must reject hostile input).
- Backward-compatibility behavior documented.
- Metrics/logging added.
- Security impact reviewed.
- At least one end-to-end integration test exists.

## 9. Commands

No CI config or task runner exists yet (plain `cargo` workspace). Until one
is added, use:

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Update this section once real CI (Spec Section 55) or a task runner lands,
so it stays accurate.

## 10. Quick map

- `docs/Nexus Remote Desktop Platform - Spec.md` — target architecture (read-mostly).
- `docs/IMPLEMENTATION_STATUS.md` — current build status, and the index of which ADRs exist (update-often).
- `docs/adr/` — frozen decisions, one file per ADR (27 written so far out of 27 tracked).
- `docs/protocol/` — design notes with the reasoning behind those ADRs, organized by area (session establishment, session authorization, connectivity, Windows agent, video pipeline).
- `crates/` — OS-independent core logic.
- `apps/` — binaries (`nexusd`, `nexus-relay`, `nexus-agent`, `nexus-desktop-host`, `nexus-client`, `nexus-cli`).
- `platform/<os>/`, `proto/`, `migrations/`, `deployment/`, `test/*` — not created yet; see status file before assuming they exist.
