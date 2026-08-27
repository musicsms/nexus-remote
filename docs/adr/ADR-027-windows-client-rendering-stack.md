# ADR-027: Native Windows Client Rendering Stack

## Status

Accepted for Phase 1 implementation.

## Decision

Use a native Win32 window with Direct3D 11-backed rendering for the Windows
client. Keep window/message handling and GPU resources outside Tokio worker
threads, and expose decoded frame surfaces through a small renderer trait.

## Consequences

The client remains native and avoids browser/runtime overhead. Direct3D 11 is
widely available on the MVP Windows target; renderer initialization failure is
reported explicitly and does not weaken frame authentication.
