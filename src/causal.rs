//! Causal metadata shared by offline clients and sync services.
//!
//! JSON reconciliation decides how two values combine. This module answers the
//! separate ordering question: whether one mutation happened before, after, or
//! concurrently with another mutation. The implementation is deterministic,
//! bounded, clock-free, and serializable across native and WebAssembly hosts.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Wire schema emitted by [`CausalEnvelope`].
pub const CAUSAL_SCHEMA_VERSION: &str = "opto-sync.causal.v1";
/// Maximum replica entries accepted in one vector clock.
pub const MAX_CAUSAL_REPLICAS: usize = 1_024;
const MAX_REPLICA_ID_BYTES: usize = 128;
const MAX_DOCUMENT_ID_BYTES: usize = 512;
const MAX_MUTATION_ID_BYTES: usize = 256;

/// Relative causal ordering of two version vectors.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionRelation {
    /// Both vectors contain the same counters.
    Equal,
    /// Every local counter is less than or equal to the remote vector and at
    /// least one counter is lower.
    Before,
    /// Every local counter is greater than or equal to the remote vector and at
    /// least one counter is higher.
    After,
    /// Each vector contains at least one counter not dominated by the other.
    Concurrent,
}

/// How an incoming envelope relates to the receiver's durable checkpoint.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CausalDisposition {
    /// The receiver has already observed exactly this causal frontier.
    Duplicate,
    /// The incoming mutation is behind the receiver and can be ignored.
    Stale,
    /// The mutation advances the receiver without a causal conflict.
    Apply,
    /// The mutation is concurrent and requires the configured conflict policy.
    ResolveConcurrent,
}

/// A bounded vector clock indexed by stable replica identifier.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct VersionVector {
    entries: BTreeMap<String, u64>,
}

impl VersionVector {
    /// Creates an empty vector clock.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Creates a validated vector from explicit counters.
    ///
    /// # Errors
    ///
    /// Returns [`VersionVectorError`] for invalid replica identifiers, zero
    /// counters, or more than [`MAX_CAUSAL_REPLICAS`] entries.
    pub fn from_entries(
        entries: impl IntoIterator<Item = (String, u64)>,
    ) -> Result<Self, VersionVectorError> {
        let mut vector = Self::new();
        for (replica_id, counter) in entries {
            vector.observe(&replica_id, counter)?;
        }
        Ok(vector)
    }

    /// Returns the counter observed for one replica, or zero when absent.
    #[must_use]
    pub fn get(&self, replica_id: &str) -> u64 {
        self.entries.get(replica_id).copied().unwrap_or(0)
    }

    /// Returns the number of replicas represented by the vector.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reports whether the vector contains no observations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over counters in deterministic replica-id order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, u64)> {
        self.entries
            .iter()
            .map(|(replica_id, counter)| (replica_id.as_str(), *counter))
    }

    /// Advances one local replica counter and returns the new value.
    ///
    /// # Errors
    ///
    /// Returns [`VersionVectorError::CounterOverflow`] at `u64::MAX`, or a
    /// validation error when adding a new replica would exceed the bounds.
    pub fn increment(&mut self, replica_id: &str) -> Result<u64, VersionVectorError> {
        validate_replica_id(replica_id)?;
        if !self.entries.contains_key(replica_id) && self.entries.len() >= MAX_CAUSAL_REPLICAS {
            return Err(VersionVectorError::TooManyReplicas {
                maximum: MAX_CAUSAL_REPLICAS,
            });
        }
        let current = self.get(replica_id);
        let next = current
            .checked_add(1)
            .ok_or_else(|| VersionVectorError::CounterOverflow(replica_id.to_owned()))?;
        self.entries.insert(replica_id.to_owned(), next);
        Ok(next)
    }

    /// Observes an explicit positive counter, retaining the larger value.
    ///
    /// Replaying an older observation is therefore idempotent.
    pub fn observe(
        &mut self,
        replica_id: &str,
        counter: u64,
    ) -> Result<bool, VersionVectorError> {
        validate_replica_id(replica_id)?;
        if counter == 0 {
            return Err(VersionVectorError::ZeroCounter(replica_id.to_owned()));
        }
        if !self.entries.contains_key(replica_id) && self.entries.len() >= MAX_CAUSAL_REPLICAS {
            return Err(VersionVectorError::TooManyReplicas {
                maximum: MAX_CAUSAL_REPLICAS,
            });
        }
        if self.get(replica_id) >= counter {
            return Ok(false);
        }
        self.entries.insert(replica_id.to_owned(), counter);
        Ok(true)
    }

    /// Merges another vector by taking the maximum counter for every replica.
    ///
    /// Returns whether the local vector changed.
    pub fn merge(&mut self, other: &Self) -> Result<bool, VersionVectorError> {
        let mut changed = false;
        for (replica_id, counter) in other.iter() {
            changed |= self.observe(replica_id, counter)?;
        }
        Ok(changed)
    }

    /// Compares two vector clocks using the standard partial order.
    #[must_use]
    pub fn relation(&self, other: &Self) -> VersionRelation {
        let replicas = self
            .entries
            .keys()
            .chain(other.entries.keys())
            .collect::<BTreeSet<_>>();
        let mut less = false;
        let mut greater = false;

        for replica_id in replicas {
            let local = self.get(replica_id);
            let remote = other.get(replica_id);
            less |= local < remote;
            greater |= local > remote;
            if less && greater {
                return VersionRelation::Concurrent;
            }
        }

        match (less, greater) {
            (false, false) => VersionRelation::Equal,
            (true, false) => VersionRelation::Before,
            (false, true) => VersionRelation::After,
            (true, true) => VersionRelation::Concurrent,
        }
    }

    /// Reports whether this vector is equal to or causally after `other`.
    #[must_use]
    pub fn dominates(&self, other: &Self) -> bool {
        matches!(
            self.relation(other),
            VersionRelation::Equal | VersionRelation::After
        )
    }
}

