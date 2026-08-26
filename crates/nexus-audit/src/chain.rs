//! Tamper-evident cryptographic hash chain for audit events.
//!
//! Implements cryptographic hash chaining according to Spec Section 35:
//! `hash_n = BLAKE3(sequence || canonical_event_bytes || hash_n-1)`.
//!
//! Provides [`ChainedAuditEvent`], [`AuditChain`], and [`verify_chain`] for
//! end-to-end audit log integrity and tamper detection.

use serde::{Deserialize, Serialize};

use crate::event::AuditEvent;

/// Default genesis seed / hash when none is explicitly provided (64 zeros hex).
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Errors that can occur during chain verification or chain operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainVerificationError {
    /// Sequence number is not strictly sequential (1, 2, 3, ...).
    #[error("non-monotonic sequence number: expected {expected}, got {actual}")]
    NonMonotonicSequence {
        /// The expected sequence number.
        expected: u64,
        /// The actual sequence number found.
        actual: u64,
    },

    /// The computed hash for the event does not match the recorded hash.
    #[error("hash mismatch at sequence {sequence}: expected {expected}, got {actual}")]
    HashMismatch {
        /// Sequence number of the corrupted event.
        sequence: u64,
        /// Expected recomputed cryptographic hash.
        expected: String,
        /// Actual recorded hash.
        actual: String,
    },

    /// The `previous_hash` of an event does not match the previous event's hash (or genesis).
    #[error("previous hash mismatch at sequence {sequence}: expected {expected}, got {actual}")]
    PreviousHashMismatch {
        /// Sequence number of the event with invalid previous hash.
        sequence: u64,
        /// Expected previous hash.
        expected: String,
        /// Actual recorded previous hash.
        actual: String,
    },

    /// Error during canonical serialization of the audit event.
    #[error("serialization error: {0}")]
    SerializationError(String),
}

/// Alias for chain operations.
pub type ChainError = ChainVerificationError;

/// An audit event wrapped with cryptographic chain linkage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainedAuditEvent {
    /// 1-indexed monotonic sequence number.
    pub sequence: u64,
    /// The underlying audit event payload.
    pub event: AuditEvent,
    /// Hex-encoded cryptographic hash of the previous event (or genesis seed).
    pub previous_hash: String,
    /// Hex-encoded BLAKE3 hash of `sequence || canonical_event_bytes || previous_hash`.
    pub hash: String,
}

impl ChainedAuditEvent {
    /// Returns the 1-indexed sequence number.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns a reference to the wrapped [`AuditEvent`].
    #[must_use]
    pub fn event(&self) -> &AuditEvent {
        &self.event
    }

    /// Returns the hex string of the previous event's hash.
    #[must_use]
    pub fn previous_hash(&self) -> &str {
        &self.previous_hash
    }

    /// Returns the hex string of this event's hash.
    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }
}

/// Computes the BLAKE3 hash over `sequence || canonical_bytes || previous_hash`.
#[must_use]
pub fn compute_hash(sequence: u64, canonical_bytes: &[u8], previous_hash: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&sequence.to_be_bytes());
    hasher.update(canonical_bytes);
    hasher.update(previous_hash.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Computes the cryptographic hash for a given sequence number, event, and previous hash.
///
/// # Errors
/// Returns [`ChainVerificationError::SerializationError`] if canonical serialization fails.
pub fn compute_event_hash(
    sequence: u64,
    event: &AuditEvent,
    previous_hash: &str,
) -> Result<String, ChainVerificationError> {
    let canonical_bytes = event
        .canonical_bytes()
        .map_err(|e| ChainVerificationError::SerializationError(e.to_string()))?;
    Ok(compute_hash(sequence, &canonical_bytes, previous_hash))
}

/// In-memory cryptographic audit chain builder and state tracker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditChain {
    genesis_seed: String,
    last_hash: String,
    sequence: u64,
    events: Vec<ChainedAuditEvent>,
}

impl Default for AuditChain {
    fn default() -> Self {
        Self::new(None)
    }
}

