//! Desktop optimistic cycle: validate, classify, then acknowledge.
//!
//! `syncer.rs` owns merge + causal ordering only. SQLite persistence and
//! process lifecycle belong in `opto-sync-clients/desktop-rust`.

use syncer_rs::{CausalDisposition, CausalEnvelope, VersionVector};

fn main() {
    let mut producer = VersionVector::from_entries([("phone".to_owned(), 2)]).expect("vector");
    let envelope = CausalEnvelope::upsert(
        "notes/42",
        "mutation-3",
        "desktop",
        &mut producer,
        serde_json::json!({"text": "draft"}),
    )
    .expect("envelope");
    envelope.validate().expect("valid wire envelope");

    let mut checkpoint =
        VersionVector::from_entries([("phone".to_owned(), 2)]).expect("checkpoint");
    assert_eq!(
        envelope.disposition_against(&checkpoint),
        CausalDisposition::Apply
    );
    envelope
        .acknowledge_into(&mut checkpoint)
        .expect("acknowledge");
    assert_eq!(checkpoint.get("phone"), 2);
    assert_eq!(checkpoint.get("desktop"), 1);
    println!("optimistic cycle: apply then acknowledge");
}
