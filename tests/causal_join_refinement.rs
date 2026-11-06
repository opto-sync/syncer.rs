//! Finite, production-backed causal clock refinement over delivery orders.
//! Pure Rust integration tests; no Wasm guest/browser execution is claimed.
use syncer_rs::causal::{
    VersionRelation,
    VersionVector,
    VersionVectorError,
    MAX_CAUSAL_REPLICAS,
};

const REPLICAS: [&str; 3] = ["alpha", "beta", "gamma"];

fn vector(counters: [u64; 3]) -> VersionVector {
    VersionVector::from_entries(
        REPLICAS
            .iter()
            .zip(counters)
            .filter_map(|(replica, counter)| {
                (counter != 0).then_some(((*replica).to_owned(), counter))
            }),
    )
    .unwrap()
}

fn all_vectors() -> Vec<VersionVector> {
    (0_u64..3)
        .flat_map(|a| {
            (0_u64..3).flat_map(move |b| {
                (0_u64..3).map(move |c| vector([a, b, c]))
            })
        })
        .collect()
}

fn max_spec(left: &VersionVector, right: &VersionVector) -> VersionVector {
    vector(std::array::from_fn(|i| {
        left.get(REPLICAS[i]).max(right.get(REPLICAS[i]))
    }))
}

#[test]
fn all_bounded_pairs_refine_independent_pointwise_maximum_specification() {
    let clocks = all_vectors();
    let mut pairs = 0;
    for left in &clocks {
        for right in &clocks {
            let left_before = left.clone();
            let right_before = right.clone();
            let (joined, changed) = left.joined(right).unwrap();
            assert_eq!(joined, max_spec(left, right));
            assert_eq!(changed, &joined != left);
            assert_eq!(*left, left_before);
            assert_eq!(*right, right_before);
            assert!(joined.dominates(left) && joined.dominates(right));
            let backwards = right.relation(left);
            let expected_inverse = match left.relation(right) {
                VersionRelation::Equal => VersionRelation::Equal,
                VersionRelation::Before => VersionRelation::After,
                VersionRelation::After => VersionRelation::Before,
                VersionRelation::Concurrent => VersionRelation::Concurrent,
            };
            assert_eq!(backwards, expected_inverse);
            pairs += 1;
        }
    }
    assert_eq!(pairs, 729);
}

#[test]
fn bounded_join_is_associative_commutative_and_idempotent() {
    let clocks = all_vectors();
    let mut triples = 0;
    for a in &clocks {
        assert_eq!(a.joined(a).unwrap().0, *a);
        for b in &clocks {
            assert_eq!(a.joined(b).unwrap().0, b.joined(a).unwrap().0);
            for c in &clocks {
                let ab_c = a.joined(b).unwrap().0.joined(c).unwrap().0;
                let a_bc = a.joined(&b.joined(c).unwrap().0).unwrap().0;
                assert_eq!(ab_c, a_bc);
                assert!(ab_c.dominates(a));
                assert!(ab_c.dominates(b));
                assert!(ab_c.dominates(c));
                triples += 1;
            }
        }
    }
    assert_eq!(triples, 19_683);
}

#[test]
fn duplicated_out_of_order_observations_never_rollback_or_mutate_prior_values() {
    for replica in REPLICAS {
        let initial = vector([1, 2, 0]);
        let mut current = initial.clone();
        for counter in [8, 3, 1, 6, 8, 2, 9, 9, 4] {
            let prior = current.get(replica);
            let (advanced, changed) = current.observed(replica, counter).unwrap();
            assert_eq!(advanced.get(replica), prior.max(counter));
            assert_eq!(changed, counter > prior);
            assert!(advanced.dominates(&current));
            current = advanced;
        }
        assert_eq!(initial, vector([1, 2, 0]));
    }
}

#[test]
fn overflow_and_capacity_errors_do_not_partially_modify_owned_vectors() {
    let mut full = VersionVector::from_entries(
        (0..MAX_CAUSAL_REPLICAS).map(|i| (format!("replica_{i}"), 1)),
    )
    .unwrap();
    let before = full.clone();
    assert!(matches!(
        full.observe("outside", 1),
        Err(VersionVectorError::TooManyReplicas { .. })
    ));
    assert_eq!(full, before);
    let extra = vector([1, 0, 0]);
    assert!(matches!(
        full.joined(&extra),
        Err(VersionVectorError::TooManyReplicas { .. })
    ));
    assert_eq!(full, before);

    let mut overflow = VersionVector::from_entries([("alpha".to_owned(), u64::MAX)])
        .unwrap();
    let unchanged = overflow.clone();
    assert!(matches!(
        overflow.increment("alpha"),
        Err(VersionVectorError::CounterOverflow(_))
    ));
    assert_eq!(overflow, unchanged);
}

#[test]
fn negative_control_rejects_minimum_as_a_false_merge_algorithm() {
    let a = vector([2, 0, 1]);
    let b = vector([0, 2, 1]);
    let actual = a.joined(&b).unwrap().0;
    let broken = vector([a.get("alpha").min(b.get("alpha")), a.get("beta").min(b.get("beta")), 1]);
    assert_ne!(actual, broken, "negative control did not detect incorrect min-merge");
    assert!(!broken.dominates(&a));
    assert!(!broken.dominates(&b));
}
