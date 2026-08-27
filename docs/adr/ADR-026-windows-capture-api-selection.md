# ADR-026: Windows Capture API Selection

## Status

Accepted for Phase 1 implementation.

## Decision

Use Windows Graphics Capture for the interactive in-session desktop capture
backend. Keep capture behind `CaptureSource` and a platform implementation;
device/session loss must surface as a recoverable backend error and never block
the Tokio runtime. A dedicated Windows capture thread owns OS capture objects.
The existing synthetic source remains the deterministic test fallback.

## Consequences

This supports modern Windows in-session capture with a narrow safe boundary.
Pre-login capture and older systems require a later DXGI/service-specific
backend and are not silently claimed by this implementation.
