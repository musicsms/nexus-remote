# ADR-016: Bind the Agent's Advertised Protocol Range Into the Session Capability

## Status

Accepted.

## Context

Spec Section 31 defines protocol version negotiation
(`client_min/max_protocol`, `agent_min/max_protocol`,
`negotiated_protocol`). Section 9 has the agent report its capability set
(including its supported protocol range) to `nexusd` continuously over the
presence channel. Section 13's mutual identity-proof handshake happens
between client and agent directly, after the client already holds a signed
`SessionCapability` (Section 12) issued by the control plane — but nothing
in that capability currently constrains which protocol range the live
handshake is allowed to negotiate down to.

This gap was identified during a system-design review of the session
establishment and signaling flow
(`docs/protocol/session-establishment-signaling.md`, item g): because
`nexusd` already knows the agent's protocol range at capability-issuance
time (from presence), an attacker positioned on the client↔agent path during
CONNECTING could attempt to force the live Section 31 negotiation to an
older/weaker protocol version, since that negotiation is currently
independent of anything the control plane signed. This touches ADR-005
(signed capability-based session authorization) and ADR-010 (protocol core
stays OS-independent), since closing it is a wire-schema decision, not an
implementation detail.

## Decision

1. `SessionCapability` (Section 12, Section 33 Protobuf schema) gains two new
   signed fields: `agent_min_protocol` and `agent_max_protocol` (both
   `uint32`), populated by the control plane from the agent's
   most-recently-reported protocol range (Section 9) at the moment the
   capability is issued.
2. During the Section 13 mutual identity-proof handshake, the agent verifies
   that the `negotiated_protocol` actually agreed for the session
   (Section 31) falls within the `agent_min_protocol..agent_max_protocol`
   range carried in the capability the client presented — not only within
   whatever range the live handshake claims. A negotiated version outside
   that signed range is rejected; the session transitions to
   `FAILED`/`DENIED` rather than proceeding, and the rejection is logged as a
   security-relevant audit event (Section 35), not a silent failure.
3. This closes the gap without moving the live Section 31 negotiation itself
   onto a signed channel — only the acceptable *range* needs to be pinned at
   issuance time; the negotiated value within that range is still agreed live
   between client and agent as today.
4. No change to the establishment-TTL/session-duration split from ADR-014 —
   this is an orthogonal field on the same capability structure.

## Consequences

**Positive**
- Closes a real downgrade vector in the mutual-identity-proof handshake
  without requiring `nexusd` to stay on the critical path during CONNECTING
  (consistent with the design principle already established in
  `docs/protocol/session-establishment-signaling.md`: "nexusd is off the
  critical path once CONNECTING starts").
- Small, additive wire-schema change — two new fields on an existing
  message, no restructuring of `SessionHello` or the handshake flow.
- Matches the capability-binding pattern already used for
  `client_device_id`/`target_device_id`/nonce in Section 12: express trust
  boundaries as signed fields the agent verifies locally.

**Negative / follow-up work**
- The control plane must keep capability issuance reasonably in sync with
  the agent's live-reported protocol range; a stale presence snapshot (agent
  updated its binary after its last heartbeat but before a new capability is
  issued) could cause a spurious rejection. Document an acceptable staleness
  tolerance — the same class of problem as the clock-skew tolerance flagged
  in `docs/protocol/session-establishment-signaling.md` item h.
- `nexus-protocol`'s `SessionCapability` schema (Section 33) needs the two
  new fields added under Epic A, before the wire format is frozen for Phase
  1, per the Definition of Done for a protocol feature (Section 56).
- Needs an explicit adversarial test: a forged/replayed handshake attempting
  to negotiate a version outside the signed range must be rejected — add to
  the malformed-input test set already required by Section 56 and the fuzz
  targets in Epic A.
- Spec Section 12 has been updated to list `agent_min_protocol` /
  `agent_max_protocol` in the `SessionCapability` struct and cross-reference
  this ADR.

## Alternatives considered

**Rely on QUIC/TLS 1.3's built-in downgrade resistance alone.** Rejected as
sufficient on its own: TLS protects the transport handshake, not the
*application-level* feature/version negotiation defined in Section 31, which
happens inside the already-established TLS channel. A MITM that cannot
break TLS is not the only threat model here (Principle 3.4's
endpoint-verifiable-authorization posture assumes defense in depth beyond
transport security alone) — binding the range at the capability layer
protects the application protocol's own guarantee, independent of transport
security.

**Sign the live `negotiated_protocol` value itself at connect time, rather
than binding a range at issuance.** Rejected: would require a live signing
operation from the control plane during the handshake, putting `nexusd`
back on the critical path for every session establishment — directly
contradicting the already-established design goal that a control-plane
restart mid-CONNECTING should not break an in-flight handshake.

**Reinterpret an existing `SessionCapability` field instead of adding two new
ones**, the way ADR-014 reused `expires_at`/`restrictions.max_duration`
rather than adding a field. Rejected: unlike ADR-014's case, no existing
field in Section 12's structure carries anything resembling a protocol
version range — there is nothing to reinterpret, so a schema addition is the
smallest change available here, not a shortcut around one.

## References

Spec Sections 9, 12, 13, 31, 33, 35, 56. ADR-005, ADR-010, ADR-014.
`docs/protocol/session-establishment-signaling.md` (item g).
