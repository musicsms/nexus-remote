# ADR-025: Encoded-Frame Payload AEAD Framing

## Status

Accepted — 2026-08-26.

## Context

ADR-006 establishes application-level E2E encryption but intentionally leaves
the framing granularity open. Encrypting each QUIC datagram would expose the
packetizer to ciphertext/tag overhead and make packet loss handling part of
the cryptographic contract. Encrypting a complete encoded frame before
fragmentation keeps the relay blind while allowing the existing Section 21
packetizer to fragment opaque bytes.

## Decision

1. Apply one ChaCha20-Poly1305 operation to each encoded-frame payload before
   Section 21 packetization. The resulting ciphertext plus 16-byte tag is the
   opaque frame payload carried by video packets.
2. Derive separate directional/channel keys from the session root key. The
   crypto primitive layer remains responsible only for key derivation and
   AEAD; session/transport owns direction and channel labels.
3. Construct the 96-bit nonce from a per-direction monotonically increasing
   frame sequence. The sender must never reuse a `(key, nonce)` pair; sequence
   exhaustion is a fatal session error. The sequence is encoded big-endian in
   the low 64 bits, with the high 32 bits reserved for the channel/direction
   domain.
4. Authenticate, but do not encrypt, stable frame metadata (session-bound
   channel, frame ID, codec/config identifier, and protocol version) as AAD.
   Packet IDs and packet counts are excluded because the packetizer may create
   or retransmit fragments independently.
5. The receiver authenticates and decrypts only after reassembling the full
   frame ciphertext. Authentication failure drops the frame; it does not
   reveal plaintext or permit partial decode.

## Consequences

- The relay sees ciphertext size and packet metadata but no encoded frame
  plaintext.
- Frame loss discards one complete encrypted frame; packet-level repair does
  not require nonce reuse or partial AEAD state.
- A future control/input channel can reuse the contract with a distinct
  direction/channel domain and key, without sharing nonce space.
- A separate implementation step must add the nonce allocator, metadata AAD
  encoding, and encrypted-frame integration tests.

## Alternatives rejected

- **Per-datagram AEAD:** couples packet loss/retransmission and packetizer
  metadata to cryptographic nonce management.
- **One long-lived stream cipher state:** makes frame loss and reconnect
  recovery fragile and complicates independent channel keys.
