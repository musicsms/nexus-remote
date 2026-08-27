# ADR-026: Windows Capture API Selection

## Status

Accepted for Phase 1 implementation.

## Decision

Use Windows Graphics Capture for the interactive in-session desktop capture
backend, with DXGI Desktop Duplication as the runtime fallback for systems or
desktop contexts where WGC cannot initialize. GDI is diagnostic-only. Keep
capture behind `CaptureSource`; device/session loss must surface as a
recoverable backend error and never block the Tokio runtime. A dedicated
Windows thread owns OS capture objects in the required COM apartment. The
existing synthetic source remains the deterministic non-Windows test fallback.

## Consequences

This supports modern Windows in-session capture with a narrow safe boundary.
Pre-login capture and older systems require a later DXGI/service-specific
backend and are not silently claimed by this implementation.
