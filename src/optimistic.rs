//! Optimistic local writes for Flutter and Rust desktop hosts.
//!
//! This module is pure: it never opens SQLite, files, sockets, or a logger.
//! A desktop or Flutter host persists [`OptimisticWrite::into_persistable`]
//! — the envelope and the version-vector snapshot — in one local transaction.
//! The pair is the only durable unit this crate produces for an optimistic
//! mutation; splitting the writes is a host bug, not a recoverable state.
//!
//! Error [`Display`] values name identifiers and error kinds only. Payload
//! bodies, credentials, and document values are never formatted.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::causal::{
    CausalDisposition, CausalEnvelope, CausalEnvelopeError, CausalOperation, VersionRelation,
    VersionVector, VersionVectorError,
};

/// Durable unit of one optimistic local mutation.
///
/// `envelope.clock` and `snapshot` are the same vector after the originating
/// replica increments. Hosts must persist both in the same local transaction
/// as the application row they render from `localView`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OptimisticWrite<T> {
    envelope: CausalEnvelope<T>,
    snapshot: VersionVector,
}

impl<T> OptimisticWrite<T> {
    /// Envelope that travels on the wire and through the mutation queue.
    #[must_use]
    pub const fn envelope(&self) -> &CausalEnvelope<T> {
        &self.envelope
    }

    /// Local version-vector snapshot taken after the replica increment.
    #[must_use]
    pub const fn snapshot(&self) -> &VersionVector {
        &self.snapshot
    }

    /// Splits the write into the two values the host stores together.
    #[must_use]
    pub fn into_persistable(self) -> (CausalEnvelope<T>, VersionVector) {
        (self.envelope, self.snapshot)
    }

    /// Rebuilds a write from storage after both halves were loaded.
    ///
    /// # Errors
    ///
    /// Returns [`OptimisticError::StaleVector`] when the stored snapshot is
    /// not exactly the envelope clock — the pair drifted, so they were not
    /// persisted atomically. Missing actor counters become
    /// [`OptimisticError::MissingReplica`].
    pub fn from_persisted(
        envelope: CausalEnvelope<T>,
        snapshot: VersionVector,
    ) -> Result<Self, OptimisticError> {
        envelope.validate().map_err(map_envelope_error)?;
        if snapshot.relation(&envelope.clock) != VersionRelation::Equal {
            return Err(OptimisticError::StaleVector);
        }
        Ok(Self { envelope, snapshot })
    }
}

/// Typed failures for optimistic record and receive paths.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum OptimisticError {
    /// Incoming clock is concurrent; do not acknowledge until the product
    /// persists a resolved value.
    Conflict {
        /// Logical document key of the conflicting mutation.
        document_id: String,
        /// Client-generated idempotency key of the conflicting mutation.
        mutation_id: String,
    },
    /// Replica identifier is invalid, or the envelope clock has no actor
    /// counter for the declared replica.
    MissingReplica {
        /// Replica that was missing or rejected.
        replica_id: String,
    },
    /// Incoming clock is strictly behind the durable checkpoint, or a stored
    /// snapshot no longer matches its envelope.
    StaleVector,
    /// Envelope identity or schema validation failed.
    Envelope(CausalEnvelopeError),
    /// Version-vector bounds or arithmetic failed.
    VersionVector(VersionVectorError),
}