impl AuditChain {
    /// Creates a new audit chain with an optional custom genesis seed.
    ///
    /// If `genesis_seed` is `None`, [`GENESIS_HASH`] is used as the initial hash.
    #[must_use]
    pub fn new(genesis_seed: Option<&str>) -> Self {
        let initial = genesis_seed.unwrap_or(GENESIS_HASH).to_string();
        Self {
            genesis_seed: initial.clone(),
            last_hash: initial,
            sequence: 0,
            events: Vec::new(),
        }
    }

    /// Appends an audit event to the chain, generating sequence and cryptographic hash.
    ///
    /// # Errors
    /// Returns [`ChainError`] if serializing the event payload fails.
    pub fn append(&mut self, event: AuditEvent) -> Result<ChainedAuditEvent, ChainError> {
        let next_seq = self.sequence + 1;
        let prev_hash = self.last_hash.clone();
        let hash = compute_event_hash(next_seq, &event, &prev_hash)?;

        let chained = ChainedAuditEvent {
            sequence: next_seq,
            event,
            previous_hash: prev_hash,
            hash: hash.clone(),
        };

        self.sequence = next_seq;
        self.last_hash = hash;
        self.events.push(chained.clone());

        Ok(chained)
    }

    /// Returns the latest cryptographic hash in the chain (or genesis hash if empty).
    #[must_use]
    pub fn last_hash(&self) -> &str {
        &self.last_hash
    }

    /// Returns the current sequence number (0 if empty).
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the number of events in the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns `true` if the chain contains no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Returns a slice of all chained audit events.
    #[must_use]
    pub fn events(&self) -> &[ChainedAuditEvent] {
        &self.events
    }

    /// Consumes the chain and returns the vector of chained audit events.
    #[must_use]
    pub fn into_events(self) -> Vec<ChainedAuditEvent> {
        self.events
    }

    /// Returns the genesis seed/hash used for this chain.
    #[must_use]
    pub fn genesis_seed(&self) -> &str {
        &self.genesis_seed
    }
}