impl Serialize for VersionVector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.entries.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VersionVector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = BTreeMap::<String, u64>::deserialize(deserializer)?;
        Self::from_entries(entries).map_err(D::Error::custom)
    }
}

/// Version-vector validation or arithmetic error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum VersionVectorError {
    /// Replica identifiers are bounded ASCII tokens suitable for every client.
    InvalidReplicaId(String),
    /// Serialized vectors must omit zero-valued entries.
    ZeroCounter(String),
    /// A local replica counter reached `u64::MAX`.
    CounterOverflow(String),
    /// The vector exceeded its cross-client memory bound.
    TooManyReplicas {
        /// Maximum supported unique replica identifiers.
        maximum: usize,
    },
}

impl Display for VersionVectorError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidReplicaId(replica_id) => write!(
                formatter,
                "replica id {replica_id:?} must be 1..={MAX_REPLICA_ID_BYTES} ASCII bytes using letters, digits, '.', '_', ':', or '-'"
            ),
            Self::ZeroCounter(replica_id) => {
                write!(formatter, "replica {replica_id:?} has a zero counter")
            }
            Self::CounterOverflow(replica_id) => {
                write!(formatter, "replica {replica_id:?} counter overflowed")
            }
            Self::TooManyReplicas { maximum } => {
                write!(formatter, "version vector exceeds {maximum} replicas")
            }
        }
    }
}

impl Error for VersionVectorError {}

/// Payload carried by a causal mutation envelope.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CausalOperation<T> {
    /// Create or replace/reconcile a document value.
    Upsert(T),
    /// Delete a document while retaining a causally ordered tombstone.
    Delete,
}

/// A transport-neutral mutation with an immutable causal snapshot.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CausalEnvelope<T> {
    /// Wire schema discriminator.
    pub schema_version: String,
    /// Stable logical document key.
    pub document_id: String,
    /// Client-generated idempotency key.
    pub mutation_id: String,
    /// Replica that advanced the vector for this mutation.
    pub replica_id: String,
    /// Vector clock after the local mutation was recorded.
    pub clock: VersionVector,
    /// Upsert payload or delete tombstone.
    pub operation: CausalOperation<T>,
}

impl<T> CausalEnvelope<T> {
    /// Creates an upsert envelope and advances `clock` for `replica_id`.
    pub fn upsert(
        document_id: impl Into<String>,
        mutation_id: impl Into<String>,
        replica_id: impl Into<String>,
        clock: &mut VersionVector,
        payload: T,
    ) -> Result<Self, CausalEnvelopeError> {
        Self::new(
            document_id,
            mutation_id,
            replica_id,
            clock,
            CausalOperation::Upsert(payload),
        )
    }

    /// Creates a delete tombstone and advances `clock` for `replica_id`.
    pub fn delete(
        document_id: impl Into<String>,
        mutation_id: impl Into<String>,
        replica_id: impl Into<String>,
        clock: &mut VersionVector,
    ) -> Result<Self, CausalEnvelopeError> {
        Self::new(
            document_id,
            mutation_id,
            replica_id,
            clock,
            CausalOperation::Delete,
        )
    }

    fn new(
        document_id: impl Into<String>,
        mutation_id: impl Into<String>,
        replica_id: impl Into<String>,
        clock: &mut VersionVector,
        operation: CausalOperation<T>,
    ) -> Result<Self, CausalEnvelopeError> {
        let document_id = document_id.into();
        let mutation_id = mutation_id.into();
        let replica_id = replica_id.into();
        validate_document_id(&document_id)?;
        validate_mutation_id(&mutation_id)?;
        validate_replica_id(&replica_id)?;
        clock.increment(&replica_id)?;

        Ok(Self {
            schema_version: CAUSAL_SCHEMA_VERSION.to_owned(),
            document_id,
            mutation_id,
            replica_id,
            clock: clock.clone(),
            operation,
        })
    }

