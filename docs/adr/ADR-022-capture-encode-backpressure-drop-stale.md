# ADR-022: Capture/Encode Backpressure Is Drop-Stale, Never Block

## Status

Accepted.

## Context

Spec Section 42 targets an encoder queue depth of ≤1 frame, and Section 57
(Engineering Rules) forbids unbounded channels in the media path. Neither
states the actual behavior when the capture stage produces frames faster
than the encoder can consume them: block the capture thread until the
encoder catches up, or drop the stale queued frame and keep only the
newest one.

Principle 3.2 (Interactive freshness over perfect delivery) already
settles this question for the network/transport stage (Section 14: "the
protocol should prefer a fresh frame over retransmission of stale frame
data"), but had not been stated explicitly for the capture/encode boundary.
This surfaced during a system-design review of the video/media pipeline
(`docs/protocol/video-media-pipeline.md`).

## Decision

The queue between capture and encode is bounded to depth 1, with
replace-not-block semantics: when a new frame is captured while a previous
one is still queued waiting for the encoder, the new frame replaces the
old one in the queue slot. The capture thread never blocks waiting for the
encoder to free up capacity. This extends Principle 3.2 explicitly to the
capture/encode stage, not just the network stage.

## Consequences

**Positive**
- Consistent with Principle 3.2 and the "no unbounded channels" /
  "media queues optimize for freshness" Engineering Rules (Section 57) —
  this ADR makes an implicit consequence of existing principles explicit
  rather than introducing a new one.
- Prevents a slow encoder (e.g. a software fallback path, or a GPU under
  load from other processes) from stalling the capture pipeline and adding
  compounding latency.

**Negative / follow-up work**
- A dropped frame is lost entirely — under sustained encoder slowness this
  reduces effective capture FPS rather than queuing up a backlog. This is
  the intended trade-off (fresher content over completeness) but should be
  visible in telemetry: add a "frames dropped at capture/encode boundary"
  counter alongside the existing "Frame drops" metric (Section 40), broken
  out from drops that happen elsewhere in the pipeline (e.g. network loss).
- `nexus-capture` and `nexus-codec`'s internal queue implementation must be
  covered by a unit test asserting depth never exceeds 1 and that the
  newest frame always wins under sustained backpressure — part of the
  Definition of Done (Section 56) for this feature.

## Alternatives considered

**Block the capture thread until the encoder is ready.** Rejected: adds
compounding latency under any encoder slowdown, directly contradicting the
latency budget in Section 43 ("the system should optimize queue depth
before visual fidelity") and Principle 3.2.

**A larger bounded queue (e.g. depth 3) instead of depth 1.** Rejected:
Section 42 already targets ≤1 frame as the goal; a larger queue would trade
some smoothing for added latency under load, which is the wrong trade for
an interactive remote-desktop protocol per Principle 3.2.

## References

Spec Sections 3.2, 14, 40, 42, 43, 57. `docs/protocol/video-media-pipeline.md`.
