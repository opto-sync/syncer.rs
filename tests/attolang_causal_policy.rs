//! Bounded independent Rust oracle for the experimental AttoLang causal policy.
//! This is not compiled or executed AttoLang guest evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Relation {
    Before,
    Equal,
    After,
    Concurrent,
}

fn before(a: (u8, u8), b: (u8, u8)) -> bool {
    a.0 <= b.0 && a.1 <= b.1 && a != b
}

fn relation(a: (u8, u8), b: (u8, u8)) -> Relation {
    if a == b {
        Relation::Equal
    } else if before(a, b) {
        Relation::Before
    } else if before(b, a) {
        Relation::After
    } else {
        Relation::Concurrent
    }
}

fn join(a: (u8, u8), b: (u8, u8)) -> (u8, u8) {
    (a.0.max(b.0), a.1.max(b.1))
}

fn inverse(relation: Relation) -> Relation {
    match relation {
        Relation::Before => Relation::After,
        Relation::After => Relation::Before,
        other => other,
    }
}

#[test]
fn finite_two_replica_causal_policy_is_exhaustive_and_nonvacuous() {
    let clocks: Vec<_> = (0..4).flat_map(|a| (0..4).map(move |b| (a, b))).collect();
    let mut counts = [0_usize; 4];
    for &a in &clocks {
        for &b in &clocks {
            let observed = relation(a, b);
            let index = match observed {
                Relation::Equal => 0,
                Relation::Before => 1,
                Relation::After => 2,
                Relation::Concurrent => 3,
            };
            counts[index] += 1;
            assert_eq!(relation(b, a), inverse(observed));
            assert_eq!(before(a, b), observed == Relation::Before);
            assert_eq!(
                usize::from(a == b)
                    + usize::from(before(a, b))
                    + usize::from(before(b, a))
                    + usize::from(observed == Relation::Concurrent),
                1
            );
            let combined = join(a, b);
            assert!(combined.0 >= a.0 && combined.0 >= b.0);
            assert!(combined.1 >= a.1 && combined.1 >= b.1);
            assert_eq!(join(a, a), a);
            assert_eq!(combined, join(b, a));
        }
    }
    assert_eq!(counts, [16, 84, 84, 72]);
    assert_eq!(relation((0, 2), (2, 0)), Relation::Concurrent);
    assert_eq!(relation((0, 1), (1, 1)), Relation::Before);
    assert_eq!(relation((1, 1), (1, 1)), Relation::Equal);
}

#[test]
fn atto_source_carries_expected_policy_predicates_not_runtime_claims() {
    let candidate = include_str!("../src/causal_policy.atto");
    for anchor in [
        "define module causal_policy",
        "clock_before(",
        "clock_equal(",
        "clock_concurrent(",
        "self_check()",
        "a0 <= b0 && a1 <= b1",
        "!clock_before(b0,b1,a0,a1)",
    ] {
        assert!(candidate.contains(anchor), "missing .atto source anchor: {anchor}");
    }
}

#[test]
fn broken_controls_are_detectable() {
    let pair_a = (0, 2);
    let pair_b = (2, 0);
    assert_ne!(relation(pair_a, pair_b), Relation::Before);
    assert_ne!(relation(pair_a, pair_b), Relation::Equal);
    assert_ne!(join(pair_a, pair_b), pair_a);
}