    /// Validates an envelope received across a trust boundary.
    pub fn validate(&self) -> Result<(), CausalEnvelopeError> {
        if self.schema_version != CAUSAL_SCHEMA_VERSION {
            return Err(CausalEnvelopeError::UnsupportedSchema(
                self.schema_version.clone(),
            ));
        }
        validate_document_id(&self.document_id)?;
        validate_mutation_id(&self.mutation_id)?;
        validate_replica_id(&self.replica_id)?;
        if self.clock.get(&self.replica_id) == 0 {
            return Err(CausalEnvelopeError::MissingActorCounter(
                self.replica_id.clone(),
            ));
        }
        Ok(())
    }

    /// Classifies this mutation against a receiver's durable causal checkpoint.
    #[must_use]
    pub fn disposition_against(&self, checkpoint: &VersionVector) -> CausalDisposition {
        match self.clock.relation(checkpoint) {
            VersionRelation::Equal => CausalDisposition::Duplicate,
            VersionRelation::Before => CausalDisposition::Stale,
            VersionRelation::After => CausalDisposition::Apply,
            VersionRelation::Concurrent => CausalDisposition::ResolveConcurrent,
        }
    }

    /// Merges this envelope's clock into a receiver after the mutation is
    /// accepted or its concurrent conflict has been resolved.
    pub fn acknowledge_into(
        &self,
        checkpoint: &mut VersionVector,
    ) -> Result<bool, VersionVectorError> {
        checkpoint.merge(&self.clock)
    }

    /// Reports whether the envelope carries a delete tombstone.
    #[must_use]
    pub fn is_delete(&self) -> bool {
        matches!(self.operation, CausalOperation::Delete)
    }

    /// Returns the local counter assigned by the envelope's originating replica.
    #[must_use]
    pub fn actor_counter(&self) -> u64 {
        self.clock.get(&self.replica_id)
    }
}

/// Causal-envelope validation error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CausalEnvelopeError {
    /// The wire schema is not supported by this client.
    UnsupportedSchema(String),
    /// A logical document key is empty, overlong, or contains control bytes.
    InvalidDocumentId,
    /// An idempotency key is empty, overlong, or contains control bytes.
    InvalidMutationId,
    /// The vector does not contain a positive counter for its declared actor.
    MissingActorCounter(String),
    /// Vector-clock validation or arithmetic failed.
    VersionVector(VersionVectorError),
}