impl Display for OptimisticError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict {
                document_id,
                mutation_id,
            } => write!(
                formatter,
                "optimistic conflict document={document_id} mutation={mutation_id}"
            ),
            Self::MissingReplica { replica_id } => {
                write!(formatter, "optimistic missing replica {replica_id}")
            }
            Self::StaleVector => write!(formatter, "optimistic stale version vector"),
            Self::Envelope(error) => Display::fmt(error, formatter),
            Self::VersionVector(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for OptimisticError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Envelope(error) => Some(error),
            Self::VersionVector(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CausalEnvelopeError> for OptimisticError {
    fn from(error: CausalEnvelopeError) -> Self {
        map_envelope_error(error)
    }
}

impl From<VersionVectorError> for OptimisticError {
    fn from(error: VersionVectorError) -> Self {
        map_vector_error(error)
    }
}

/// Receiver outcome after a write that may be acknowledged.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum OptimisticAck {
    /// The receiver already holds this exact frontier; acknowledgement is
    /// idempotent and does not re-apply the payload.
    Duplicate {
        /// Unchanged durable checkpoint.
        checkpoint: VersionVector,
    },
    /// The mutation advanced the checkpoint without a causal conflict.
    Applied {
        /// Checkpoint after merging the envelope clock.
        checkpoint: VersionVector,
    },
}

impl OptimisticAck {
    /// Durable checkpoint to persist after this outcome.
    #[must_use]
    pub const fn checkpoint(&self) -> &VersionVector {
        match self {
            Self::Duplicate { checkpoint } | Self::Applied { checkpoint } => checkpoint,
        }
    }
}

/// Records an optimistic upsert against an immutable local clock.
///
/// Increments `replica_id` on a clone of `local_clock`, snapshots the result
/// into the envelope, and returns the pair the host must persist together.
///
/// # Errors
///
/// Returns [`OptimisticError::MissingReplica`] for an invalid replica id and
/// [`OptimisticError::Envelope`] / [`OptimisticError::VersionVector`] for
/// other constructor failures.
pub fn record_upsert<T>(
    document_id: impl Into<String>,
    mutation_id: impl Into<String>,
    replica_id: impl Into<String>,
    local_clock: &VersionVector,
    payload: T,
) -> Result<OptimisticWrite<T>, OptimisticError> {
    record_with(
        document_id.into(),
        mutation_id.into(),
        replica_id.into(),
        local_clock,
        CausalOperation::Upsert(payload),
    )
}

/// Records an optimistic delete tombstone against an immutable local clock.
///
/// # Errors
///
/// Same classes as [`record_upsert`].
pub fn record_delete<T>(
    document_id: impl Into<String>,
    mutation_id: impl Into<String>,
    replica_id: impl Into<String>,
    local_clock: &VersionVector,
) -> Result<OptimisticWrite<T>, OptimisticError> {
    record_with(
        document_id.into(),
        mutation_id.into(),
        replica_id.into(),
        local_clock,
        CausalOperation::Delete,
    )
}

/// Validates an incoming envelope and acknowledges it only when safe.
///
/// Concurrent envelopes return [`OptimisticError::Conflict`] and never merge
/// into the checkpoint. Stale envelopes return [`OptimisticError::StaleVector`]
/// without applying the payload. Duplicates acknowledge idempotently.
///
/// # Errors
///
/// Conflict, stale, missing replica, or envelope/vector validation.
pub fn receive_and_ack<T>(
    envelope: &CausalEnvelope<T>,
    checkpoint: &VersionVector,
) -> Result<OptimisticAck, OptimisticError> {
    envelope.validate().map_err(map_envelope_error)?;
    match envelope.disposition_against(checkpoint) {
        CausalDisposition::Duplicate => Ok(OptimisticAck::Duplicate {
            checkpoint: join_checkpoint(envelope, checkpoint)?,
        }),
        CausalDisposition::Stale => Err(OptimisticError::StaleVector),
        CausalDisposition::Apply => Ok(OptimisticAck::Applied {
            checkpoint: join_checkpoint(envelope, checkpoint)?,
        }),
        CausalDisposition::ResolveConcurrent => Err(OptimisticError::Conflict {
            document_id: envelope.document_id.clone(),
            mutation_id: envelope.mutation_id.clone(),
        }),
    }
}

/// Acknowledges a concurrent envelope after the host persisted a resolution.
///
/// Call this only after the product conflict policy has written the resolved
/// document. Acknowledging earlier would suppress a retry while the document
/// remained unresolved.
///
/// # Errors
///
/// Returns [`OptimisticError::Conflict`] when the envelope is not concurrent
/// (the caller used the wrong path). Other validation failures map as usual.
pub fn acknowledge_resolved<T>(
    envelope: &CausalEnvelope<T>,
    checkpoint: &VersionVector,
) -> Result<VersionVector, OptimisticError> {
    envelope.validate().map_err(map_envelope_error)?;
    match envelope.disposition_against(checkpoint) {
        CausalDisposition::ResolveConcurrent => join_checkpoint(envelope, checkpoint),
        CausalDisposition::Duplicate | CausalDisposition::Stale | CausalDisposition::Apply => {
            Err(OptimisticError::Conflict {
                document_id: envelope.document_id.clone(),
                mutation_id: envelope.mutation_id.clone(),
            })
        }
    }
}

/// Reports whether a write's envelope clock and snapshot still match.
#[must_use]
pub fn same_transaction_pair<T>(write: &OptimisticWrite<T>) -> bool {
    write.snapshot.relation(&write.envelope.clock) == VersionRelation::Equal
}

fn record_with<T>(
    document_id: String,
    mutation_id: String,
    replica_id: String,
    local_clock: &VersionVector,
    operation: CausalOperation<T>,
) -> Result<OptimisticWrite<T>, OptimisticError> {
    let prior = local_clock.get(&replica_id);
    let mut clock = local_clock.clone();
    let envelope = match operation {
        CausalOperation::Upsert(payload) => {
            CausalEnvelope::upsert(document_id, mutation_id, replica_id, &mut clock, payload)
        }
        CausalOperation::Delete => {
            CausalEnvelope::delete(document_id, mutation_id, replica_id, &mut clock)
        }
    }
    .map_err(map_envelope_error)?;

    if envelope.clock.relation(&clock) != VersionRelation::Equal {
        return Err(OptimisticError::StaleVector);
    }
    let expected = prior.checked_add(1);
    if expected.is_some_and(|counter| clock.get(&envelope.replica_id) != counter) {
        return Err(OptimisticError::StaleVector);
    }

    Ok(OptimisticWrite {
        snapshot: clock,
        envelope,
    })
}

fn join_checkpoint<T>(
    envelope: &CausalEnvelope<T>,
    checkpoint: &VersionVector,
) -> Result<VersionVector, OptimisticError> {
    let mut next = checkpoint.clone();
    envelope
        .acknowledge_into(&mut next)
        .map_err(map_vector_error)?;
    Ok(next)
}

fn map_envelope_error(error: CausalEnvelopeError) -> OptimisticError {
    match error {
        CausalEnvelopeError::MissingActorCounter(replica_id) => {
            OptimisticError::MissingReplica { replica_id }
        }
        CausalEnvelopeError::VersionVector(error) => map_vector_error(error),
        other => OptimisticError::Envelope(other),
    }
}

fn map_vector_error(error: VersionVectorError) -> OptimisticError {
    match error {
        VersionVectorError::InvalidReplicaId(replica_id) => {
            OptimisticError::MissingReplica { replica_id }
        }
        other => OptimisticError::VersionVector(other),
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
    fn record_increments_replica_and_snapshots_the_same_vector() {
        let local = vector(&[("phone", 2), ("desktop", 4)]);
        let write = record_upsert(
            "notes/42",
            "mutation-1",
            "desktop",
            &local,
            json!({"title": "offline edit", "token": "s3cret-value"}),
        )
        .unwrap_or_else(|error| panic!("record failed: {error}"));

        assert_eq!(write.snapshot().get("desktop"), 5);
        assert_eq!(write.snapshot().get("phone"), 2);
        assert_eq!(
            write.envelope().clock.relation(write.snapshot()),
            VersionRelation::Equal
        );
        assert_eq!(write.envelope().actor_counter(), 5);
        assert!(same_transaction_pair(&write));
        assert_eq!(local.get("desktop"), 4, "caller clock stays immutable");
    }

    #[test]
    fn optimistic_write_plus_ack_converges_the_vector() {
        let local = vector(&[("phone", 2)]);
        let write = record_upsert(
            "notes/42",
            "mutation-3",
            "desktop",
            &local,
            json!({"text": "draft"}),
        )
        .unwrap_or_else(|error| panic!("record failed: {error}"));

        let checkpoint = vector(&[("phone", 2)]);
        let ack = receive_and_ack(write.envelope(), &checkpoint)
            .unwrap_or_else(|error| panic!("ack failed: {error}"));
        assert!(matches!(ack, OptimisticAck::Applied { .. }));
        assert_eq!(
            write.snapshot().relation(ack.checkpoint()),
            VersionRelation::Equal
        );
        assert_eq!(ack.checkpoint().get("phone"), 2);
        assert_eq!(ack.checkpoint().get("desktop"), 1);
    }

    #[test]
    fn persisted_pair_round_trips_only_when_snapshot_matches_envelope() {
        let write = record_upsert("doc", "m1", "phone", &VersionVector::new(), json!({"n": 1}))
            .unwrap_or_else(|error| panic!("record failed: {error}"));
        let (envelope, snapshot) = write.into_persistable();
        let restored = OptimisticWrite::from_persisted(envelope.clone(), snapshot.clone())
            .unwrap_or_else(|error| panic!("restore failed: {error}"));
        assert_eq!(restored.envelope(), &envelope);
        assert_eq!(restored.snapshot(), &snapshot);

        let drifted = vector(&[("phone", 9)]);
        assert_eq!(
            OptimisticWrite::from_persisted(envelope, drifted),
            Err(OptimisticError::StaleVector)
        );
    }

    #[test]
    fn concurrent_receive_is_conflict_and_does_not_ack() {
        let local = vector(&[("phone", 2)]);
        let write = record_upsert("notes/1", "m-desk", "desktop", &local, json!({"v": 1}))
            .unwrap_or_else(|error| panic!("record failed: {error}"));
        let checkpoint = vector(&[("phone", 3)]);
        assert_eq!(
            write.envelope().disposition_against(&checkpoint),
            CausalDisposition::ResolveConcurrent
        );
        assert_eq!(
            receive_and_ack(write.envelope(), &checkpoint),
            Err(OptimisticError::Conflict {
                document_id: "notes/1".to_owned(),
                mutation_id: "m-desk".to_owned(),
            })
        );
        assert_eq!(checkpoint.get("desktop"), 0);

        let resolved = acknowledge_resolved(write.envelope(), &checkpoint)
            .unwrap_or_else(|error| panic!("resolved ack failed: {error}"));
        assert_eq!(resolved.get("phone"), 3);
        assert_eq!(resolved.get("desktop"), 1);
    }

    #[test]
    fn stale_and_missing_replica_are_typed_errors() {
        let write = record_upsert("doc", "m-old", "phone", &VersionVector::new(), json!({}))
            .unwrap_or_else(|error| panic!("record failed: {error}"));
        let ahead = vector(&[("phone", 4)]);
        assert_eq!(
            receive_and_ack(write.envelope(), &ahead),
            Err(OptimisticError::StaleVector)
        );

        assert!(matches!(
            record_upsert("doc", "m2", "phone space", &VersionVector::new(), json!({})),
            Err(OptimisticError::MissingReplica { .. })
        ));

        let mut envelope = write.envelope().clone();
        envelope.replica_id = "ghost".to_owned();
        assert!(matches!(
            receive_and_ack(&envelope, &VersionVector::new()),
            Err(OptimisticError::MissingReplica { replica_id }) if replica_id == "ghost"
        ));
    }

    #[test]
    fn delete_tombstone_records_and_acks_like_an_upsert() {
        let write = record_delete::<serde_json::Value>(
            "notes/42",
            "mutation-2",
            "phone",
            &vector(&[("phone", 1)]),
        )
        .unwrap_or_else(|error| panic!("delete failed: {error}"));
        assert!(write.envelope().is_delete());
        assert_eq!(write.snapshot().get("phone"), 2);
        let ack = receive_and_ack(write.envelope(), &vector(&[("phone", 1)]))
            .unwrap_or_else(|error| panic!("ack failed: {error}"));
        assert_eq!(
            ack.checkpoint().relation(write.snapshot()),
            VersionRelation::Equal
        );
    }

    #[test]
    fn duplicate_ack_is_idempotent() {
        let write = record_upsert("doc", "same", "phone", &VersionVector::new(), json!({}))
            .unwrap_or_else(|error| panic!("record failed: {error}"));
        let first = receive_and_ack(write.envelope(), &VersionVector::new())
            .unwrap_or_else(|error| panic!("first ack failed: {error}"));
        let again = receive_and_ack(write.envelope(), first.checkpoint())
            .unwrap_or_else(|error| panic!("replay ack failed: {error}"));
        assert!(matches!(again, OptimisticAck::Duplicate { .. }));
        assert_eq!(
            again.checkpoint().relation(first.checkpoint()),
            VersionRelation::Equal
        );
    }

    #[test]
    fn display_never_includes_payload_secrets() {
        let secret = "s3cret-value";
        let write = record_upsert(
            "notes/42",
            "mutation-1",
            "desktop",
            &VersionVector::new(),
            json!({"password": secret, "token": "another-secret"}),
        )
        .unwrap_or_else(|error| panic!("record failed: {error}"));
        let checkpoint = vector(&[("phone", 1)]);
        let error = receive_and_ack(write.envelope(), &checkpoint)
            .expect_err("concurrent write must conflict");
        let rendered = error.to_string();
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("another-secret"));
        assert!(rendered.contains("notes/42"));
        assert!(rendered.contains("mutation-1"));
    }

    #[test]
    fn acknowledge_resolved_rejects_non_concurrent_envelopes() {
        let write = record_upsert("doc", "m1", "phone", &VersionVector::new(), json!({}))
            .unwrap_or_else(|error| panic!("record failed: {error}"));
        assert_eq!(
            acknowledge_resolved(write.envelope(), &VersionVector::new()),
            Err(OptimisticError::Conflict {
                document_id: "doc".to_owned(),
                mutation_id: "m1".to_owned(),
            })
        );
    }
}
