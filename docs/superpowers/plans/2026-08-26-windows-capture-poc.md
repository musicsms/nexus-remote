# Windows Capture PoC — Implementation Plan

## Goal

Define the OS-independent capture contract and enforce the ADR-022
capture-to-encode freshness boundary. The Windows Graphics Capture backend
will be added behind a Windows-only module in a follow-up VM task; this
milestone keeps Linux CI buildable and makes the queue semantics testable.

## Tasks

- [x] Add `CapturedFrame`, `CaptureSource`, and explicit frame metadata to
  `nexus-capture`.
- [x] Add a bounded depth-1 latest-frame queue with replace-not-block
  semantics and dropped-frame accounting.
- [x] Add unit tests for ordering, replacement, and empty-queue behavior.
- [x] Update implementation status and run workspace verification.
- [x] Add frame validation errors for dimensions and CPU-backed BGRA data.
- [x] Add a validated `CapturedFrame::new_bgra` constructor for backend use.

## Scope boundary

No Windows API or `unsafe` code is introduced in this milestone. The next
Windows VM milestone will implement `CaptureSource` using Windows Graphics
Capture with DXGI fallback and convert native textures into `CapturedFrame`.

The encoder contract is implemented alongside the capture contract so native
capture and software/hardware H.264 backends share the same interface.

Encoder configuration validation and queue producer/consumer concurrency are
covered by unit tests before native backends are introduced.