impl Display for CausalEnvelopeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "unsupported causal envelope schema {schema:?}")
            }
            Self::InvalidDocumentId => write!(
                formatter,
                "document id must be 1..={MAX_DOCUMENT_ID_BYTES} bytes without control characters"
            ),
            Self::InvalidMutationId => write!(
                formatter,
                "mutation id must be 1..={MAX_MUTATION_ID_BYTES} bytes without control characters"
            ),
            Self::MissingActorCounter(replica_id) => write!(
                formatter,
                "causal envelope actor {replica_id:?} has no positive counter"
            ),
            Self::VersionVector(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for CausalEnvelopeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::VersionVector(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VersionVectorError> for CausalEnvelopeError {
    fn from(error: VersionVectorError) -> Self {
        Self::VersionVector(error)
    }
}

fn validate_replica_id(replica_id: &str) -> Result<(), VersionVectorError> {
    let valid = !replica_id.is_empty()
        && replica_id.len() <= MAX_REPLICA_ID_BYTES
        && replica_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(VersionVectorError::InvalidReplicaId(replica_id.to_owned()))
    }
}

fn validate_document_id(document_id: &str) -> Result<(), CausalEnvelopeError> {
    if valid_external_id(document_id, MAX_DOCUMENT_ID_BYTES) {
        Ok(())
    } else {
        Err(CausalEnvelopeError::InvalidDocumentId)
    }
}

fn validate_mutation_id(mutation_id: &str) -> Result<(), CausalEnvelopeError> {
    if valid_external_id(mutation_id, MAX_MUTATION_ID_BYTES) {
        Ok(())
    } else {
        Err(CausalEnvelopeError::InvalidMutationId)
    }
}

fn valid_external_id(value: &str, maximum_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= maximum_bytes
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vector(entries: &[(&str, u64)]) -> VersionVector {
        VersionVector::from_entries(
            entries
                .iter()
                .map(|(replica, counter)| ((*replica).to_owned(), *counter)),
        )
        .unwrap_or_else(|error| panic!("test vector failed: {error}"))
    }

    #[test]
    fn vector_relations_cover_equal_before_after_and_concurrent() {
        let base = vector(&[("phone", 2), ("desktop", 1)]);
        assert_eq!(base.relation(&base), VersionRelation::Equal);
        assert_eq!(
            base.relation(&vector(&[("phone", 3), ("desktop", 1)])),
            VersionRelation::Before
        );
        assert_eq!(
            base.relation(&vector(&[("phone", 1), ("desktop", 1)])),
            VersionRelation::After
        );
        assert_eq!(
            base.relation(&vector(&[("phone", 1), ("desktop", 2)])),
            VersionRelation::Concurrent
        );
    }

    #[test]
    fn merge_and_observe_are_monotonic_and_idempotent() {
        let mut local = vector(&[("phone", 2)]);
        let remote = vector(&[("phone", 1), ("desktop", 4)]);
        assert_eq!(local.merge(&remote), Ok(true));
        assert_eq!(local.get("phone"), 2);
        assert_eq!(local.get("desktop"), 4);
        assert_eq!(local.merge(&remote), Ok(false));
        assert!(local.dominates(&remote));
    }

    #[test]
    fn envelope_creation_advances_clock_and_tombstones_round_trip() {
        let mut clock = VersionVector::new();
        let upsert = CausalEnvelope::upsert(
            "notes/42",
            "mutation-1",
            "phone",
            &mut clock,
            json!({"title": "offline"}),
        )
        .unwrap_or_else(|error| panic!("upsert failed: {error}"));
        assert_eq!(upsert.actor_counter(), 1);
        assert_eq!(clock.get("phone"), 1);
        assert_eq!(upsert.validate(), Ok(()));

        let delete = CausalEnvelope::<serde_json::Value>::delete(
            "notes/42",
            "mutation-2",
            "phone",
            &mut clock,
        )
        .unwrap_or_else(|error| panic!("delete failed: {error}"));
        assert!(delete.is_delete());
        assert_eq!(delete.actor_counter(), 2);

        let encoded = serde_json::to_string(&delete)
            .unwrap_or_else(|error| panic!("serialization failed: {error}"));
        let decoded: CausalEnvelope<serde_json::Value> = serde_json::from_str(&encoded)
            .unwrap_or_else(|error| panic!("deserialization failed: {error}"));
        assert_eq!(decoded, delete);
        assert_eq!(decoded.validate(), Ok(()));
    }

    #[test]
    fn disposition_distinguishes_replay_stale_advance_and_conflict() {
        let checkpoint = vector(&[("phone", 2), ("desktop", 1)]);
        let mut equal = CausalEnvelope {
            schema_version: CAUSAL_SCHEMA_VERSION.to_owned(),
            document_id: "doc".to_owned(),
            mutation_id: "same".to_owned(),
            replica_id: "phone".to_owned(),
            clock: checkpoint.clone(),
            operation: CausalOperation::Upsert(json!({})),
        };
        assert_eq!(
            equal.disposition_against(&checkpoint),
            CausalDisposition::Duplicate
        );

        equal.clock = vector(&[("phone", 1), ("desktop", 1)]);
        assert_eq!(
            equal.disposition_against(&checkpoint),
            CausalDisposition::Stale
        );

        equal.clock = vector(&[("phone", 3), ("desktop", 1)]);
        assert_eq!(
            equal.disposition_against(&checkpoint),
            CausalDisposition::Apply
        );

        equal.clock = vector(&[("phone", 1), ("desktop", 2)]);
        assert_eq!(
            equal.disposition_against(&checkpoint),
            CausalDisposition::ResolveConcurrent
        );
    }

    #[test]
    fn deserialize_rejects_zero_counters_and_invalid_replica_ids() {
        let zero = serde_json::from_value::<VersionVector>(json!({"phone": 0}));
        assert!(zero.is_err());
        let invalid = serde_json::from_value::<VersionVector>(json!({"phone space": 1}));
        assert!(invalid.is_err());
    }

    #[test]
    fn acknowledgement_merges_only_after_acceptance() {
        let mut producer_clock = vector(&[("phone", 4)]);
        let envelope = CausalEnvelope::upsert(
            "doc",
            "mutation-5",
            "desktop",
            &mut producer_clock,
            json!({"value": 5}),
        )
        .unwrap_or_else(|error| panic!("envelope failed: {error}"));
        let mut checkpoint = vector(&[("phone", 2)]);
        assert_eq!(
            envelope.disposition_against(&checkpoint),
            CausalDisposition::Apply
        );
        assert_eq!(envelope.acknowledge_into(&mut checkpoint), Ok(true));
        assert_eq!(checkpoint.get("phone"), 4);
        assert_eq!(checkpoint.get("desktop"), 1);
    }
}
