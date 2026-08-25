# ADR-012: Native Client, No Electron

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 2,
Section 3.1, Section 29); surfaced as unwritten during an architecture
consistency audit (see `docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 2 excludes a browser client from MVP. Section 3.1 (Lightweight by
construction) forbids embedding Electron, JVM, or a browser runtime and
requires native Rust processes using OS APIs directly. Section 29 states
this for the client specifically: "Avoid Electron ... The remote-display
surface should be native/GPU rendered rather than drawn through a browser
canvas." Section 30's desired Windows client path (QUIC → hardware decoder
→ GPU texture → D3D11 renderer) is only achievable without a browser
compositor in between.

## Decision

`nexus-client` is a native Rust application with a native/GPU-rendered
display surface — not an Electron/Chromium-embedded client and not a
browser-canvas-based renderer for the remote-desktop surface specifically.
The final UI framework choice among the candidates named in Section 29
(Slint, iced, egui) remains open per Section 58.

## Consequences

**Positive**
- Enables the direct hardware-decode → GPU-texture → native-renderer path
  (Section 30) with no browser compositor or canvas round-trip in the
  middle — load-bearing for the latency budget (Section 43: decode 5ms,
  render 8ms LAN-aspirational).
- Keeps the client's resource footprint aligned with the product's
  "lightweight by construction" positioning (Section 3.1), the same
  reasoning behind choosing Rust for the whole codebase (ADR-001).

**Negative / follow-up work**
- Native UI framework ecosystems (Slint/iced/egui) are less mature for
  building polished settings/admin screens than web-based UI tooling — more
  UI engineering effort per screen than an Electron+React equivalent would
  require. The specific framework choice remains an open item (Section 58).
- Cross-platform UI consistency, once macOS/Linux clients exist (Phase 5,
  ADR-008), must be re-validated per native framework rather than achieved
  "write once" via a web view.

## Alternatives considered

**Electron (Chromium + Node.js).** Rich UI tooling ecosystem, fast to build
polished screens. Rejected outright: directly violates Section 3.1's
lightweight-by-construction principle, and a browser compositor/canvas path
adds GPU-texture-upload and color-space-conversion overhead that Section
30's native D3D11 path is specifically designed to avoid — unacceptable for
a client that must render 60fps 1080p+ video with minimal added latency.

**Tauri** (Rust backend + system webview for UI, lighter than Electron).
Considered as a middle ground. Rejected for the remote-display surface
specifically: still routes video rendering through a browser engine's
canvas/WebGL, incurring the same GPU-texture round-trip Section 30's
"Desired Windows client path" is designed to avoid. A Tauri-style split
(native rendering surface for the desktop view, webview for chrome/settings
screens only) is not ruled out by this ADR and could be a legitimate
refinement of the UI-framework choice in Section 58 — but the remote-display
surface itself must stay native/GPU regardless of that choice.

## References

Spec Sections 2, 3.1, 29, 30, 43, 58. ADR-001, ADR-008.
