//! Desktop optimistic cycle used by Rust desktop apps and documented for Dart.
//!
//! `syncer.rs` records the persistable `(envelope, snapshot)` pair and
//! acknowledges it. SQLite / Drift persistence stays in the host.

use syncer_rs::{OptimisticAck, VersionRelation, VersionVector, receive_and_ack, record_upsert};

fn main() {
    let local = VersionVector::from_entries([("phone".to_owned(), 2)]).expect("vector");
    let write = record_upsert(
        "notes/42",
        "mutation-3",
        "desktop",
        &local,
        serde_json::json!({"text": "draft"}),
    )
    .expect("record");
    assert!(syncer_rs::same_transaction_pair(&write));

    let (envelope, snapshot) = write.into_persistable();
    let checkpoint = VersionVector::from_entries([("phone".to_owned(), 2)]).expect("checkpoint");
    let ack = receive_and_ack(&envelope, &checkpoint).expect("ack");
    assert!(matches!(ack, OptimisticAck::Applied { .. }));
    assert_eq!(snapshot.relation(ack.checkpoint()), VersionRelation::Equal);
    println!("optimistic cycle: record then acknowledge");
}
