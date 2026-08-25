# ADR-006: Application-Level End-to-End Encryption Through the Relay

## Status

Accepted — retroactively frozen. Already stated in the spec (Section 1,
Section 17, Principle 3.5) and the Section 44 threat model; surfaced as
unwritten during an architecture consistency audit (see
`docs/IMPLEMENTATION_STATUS.md` §3).

## Context

Section 1 states "End-to-end encryption even when traffic traverses a
relay" as a core product goal. Principle 3.5 (relay blindness) requires
relay nodes to forward encrypted traffic without possessing keys needed to
decrypt desktop content, clipboard, files, or input. Section 16 lists
exactly what a relay must not know. QUIC (ADR-003) already provides
transport-level TLS 1.3, but that protects each hop (client↔relay,
relay↔agent) independently — it does not by itself guarantee the relay
cannot see plaintext, since a relay could in principle terminate TLS on
each side.

## Decision

Beyond QUIC's mandatory transport-level TLS 1.3, Nexus applies its own
application-level end-to-end encryption between client and agent (Section
17): mutual device/session identity authentication, ephemeral X25519 key
exchange, HKDF-SHA256 key derivation, and ChaCha20-Poly1305/AES-GCM AEAD
encryption of payloads, with separate directional/channel keys. This layer
is independent of, and on top of, whatever transport security exists
between each hop, so a relay forwarding QUIC packets never holds the keys
needed to decrypt session content.

## Consequences

**Positive**
- Relay compromise, subpoena, or misconfiguration does not expose desktop
  content, clipboard, files, or input — directly satisfies the "Rogue
  relay" mitigation in the Section 44 threat model.
- Matches Section 1's explicit product goal and is a meaningful
  differentiator for the self-hosting trust model: a self-hosted relay
  operator, potentially on a lower-trust network segment, cannot see session
  content even with full access to the relay process.
- Composes cleanly with the signed-capability authorization model (ADR-005):
  identity verification during the Section 13 handshake and the E2E key
  exchange share the same device-identity trust anchors.

**Negative / follow-up work**
- Double encryption overhead (QUIC/TLS plus application-level AEAD) —
  Section 61.2's long-term row already identifies hardware-accelerated
  AES-NI/ARM Crypto extensions as the mitigation path for this cost at
  scale.
- The exact framing granularity at which application-level AEAD is applied
  (per QUIC datagram vs. per encoded frame payload) is flagged in Section
  58 as still needing its own ADR — this ADR establishes that E2E
  encryption exists and where its trust boundary sits, not the wire-level
  framing detail, which remains open follow-up work (natural home: Section
  17, per the note added to Section 58 in this review).

## Alternatives considered

**Rely on QUIC/TLS alone, with the relay as a TLS-terminating proxy.**
Simpler; no second encryption layer. Rejected outright: this is precisely
what Principle 3.5 forbids. A TLS-terminating relay sees plaintext,
directly violating the "rogue relay" mitigation and the self-hosting trust
model, where a self-hosted relay operator must not be able to view desktop
content.

**QUIC transport encryption "passed through" without an additional
application layer**, i.e. relying on the relay never terminating TLS as an
operational promise rather than a cryptographic guarantee. Rejected: this
is an operational assumption, not a verifiable property — nothing prevents
a compromised or malicious relay from terminating and re-originating
separate QUIC/TLS sessions to each endpoint (a connection split), unless the
endpoints cryptographically bind to each other independently of the relay.
The application-level E2E layer, verified against device identities
established in Section 12/13, is what makes relay-blindness a guarantee
rather than a policy.

## References

Spec Sections 1, 3.5, 12, 13, 16, 17, 44, 58, 61.2. ADR-003, ADR-005.
