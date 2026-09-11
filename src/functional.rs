//! Value-oriented causal helpers for hosts that prefer immutable state flow.
//!
//! The low-level [`crate::VersionVector`] mutation methods remain available for
//! allocation-sensitive loops. This module deliberately rebuilds vectors into
//! independent storage so callers can express `next = transform(current)`
//! without a helper mutating caller-owned state.

use crate::{
    CausalEnvelope, CausalEnvelopeError, VersionVector, VersionVectorError,
};

/// Result of advancing or joining a vector without mutating the input vector.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VectorTransition {
    /// Fully independent next vector.
    pub next: VersionVector,
    /// Whether the transformation changed the logical vector.
    pub changed: bool,
}

/// Result of constructing a causal mutation from an immutable local clock.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CausalWrite<T> {
    /// Wire envelope containing its own immutable causal snapshot.
    pub envelope: CausalEnvelope<T>,
    /// Independent next local clock to persist for subsequent writes.
    pub next_clock: VersionVector,
}

/// Returns a completely independent copy of `clock`.
///
/// This intentionally reconstructs the map from owned replica-id strings
/// instead of sharing interior storage. It is the semantic boundary used by
/// the functional helpers in this module.
pub fn independent_clock(clock: &VersionVector) -> Result<VersionVector, VersionVectorError> {
    VersionVector::from_entries(
        clock
            .iter()
            .map(|(replica_id, counter)| (replica_id.to_owned(), counter)),
    )
}

/// Advances one replica and returns a fresh vector plus the assigned counter.
pub fn incremented_clock(
    clock: &VersionVector,
    replica_id: &str,
) -> Result<(VersionVector, u64), VersionVectorError> {
    let mut next = independent_clock(clock)?;
    let counter = next.increment(replica_id)?;
    Ok((next, counter))
}

/// Observes a counter and returns the next independent vector.
pub fn observed_clock(
    clock: &VersionVector,
    replica_id: &str,
    counter: u64,
) -> Result<VectorTransition, VersionVectorError> {
    let mut next = independent_clock(clock)?;
    let changed = next.observe(replica_id, counter)?;
    Ok(VectorTransition { next, changed })
}

/// Joins two clocks and returns a fresh vector instead of mutating either input.
pub fn merged_clock(
    clock: &VersionVector,
    other: &VersionVector,
) -> Result<VectorTransition, VersionVectorError> {
    let mut next = independent_clock(clock)?;
    let changed = next.merge(other)?;
    Ok(VectorTransition { next, changed })
}

/// Constructs an upsert envelope and next local clock without mutating `clock`.
pub fn causal_upsert<T>(
    document_id: impl Into<String>,
    mutation_id: impl Into<String>,
    replica_id: impl Into<String>,
    clock: &VersionVector,
    payload: T,
) -> Result<CausalWrite<T>, CausalEnvelopeError> {
    let mut next_clock = independent_clock(clock)?;
    let envelope = CausalEnvelope::upsert(
        document_id,
        mutation_id,
        replica_id,
        &mut next_clock,
        payload,
    )?;
    Ok(CausalWrite {
        envelope,
        next_clock,
    })
}

/// Constructs a delete envelope and next local clock without mutating `clock`.
pub fn causal_delete<T>(
    document_id: impl Into<String>,
    mutation_id: impl Into<String>,
    replica_id: impl Into<String>,
    clock: &VersionVector,
) -> Result<CausalWrite<T>, CausalEnvelopeError> {
    let mut next_clock = independent_clock(clock)?;
    let envelope = CausalEnvelope::delete(
        document_id,
        mutation_id,
        replica_id,
        &mut next_clock,
    )?;
    Ok(CausalWrite {
        envelope,
        next_clock,
    })
}

/// Returns a fresh acknowledged checkpoint instead of mutating `checkpoint`.
pub fn acknowledged_checkpoint<T>(
    envelope: &CausalEnvelope<T>,
    checkpoint: &VersionVector,
) -> Result<VectorTransition, VersionVectorError> {
    let mut next = independent_clock(checkpoint)?;
    let changed = envelope.acknowledge_into(&mut next)?;
    Ok(VectorTransition { next, changed })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn vector(entries: &[(&str, u64)]) -> VersionVector {
        VersionVector::from_entries(
            entries
                .iter()
                .map(|(replica, counter)| ((*replica).to_owned(), *counter)),
        )
        .expect("valid test vector")
    }

    #[test]
    fn increment_returns_fresh_clock_and_preserves_input() {
        let original = vector(&[("desktop", 2)]);
        let (next, counter) = incremented_clock(&original, "desktop").expect("increment");

        assert_eq!(counter, 3);
        assert_eq!(original.get("desktop"), 2);
        assert_eq!(next.get("desktop"), 3);
    }

    #[test]
    fn merge_returns_fresh_clock_and_preserves_both_inputs() {
        let local = vector(&[("desktop", 2)]);
        let remote = vector(&[("phone", 4)]);
        let transition = merged_clock(&local, &remote).expect("merge");

        assert!(transition.changed);
        assert_eq!(local.get("phone"), 0);
        assert_eq!(remote.get("desktop"), 0);
        assert_eq!(transition.next.get("desktop"), 2);
        assert_eq!(transition.next.get("phone"), 4);
    }

    #[test]
    fn causal_write_returns_new_clock_without_touching_source() {
        let original = vector(&[("desktop", 2)]);
        let write = causal_upsert(
            "notes/42",
            "mutation-1",
            "desktop",
            &original,
            json!({"title": "offline"}),
        )
        .expect("causal write");

        assert_eq!(original.get("desktop"), 2);
        assert_eq!(write.next_clock.get("desktop"), 3);
        assert_eq!(write.envelope.actor_counter(), 3);
        assert_eq!(write.envelope.clock, write.next_clock);
    }

    #[test]
    fn acknowledging_returns_new_checkpoint_without_touching_source() {
        let checkpoint = vector(&[("desktop", 1)]);
        let source = vector(&[("desktop", 1)]);
        let write = causal_delete::<serde_json::Value>(
            "notes/42",
            "mutation-2",
            "desktop",
            &source,
        )
        .expect("delete");

        let transition = acknowledged_checkpoint(&write.envelope, &checkpoint)
            .expect("acknowledge");
        assert!(transition.changed);
        assert_eq!(checkpoint.get("desktop"), 1);
        assert_eq!(transition.next.get("desktop"), 2);
    }
}
