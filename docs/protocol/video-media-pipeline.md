# Design Note — Video / Media Pipeline

Status: Draft, open for review.
Related spec sections: 3.2, 14, 19, 20, 22, 41-43, 47, 61.1, 61.5.
Produced via a system-design review, following on from the Windows agent
privilege boundary review.

## 1. Requirements

Desired path (Section 19): desktop compositor → D3D11 texture → hardware
encoder → bitstream → packetizer → QUIC datagrams, avoiding GPU-to-CPU
copies. Latency budget (Section 43): ~26ms LAN aspirational (capture 5,
encode 5, network 3, decode 5, render 8). Encoder/decoder queues target
≤1 frame (Section 42), consistent with "no unbounded channels" (Section
57).

## 2. High-level design — a missing step in the primary diagram

Section 19's "Desired path" omits color-space conversion, while Section
61.1 (short-term row) specifies it: a D3D11 Video Processor converts
RGBA8 → NV12 directly on VRAM, zero-CPU-copy. This isn't a minor detail —
hardware H.264 encoders (NVENC/QSV/AMF) take NV12 input, not the
compositor's native RGBA8/BGRA8 texture, so this step exists on the normal
path whether or not it's drawn. The diagram in Section 19 has been
corrected to show it explicitly (see spec edit accompanying this note).

```
Desktop compositor (RGBA8, D3D11 texture)
    -> D3D11 Video Processor: RGBA8 -> NV12 (on VRAM, no CPU copy)
    -> hardware encoder (NVENC / QSV / AMF)
    -> encoded bitstream
    -> packetizer
    -> QUIC datagrams
```

## 3. Deep dive

**Backpressure at the capture/encode boundary wasn't explicit.** Section
42 sets a target queue depth (≤1 frame) and Section 57 forbids unbounded
channels, but neither states the actual *behavior* when capture outpaces
encode: block, or drop? Principle 3.2 (interactive freshness) already
settles this for the network/transport stage (Section 14) but hadn't been
extended to capture/encode explicitly. Decision: the queue between capture
and encode is bounded to depth 1 with replace-not-block semantics — a
newly captured frame replaces a still-queued older one; the capture thread
never blocks waiting on a slow encoder. Recorded as ADR-022.

**Resolution-scale changes need a forced keyframe.** The QualityController
(Section 22) can reduce `resolution scale` to adapt to network conditions.
A resolution change breaks inter-frame prediction — decoding will fail
unless the encoder issues a keyframe at the exact moment the resolution
changes. Section 14/20 already list keyframe triggers for packet loss and
post-reconnect (Section 46), but not this one. This has been added directly
to Spec Section 22 as a requirement (not a separate ADR — there's no real
alternative to weigh; H.264 correctness requires it).

**Encoder backend selection is a runtime concern, not yet specified.**
Section 20 lists backends (NVENC, QSV/Media Foundation, AMF/Media
Foundation, software fallback) but not how the agent picks one at startup
when multiple GPUs are present (e.g. a laptop with both an Intel iGPU and
an NVIDIA dGPU), or what happens when the preferred backend's
initialization fails at runtime. Recommendation (implementation guideline
for Epic B, not a formal ADR — no architecture-level trade-off, just an
unspecified procedure): probe backends in priority order at agent startup,
log which one was selected, and fall back to the next backend in the list
on init failure, down to the software encoder. This should be exercised
against Appendix C's multi-vendor GPU test matrix.

**Multi-viewer fan-out authorization policy is now settled by ADR-015**
(`docs/adr/ADR-015-concurrent-session-policy-per-device.md`):
`desktop.control` is exclusive per target device, `desktop.view` may be
granted concurrently to multiple sessions. This unblocks the authorization
question but deliberately leaves the pipeline architecture question open:
whether concurrent viewers share one capture+encode pipeline whose output
fans out to N QUIC sessions, or each gets an independent pipeline
(multiplying GPU/CPU cost per viewer), is left to Epic B/C as a performance
optimization to make once multi-viewer sessions are actually used in
practice (ADR-015 §Decision, point 5) — not a media-pipeline question this
note resolves.

## 4. Scale and reliability

Latency/queue instrumentation is already well specified (Sections 40-41) —
no gap found here. The one addition worth making: the QualityController's
"degrade quickly / recover slowly" tuning constants (Section 22) should be
validated as regression tests directly against the network-simulation
profiles already defined in Section 47, rather than left as free
parameters discovered ad hoc during Phase 2 implementation.

## 5. Trade-off analysis

| Decision | Option A | Option B | Chosen |
|---|---|---|---|
| Capture/encode backpressure | Block capture thread until encoder catches up | Drop stale queued frame, always encode newest | B (ADR-022) — extends Principle 3.2 |
| Resolution-scale change | No forced keyframe | Force keyframe immediately | B — added to Section 22 directly, no ADR needed (no real alternative) |
| Encoder backend selection | Fixed at compile time | Runtime probe + fallback chain, logged | B — implementation guideline for Epic B, not an ADR |
| Multi-viewer fan-out | One pipeline per viewer | One pipeline, fan out encoder output | Authorization policy settled by ADR-015; pipeline architecture left open for Epic B/C |

Decision recorded as ADR-022 (`docs/adr/ADR-022-capture-encode-backpressure-drop-stale.md`).
