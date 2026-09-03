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
   domain. Video wire-header version 2 carries that low 64-bit value as
   `nonce_sequence` in every fragment, so a receiver can derive the nonce for
   a complete frame without depending on datagram arrival order.
4. Authenticate, but do not encrypt, stable frame metadata (session-bound
   channel, frame ID, codec/config identifier, protocol version,
   `timestamp_us`, and the `KEYFRAME` flag) as AAD. Packet IDs and packet
   counts are excluded because the packetizer may create or retransmit
   fragments independently.
5. The receiver authenticates and decrypts only after reassembling the full
   frame ciphertext. Authentication failure drops the frame; it does not
   reveal plaintext or permit partial decode. Freshness and nonce replay state
   are committed only after authentication. Receivers retain a bounded 4096
   sequence replay window: out-of-order frames in that window are allowed once,
   while duplicate or too-old sequences are rejected.

## Wire Compatibility

Video packet version 2 has a 30-byte fixed header: the former 22-byte layout
plus the big-endian `nonce_sequence: u64` after `timestamp_us`. Version 1 does
not carry enough information for loss/reorder-safe nonce selection and is
rejected rather than interpreted as version 2. Peers must negotiate/support
version 2 before exchanging encrypted video.

## Consequences

- The relay sees ciphertext size and packet metadata but no encoded frame
  plaintext.
- Frame loss discards one complete encrypted frame; packet-level repair does
  not require nonce reuse or partial AEAD state.
- Timestamp/keyframe presentation metadata cannot be changed in transit
  without causing AEAD authentication failure.
- A forged frame cannot advance the authenticated frame or nonce freshness
  watermarks; a correctly sealed duplicate nonce sequence is rejected.
- A future control/input channel can reuse the contract with a distinct
  direction/channel domain and key, without sharing nonce space.
- A separate implementation step must add the nonce allocator, metadata AAD
  encoding, and encrypted-frame integration tests.

## Alternatives rejected

- **Per-datagram AEAD:** couples packet loss/retransmission and packetizer
  metadata to cryptographic nonce management.
- **One long-lived stream cipher state:** makes frame loss and reconnect
  recovery fragile and complicates independent channel keys.
