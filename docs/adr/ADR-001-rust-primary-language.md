# ADR-001: Rust as Primary Implementation Language

## Status

Accepted — retroactively frozen. The decision is already implemented
throughout the workspace (every crate/app in `Cargo.toml` is Rust) and
stated in the spec's prose (Section 1, Section 3.1, Section 57), but Section
51 calls for it to be recorded as a standalone ADR before heavy
implementation; this was surfaced as unwritten during an architecture
consistency audit (see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 1 states the implementation target is "a Rust-first codebase."
Section 3.1 (Lightweight by construction) requires the host agent to avoid
Electron, JVM, or a browser runtime and use OS APIs directly. Section 57's
Engineering Rules depend on language-level guarantees: no `unsafe` without a
narrow, documented boundary; OS/codec FFI wrapped in safe abstractions;
protocol parsers must treat all remote input as hostile. These rules are
easiest to enforce, and most credible to an auditor, in a memory-safe
systems language.

## Decision

Rust (stable toolchain) is the implementation language for every component
in the workspace: `nexusd`, `nexus-relay`, `nexus-agent`,
`nexus-desktop-host`, `nexus-client`, `nexus-cli`, and all `crates/`
members.

## Consequences

**Positive**
- Memory safety without garbage-collection pauses — directly serves the
  latency budget (Section 43) in a way a GC'd language cannot guarantee, and
  serves Engineering Rule 5 (hostile-input parsing) more credibly than a
  memory-unsafe language.
- One toolchain across control plane, agent, and client simplifies the
  dependency graph (Section 5) and keeps engineering practices (testing,
  fuzzing, CI) uniform across the whole system.
- `unsafe` blocks required for OS/codec FFI (Windows Graphics Capture,
  DXGI, NVENC/QSV/AMF, Media Foundation) are isolated and auditable per
  Engineering Rule 3, rather than being the default risk posture of the
  whole codebase.

**Negative / follow-up work**
- Smaller pool of Rust-experienced Windows systems engineers than
  C++/C#, a hiring/onboarding cost accepted for the safety benefit.
- Hardware encoder/capture SDKs (NVENC, QSV, AMF, Media Foundation, WGC,
  DXGI) are C/C++ APIs; every one of them requires a hand-written or
  `bindgen`-generated FFI wrapper (Section 57 rule 4), an ongoing binding
  maintenance burden not present in a C++-native implementation.

## Alternatives considered

**C++.** Native performance parity with Rust and the most direct FFI story
to the Windows media/codec SDKs. Rejected: the memory-safety burden falls
entirely on manual discipline and review, which is a materially weaker
guarantee against Engineering Rule 5's "hostile input" parsers than Rust's
compiler-enforced safety — and this project processes untrusted network
input at every layer (protocol, capability, video packet parsing).

**Go.** Simple concurrency model, fast compile times, easy cross-compilation.
Rejected: garbage-collector pauses are a direct risk to the latency budget
(Section 43's ~26ms LAN-aspirational target has no room for unpredictable
GC stalls), and Go's story for safe, ergonomic FFI to native codec/capture
APIs is weaker than Rust's without added `cgo` overhead.

**C#/.NET.** Strong native Windows interop via P/Invoke, mature tooling.
Rejected: the CLR runtime's footprint conflicts with the agent idle-RAM
target (< 30 MB, Section 42) and Section 3.1's "avoid large runtime
dependencies" principle in the same way Electron/JVM are rejected for the
client (see ADR-012).

## References

Spec Sections 1, 3.1, 5, 42, 43, 57.