/// Verifies the cryptographic integrity of an audit event chain slice.
///
/// Verifies:
/// 1. An empty slice is valid.
/// 2. The first event has `sequence == 1` and `previous_hash == genesis_seed.unwrap_or(GENESIS_HASH)`.
/// 3. For every event `i`: `events[i].sequence == i + 1`.
/// 4. For every event `i > 0`: `events[i].previous_hash == events[i - 1].hash`.
/// 5. For every event: recomputed `BLAKE3(sequence || canonical_bytes || previous_hash) == events[i].hash`.
///
/// # Errors
/// Returns [`ChainVerificationError`] if any verification condition is violated.
pub fn verify_chain(
    events: &[ChainedAuditEvent],
    genesis_seed: Option<&str>,
) -> Result<(), ChainVerificationError> {
    if events.is_empty() {
        return Ok(());
    }

    let expected_genesis = genesis_seed.unwrap_or(GENESIS_HASH);

    for (i, chained) in events.iter().enumerate() {
        let expected_seq = (i as u64) + 1;

        if chained.sequence != expected_seq {
            return Err(ChainVerificationError::NonMonotonicSequence {
                expected: expected_seq,
                actual: chained.sequence,
            });
        }

        let expected_prev = if i == 0 {
            expected_genesis
        } else {
            &events[i - 1].hash
        };

        if chained.previous_hash != expected_prev {
            return Err(ChainVerificationError::PreviousHashMismatch {
                sequence: chained.sequence,
                expected: expected_prev.to_string(),
                actual: chained.previous_hash.clone(),
            });
        }

        let recomputed_hash =
            compute_event_hash(chained.sequence, &chained.event, &chained.previous_hash)?;

        if chained.hash != recomputed_hash {
            return Err(ChainVerificationError::HashMismatch {
                sequence: chained.sequence,
                expected: recomputed_hash,
                actual: chained.hash.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::AuditEventType;
    use nexus_common::id::{DeviceId, SessionId, TenantId, UserId};
    use nexus_common::time::UnixTimestamp;

    fn sample_event(id: &str, event_type: AuditEventType) -> AuditEvent {
        AuditEvent::new(
            id,
            UnixTimestamp::from_secs(1_700_000_000),
            TenantId::new("tenant-corp-1").unwrap(),
            event_type,
        )
        .with_user_id(UserId::new("user-alice").unwrap())
        .with_device_id(DeviceId::new("device-laptop-1").unwrap())
        .with_session_id(SessionId::new("session-xyz-100").unwrap())
        .with_metadata(serde_json::json!({
            "ip_address": "192.168.1.50",
            "action": "auth_success"
        }))
    }

    #[test]
    fn test_empty_chain_and_default() {
        let chain = AuditChain::default();
        assert_eq!(chain.len(), 0);
        assert!(chain.is_empty());
        assert_eq!(chain.sequence(), 0);
        assert_eq!(chain.last_hash(), GENESIS_HASH);
        assert_eq!(chain.genesis_seed(), GENESIS_HASH);
        assert!(chain.events().is_empty());

        assert!(verify_chain(&[], None).is_ok());
        assert!(verify_chain(&[], Some("custom-seed")).is_ok());
    }

    #[test]
    fn test_incremental_appending_and_sequential_hashing() {
        let mut chain = AuditChain::new(None);

        let ev1 = sample_event("evt-1", AuditEventType::UserLogin);
        let chained1 = chain.append(ev1).expect("append ev1");

        assert_eq!(chained1.sequence(), 1);
        assert_eq!(chained1.previous_hash(), GENESIS_HASH);
        assert_eq!(chain.sequence(), 1);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.last_hash(), chained1.hash());
        assert_eq!(chain.events()[0], chained1);

        let ev2 = sample_event("evt-2", AuditEventType::SessionStart);
        let chained2 = chain.append(ev2).expect("append ev2");

        assert_eq!(chained2.sequence(), 2);
        assert_eq!(chained2.previous_hash(), chained1.hash());
        assert_eq!(chain.sequence(), 2);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain.last_hash(), chained2.hash());

        let ev3 = sample_event("evt-3", AuditEventType::ClipboardWrite);
        let chained3 = chain.append(ev3).expect("append ev3");

        assert_eq!(chained3.sequence(), 3);
        assert_eq!(chained3.previous_hash(), chained2.hash());
        assert_eq!(chain.sequence(), 3);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain.last_hash(), chained3.hash());

        // Verify full valid chain
        assert!(verify_chain(chain.events(), None).is_ok());
    }

    #[test]
    fn test_custom_genesis_seed() {
        let custom_seed = "custom_genesis_seed_1234567890abcdef";
        let mut chain = AuditChain::new(Some(custom_seed));

        assert_eq!(chain.last_hash(), custom_seed);
        assert_eq!(chain.genesis_seed(), custom_seed);

        let ev1 = sample_event("evt-1", AuditEventType::DeviceEnroll);
        let chained1 = chain.append(ev1).expect("append ev1");

        assert_eq!(chained1.previous_hash(), custom_seed);

        assert!(verify_chain(chain.events(), Some(custom_seed)).is_ok());

        // Verifying with None or wrong seed should fail
        let err = verify_chain(chain.events(), None).unwrap_err();
        match err {
            ChainVerificationError::PreviousHashMismatch {
                sequence,
                expected,
                actual,
            } => {
                assert_eq!(sequence, 1);
                assert_eq!(expected, GENESIS_HASH);
                assert_eq!(actual, custom_seed);
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }
    }

    #[test]
    fn test_tamper_payload_mutation_fails_verification() {
        let mut chain = AuditChain::new(None);
        chain
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();
        chain
            .append(sample_event("evt-2", AuditEventType::SessionStart))
            .unwrap();
        chain
            .append(sample_event("evt-3", AuditEventType::SessionEnd))
            .unwrap();

        let mut events = chain.into_events();

        // Mutate event payload at index 1 (change user_id)
        events[1].event.user_id = Some(UserId::new("user-attacker-mallory").unwrap());

        let err = verify_chain(&events, None).unwrap_err();
        match err {
            ChainVerificationError::HashMismatch {
                sequence,
                expected,
                actual,
            } => {
                assert_eq!(sequence, 2);
                assert_ne!(expected, actual);
                assert_eq!(actual, events[1].hash);
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }

        // Test mutating event_type directly
        let mut chain2 = AuditChain::new(None);
        chain2
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();
        let mut events2 = chain2.into_events();
        events2[0].event.event_type = AuditEventType::PolicyUpdate;

        let err2 = verify_chain(&events2, None).unwrap_err();
        assert!(matches!(
            err2,
            ChainVerificationError::HashMismatch { sequence: 1, .. }
        ));
    }

    #[test]
    fn test_tamper_event_reordering_fails_verification() {
        let mut chain = AuditChain::new(None);
        chain
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();
        chain
            .append(sample_event("evt-2", AuditEventType::SessionStart))
            .unwrap();
        chain
            .append(sample_event("evt-3", AuditEventType::SessionEnd))
            .unwrap();

        let mut events = chain.into_events();

        // Swap event 0 and event 1
        events.swap(0, 1);

        let err = verify_chain(&events, None).unwrap_err();
        match err {
            ChainVerificationError::NonMonotonicSequence { expected, actual } => {
                assert_eq!(expected, 1);
                assert_eq!(actual, 2);
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }
    }

    #[test]
    fn test_tamper_dropping_event_creates_sequence_gap() {
        let mut chain = AuditChain::new(None);
        chain
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();
        chain
            .append(sample_event("evt-2", AuditEventType::SessionStart))
            .unwrap();
        chain
            .append(sample_event("evt-3", AuditEventType::SessionEnd))
            .unwrap();

        let mut events = chain.into_events();

        // Drop middle event (evt-2 at index 1)
        events.remove(1);

        let err = verify_chain(&events, None).unwrap_err();
        match err {
            ChainVerificationError::NonMonotonicSequence { expected, actual } => {
                assert_eq!(expected, 2);
                assert_eq!(actual, 3);
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }
    }

    #[test]
    fn test_tamper_previous_hash_fails_verification() {
        let mut chain = AuditChain::new(None);
        chain
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();
        chain
            .append(sample_event("evt-2", AuditEventType::SessionStart))
            .unwrap();

        let mut events = chain.into_events();

        // Tamper with previous_hash of event 1
        events[1].previous_hash =
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string();

        let err = verify_chain(&events, None).unwrap_err();
        match err {
            ChainVerificationError::PreviousHashMismatch {
                sequence,
                expected,
                actual,
            } => {
                assert_eq!(sequence, 2);
                assert_eq!(expected, events[0].hash);
                assert_eq!(
                    actual,
                    "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                );
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }
    }

    #[test]
    fn test_tamper_sequence_fails_verification() {
        let mut chain = AuditChain::new(None);
        chain
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();

        let mut events = chain.into_events();
        events[0].sequence = 999;

        let err = verify_chain(&events, None).unwrap_err();
        match err {
            ChainVerificationError::NonMonotonicSequence { expected, actual } => {
                assert_eq!(expected, 1);
                assert_eq!(actual, 999);
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }
    }

    #[test]
    fn test_tamper_hash_directly_fails_verification() {
        let mut chain = AuditChain::new(None);
        chain
            .append(sample_event("evt-1", AuditEventType::UserLogin))
            .unwrap();

        let mut events = chain.into_events();
        events[0].hash =
            "1111111111111111111111111111111111111111111111111111111111111111".to_string();

        let err = verify_chain(&events, None).unwrap_err();
        match err {
            ChainVerificationError::HashMismatch { sequence, .. } => {
                assert_eq!(sequence, 1);
            }
            _ => panic!("unexpected error variant: {err:?}"),
        }
    }

    #[test]
    fn test_chained_event_serde_roundtrip() {
        let mut chain = AuditChain::new(None);
        let original = chain
            .append(sample_event("evt-100", AuditEventType::AccessApprove))
            .unwrap();

        let json = serde_json::to_string(&original).expect("serialize chained event");
        let deserialized: ChainedAuditEvent =
            serde_json::from_str(&json).expect("deserialize chained event");

        assert_eq!(original, deserialized);

        let slice = vec![deserialized];
        assert!(verify_chain(&slice, None).is_ok());
    }
}
