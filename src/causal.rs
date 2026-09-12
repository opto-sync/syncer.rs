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
/// Stable identifier of the canonical causal-envelope JSON Schema.
pub const CAUSAL_ENVELOPE_SCHEMA_ID: &str =
    "https://opto-sync.dev/schema/causal-envelope.schema.json";
/// Draft 2020-12 wire-shape contract shipped with the crate.
pub const CAUSAL_ENVELOPE_JSON_SCHEMA: &str = include_str!("../schema/causal-envelope.schema.json");
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
        // HOT-PATH (imperative by design): this runs for every vector deserialized
        // off the wire; observing into the owned accumulator is O(n log n) whereas
        // rebuilding a new vector per entry (`observed`) would be O(n^2) at the
        // 1_024-replica bound. The mutation is confined to the accumulator this
        // fold owns, and callers receive the finished immutable vector.
        entries.into_iter().try_fold(
            Self::new(),
            |mut vector, (replica_id, counter)| -> Result<Self, VersionVectorError> {
                vector.observe(&replica_id, counter)?;
                Ok(vector)
            },
        )
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
        let next = self.next_counter(replica_id)?;
        self.entries.insert(replica_id.to_owned(), next);
        Ok(next)
    }

    /// Returns a new vector with `replica_id` advanced, plus the assigned counter.
    ///
    /// `self` is never modified; the result owns fully independent storage.
    ///
    /// # Errors
    ///
    /// Same conditions as [`VersionVector::increment`].
    pub fn incremented(&self, replica_id: &str) -> Result<(Self, u64), VersionVectorError> {
        let next = self.next_counter(replica_id)?;
        Ok((self.with_counter(replica_id, next), next))
    }

    /// Observes an explicit positive counter, retaining the larger value.
    ///
    /// Replaying an older observation is therefore idempotent.
    pub fn observe(&mut self, replica_id: &str, counter: u64) -> Result<bool, VersionVectorError> {
        let Some(joined) = self.joined_counter(replica_id, counter)? else {
            return Ok(false);
        };
        self.entries.insert(replica_id.to_owned(), joined);
        Ok(true)
    }

    /// Returns a new vector that has observed `counter`, plus whether it differs
    /// from `self`. `self` is never modified.
    ///
    /// # Errors
    ///
    /// Same conditions as [`VersionVector::observe`].
    pub fn observed(
        &self,
        replica_id: &str,
        counter: u64,
    ) -> Result<(Self, bool), VersionVectorError> {
        Ok(match self.joined_counter(replica_id, counter)? {
            Some(joined) => (self.with_counter(replica_id, joined), true),
            None => (self.clone(), false),
        })
    }

    /// Merges another vector by taking the maximum counter for every replica.
    ///
    /// Returns whether the local vector changed.
    pub fn merge(&mut self, other: &Self) -> Result<bool, VersionVectorError> {
        other.iter().try_fold(
            false,
            |changed, (replica_id, counter)| -> Result<bool, VersionVectorError> {
                Ok(changed | self.observe(replica_id, counter)?)
            },
        )
    }

    /// Returns the least upper bound of `self` and `other` as a new vector, plus
    /// whether it differs from `self`. Neither input is modified.
    ///
    /// # Errors
    ///
    /// Returns [`VersionVectorError::TooManyReplicas`] when the union of replicas
    /// exceeds [`MAX_CAUSAL_REPLICAS`]; unlike [`VersionVector::merge`] nothing
    /// is partially applied in that case.
    pub fn joined(&self, other: &Self) -> Result<(Self, bool), VersionVectorError> {
        let entries = self
            .entries
            .keys()
            .chain(other.entries.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|replica_id| {
                (
                    replica_id.clone(),
                    join_counter(self.get(replica_id), other.get(replica_id)),
                )
            })
            .collect::<BTreeMap<String, u64>>();
        if entries.len() > MAX_CAUSAL_REPLICAS {
            return Err(VersionVectorError::TooManyReplicas {
                maximum: MAX_CAUSAL_REPLICAS,
            });
        }
        let changed = entries != self.entries;
        Ok((Self { entries }, changed))
    }

    /// Compares two vector clocks using the standard partial order.
    #[must_use]
    pub fn relation(&self, other: &Self) -> VersionRelation {
        let replicas = self
            .entries
            .keys()
            .chain(other.entries.keys())
            .collect::<BTreeSet<_>>();
        // `Err(())` short-circuits as soon as both directions have been seen.
        let flags = replicas
            .into_iter()
            .try_fold((false, false), |(less, greater), replica_id| {
                let local = self.get(replica_id);
                let remote = other.get(replica_id);
                let less = less || local < remote;
                let greater = greater || local > remote;
                if less && greater {
                    Err(())
                } else {
                    Ok((less, greater))
                }
            });
        match flags {
            Err(()) => VersionRelation::Concurrent,
            Ok((less, greater)) => relation_from_order_flags(less, greater),
        }
    }

    /// The counter `replica_id` would receive from an increment, after the
    /// identifier and capacity checks shared by the in-place and value forms.
    fn next_counter(&self, replica_id: &str) -> Result<u64, VersionVectorError> {
        validate_replica_id(replica_id)?;
        self.admit_replica(replica_id)?;
        self.get(replica_id)
            .checked_add(1)
            .ok_or_else(|| VersionVectorError::CounterOverflow(replica_id.to_owned()))
    }

    /// The counter `replica_id` would hold after observing `counter`, or `None`
    /// when the observation changes nothing.
    fn joined_counter(
        &self,
        replica_id: &str,
        counter: u64,
    ) -> Result<Option<u64>, VersionVectorError> {
        validate_replica_id(replica_id)?;
        if counter == 0 {
            return Err(VersionVectorError::ZeroCounter(replica_id.to_owned()));
        }
        self.admit_replica(replica_id)?;
        let current = self.get(replica_id);
        let joined = join_counter(current, counter);
        Ok((joined != current).then_some(joined))
    }

    /// Rejects a replica that would push the vector past its bound.
    fn admit_replica(&self, replica_id: &str) -> Result<(), VersionVectorError> {
        if !self.entries.contains_key(replica_id) && self.entries.len() >= MAX_CAUSAL_REPLICAS {
            return Err(VersionVectorError::TooManyReplicas {
                maximum: MAX_CAUSAL_REPLICAS,
            });
        }
        Ok(())
    }

    /// A new vector equal to `self` with one counter set, in independent storage.
    fn with_counter(&self, replica_id: &str, counter: u64) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .map(|(existing_id, existing)| (existing_id.clone(), *existing))
                .chain([(replica_id.to_owned(), counter)])
                .collect(),
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
    ///
    /// This is the in-place compatibility form: the envelope is built from an
    /// immutable view of `clock` by [`CausalEnvelope::new`], and the single
    /// assignment below is the only mutation. Hosts that keep their clock as a
    /// value should use [`crate::functional::causal_upsert`] instead.
    pub fn upsert(
        document_id: impl Into<String>,
        mutation_id: impl Into<String>,
        replica_id: impl Into<String>,
        clock: &mut VersionVector,
        payload: T,
    ) -> Result<Self, CausalEnvelopeError> {
        let envelope = Self::new(
            document_id,
            mutation_id,
            replica_id,
            clock,
            CausalOperation::Upsert(payload),
        )?;
        *clock = envelope.clock.clone();
        Ok(envelope)
    }

    /// Creates a delete tombstone and advances `clock` for `replica_id`.
    ///
    /// See [`CausalEnvelope::upsert`] for the mutation boundary; the value form
    /// is [`crate::functional::causal_delete`].
    pub fn delete(
        document_id: impl Into<String>,
        mutation_id: impl Into<String>,
        replica_id: impl Into<String>,
        clock: &mut VersionVector,
    ) -> Result<Self, CausalEnvelopeError> {
        let envelope = Self::new(
            document_id,
            mutation_id,
            replica_id,
            clock,
            CausalOperation::Delete,
        )?;
        *clock = envelope.clock.clone();
        Ok(envelope)
    }

    /// Builds an envelope whose `clock` is `clock` advanced for `replica_id`.
    ///
    /// `clock` is only read; the envelope owns the advanced vector. The next
    /// local clock a host must persist is exactly `envelope.clock`.
    pub(crate) fn new(
        document_id: impl Into<String>,
        mutation_id: impl Into<String>,
        replica_id: impl Into<String>,
        clock: &VersionVector,
        operation: CausalOperation<T>,
    ) -> Result<Self, CausalEnvelopeError> {
        let document_id = document_id.into();
        let mutation_id = mutation_id.into();
        let replica_id = replica_id.into();
        validate_document_id(&document_id)?;
        validate_mutation_id(&mutation_id)?;
        validate_replica_id(&replica_id)?;
        let (clock, _counter) = clock.incremented(&replica_id)?;

        Ok(Self {
            schema_version: CAUSAL_SCHEMA_VERSION.to_owned(),
            document_id,
            mutation_id,
            replica_id,
            clock,
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
        disposition_from_relation(self.clock.relation(checkpoint))
    }

    /// Merges this envelope's clock into a receiver after the mutation is
    /// accepted or its concurrent conflict has been resolved.
    ///
    /// In-place compatibility form of [`CausalEnvelope::acknowledged`]: the next
    /// checkpoint is computed as a value and assigned once, so a rejected join
    /// leaves `checkpoint` untouched.
    pub fn acknowledge_into(
        &self,
        checkpoint: &mut VersionVector,
    ) -> Result<bool, VersionVectorError> {
        let (next, changed) = self.acknowledged(checkpoint)?;
        *checkpoint = next;
        Ok(changed)
    }

    /// Returns the receiver checkpoint joined with this envelope's clock as a
    /// new vector, plus whether it differs from `checkpoint`.
    ///
    /// # Errors
    ///
    /// Returns [`VersionVectorError::TooManyReplicas`] when the joined vector
    /// would exceed [`MAX_CAUSAL_REPLICAS`].
    pub fn acknowledged(
        &self,
        checkpoint: &VersionVector,
    ) -> Result<(VersionVector, bool), VersionVectorError> {
        checkpoint.joined(&self.clock)
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
        && replica_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'));
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
    !value.trim().is_empty() && value.len() <= maximum_bytes && !value.chars().any(char::is_control)
}

const fn join_counter(local: u64, incoming: u64) -> u64 {
    if local >= incoming { local } else { incoming }
}

const fn relation_from_order_flags(less: bool, greater: bool) -> VersionRelation {
    match (less, greater) {
        (false, false) => VersionRelation::Equal,
        (true, false) => VersionRelation::Before,
        (false, true) => VersionRelation::After,
        (true, true) => VersionRelation::Concurrent,
    }
}

const fn disposition_from_relation(relation: VersionRelation) -> CausalDisposition {
    match relation {
        VersionRelation::Equal => CausalDisposition::Duplicate,
        VersionRelation::Before => CausalDisposition::Stale,
        VersionRelation::After => CausalDisposition::Apply,
        VersionRelation::Concurrent => CausalDisposition::ResolveConcurrent,
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    fn reverse(relation: VersionRelation) -> VersionRelation {
        match relation {
            VersionRelation::Equal => VersionRelation::Equal,
            VersionRelation::Before => VersionRelation::After,
            VersionRelation::After => VersionRelation::Before,
            VersionRelation::Concurrent => VersionRelation::Concurrent,
        }
    }

    #[kani::proof]
    fn two_replica_relation_is_dual_for_all_counters() {
        let left_alpha = kani::any::<u64>();
        let left_beta = kani::any::<u64>();
        let right_alpha = kani::any::<u64>();
        let right_beta = kani::any::<u64>();

        let forward = relation_from_order_flags(
            left_alpha < right_alpha || left_beta < right_beta,
            left_alpha > right_alpha || left_beta > right_beta,
        );
        let backward = relation_from_order_flags(
            right_alpha < left_alpha || right_beta < left_beta,
            right_alpha > left_alpha || right_beta > left_beta,
        );
        assert_eq!(forward, reverse(backward));
    }

    #[kani::proof]
    fn counter_join_is_a_commutative_idempotent_upper_bound() {
        let left = kani::any::<u64>();
        let right = kani::any::<u64>();
        let joined = join_counter(left, right);

        assert_eq!(joined, join_counter(right, left));
        assert_eq!(join_counter(left, left), left);
        assert!(joined >= left);
        assert!(joined >= right);
    }

    #[kani::proof]
    fn disposition_mapping_is_total_and_distinct() {
        assert_eq!(
            disposition_from_relation(VersionRelation::Equal),
            CausalDisposition::Duplicate
        );
        assert_eq!(
            disposition_from_relation(VersionRelation::Before),
            CausalDisposition::Stale
        );
        assert_eq!(
            disposition_from_relation(VersionRelation::After),
            CausalDisposition::Apply
        );
        assert_eq!(
            disposition_from_relation(VersionRelation::Concurrent),
            CausalDisposition::ResolveConcurrent
        );
    }
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

    #[test]
    fn embedded_causal_schema_matches_the_wire_discriminants_and_bounds() {
        let schema: serde_json::Value = serde_json::from_str(CAUSAL_ENVELOPE_JSON_SCHEMA)
            .expect("embedded causal schema must be valid JSON");
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(schema["$id"], CAUSAL_ENVELOPE_SCHEMA_ID);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["schemaVersion"]["const"],
            CAUSAL_SCHEMA_VERSION
        );
        assert_eq!(
            schema["properties"]["clock"]["maxProperties"],
            MAX_CAUSAL_REPLICAS
        );
        assert_eq!(
            schema["properties"]["clock"]["additionalProperties"]["maximum"],
            u64::MAX
        );

        let operation_kinds = schema["properties"]["operation"]["oneOf"]
            .as_array()
            .expect("operation must be a union")
            .iter()
            .map(|variant| {
                variant["properties"]["kind"]["const"]
                    .as_str()
                    .expect("operation kind must be a string")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(operation_kinds, BTreeSet::from(["delete", "upsert"]));
    }
}
