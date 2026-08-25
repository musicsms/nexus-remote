# ADR-004: H.264 as the Mandatory MVP Codec

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 2,
Section 6, Section 20) and reflected in the `VideoEncoder` trait design;
surfaced as unwritten during an architecture consistency audit (see
`docs/IMPLEMENTATION_STATUS.md` §3). This ADR gates Phase 0's H.264
hardware-encoder PoC (Section 48).

## Context

Section 2 explicitly excludes "AV1 or HEVC mandatory support" from MVP.
Section 20's codec policy already states H.264 mandatory, HEVC optional
post-MVP, AV1 later. Section 6 lists H.264 as the mandatory initial codec
across the recommended Windows media stack (NVENC, QSV/Media Foundation,
AMF/Media Foundation). Section 31's capability negotiation already reserves
`H264`/`HEVC`/`AV1` as distinct feature flags, so the codec choice is meant
to be extensible from the start, not hardcoded.

## Decision

H.264 (hardware-encoded via NVENC/QSV/AMF/Media Foundation, with a software
encoder fallback for CI/testing only) is the only codec the MVP must support
end-to-end, encoder to decoder. The `VideoEncoder` trait (Section 20) is
codec-agnostic by design so HEVC and AV1 can be added later as additional
backends without changing the trait or the surrounding pipeline
architecture.

## Consequences

**Positive**
- Broadest hardware encoder/decoder support across Appendix C's GPU test
  matrix (NVIDIA, Intel integrated, AMD) — minimizes how often the software
  fallback path is exercised in practice.
- Safest choice for meeting the <1–2s first-frame and 1080p60 targets
  (Section 1, Section 50) on the defined minimum test hardware, since H.264
  hardware encode/decode is universally available on that hardware set.
- The codec-agnostic trait and Section 31 capability flags mean this
  decision doesn't need to be revisited architecturally when HEVC/AV1 are
  added post-MVP — only a new backend implementation is needed.

**Negative / follow-up work**
- H.264 has materially worse compression efficiency than HEVC/AV1 at
  equivalent perceptual quality, meaning higher bandwidth for the same
  visual fidelity (Section 42's 5–20 Mbps high-motion target reflects this
  cost) — an accepted MVP trade-off, not an oversight.
- Multiple hardware backends (NVENC, QSV, AMF) must each be implemented,
  tested, and kept working across driver/SDK updates — the encoder-backend
  selection/fallback procedure is a separate, already-flagged
  implementation task for Epic B (see
  `docs/protocol/video-media-pipeline.md`).

## Alternatives considered

**VP9.** Royalty-free. Rejected: weaker, less universal hardware encoder
support across the NVIDIA/Intel/AMD consumer and prosumer GPU landscape
compared to H.264, and Windows Media Foundation's hardware encoder ecosystem
centers on H.264/HEVC, not VP9 — would force more of the fleet onto the
software fallback path, directly hurting the performance targets.

**AV1.** Best compression efficiency of the three. Rejected for MVP:
hardware encode support is still comparatively new and limited, especially
on older/integrated GPUs in Appendix C's baseline test matrix (e.g. some
Intel integrated GPUs lack AV1 hardware encode entirely), and AV1 encode
complexity is higher, risking the latency budget (Section 43) on exactly
the "defined minimum test hardware" Section 50's acceptance criteria
requires 1080p30 to work on. Reserved as a later optimization for capable
hardware (Section 20), not rejected permanently.

**HEVC.** Comparable hardware support breadth to H.264. Deferred, not
rejected: licensing/patent-pool royalty complexity is real adoption
friction for a self-hostable product (Section 1's self-hosting goal implies
minimizing legal/operational overhead for self-hosters), and client-side
hardware decode support is less universal than H.264 on older Windows
client hardware still in Appendix C's target matrix. Explicitly staged as
"optional post-MVP" per Section 2/20.

## References

Spec Sections 1, 2, 6, 20, 31, 42, 43, 50, Appendix C.
