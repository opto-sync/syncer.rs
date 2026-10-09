use std::ffi::{CStr, CString};

use syncer_rs::ffi::{
    SYNCER_RS_DISP_APPLY, SYNCER_RS_ERR_JSON, SYNCER_RS_ERR_NULL, SYNCER_RS_OK,
    SYNCER_RS_OPT_ERR_CONFLICT, SYNCER_RS_OPT_ERR_MISSING_REPLICA, SYNCER_RS_OPT_ERR_STALE_VECTOR,
    SYNCER_RS_OPT_OK, SyncerRsOptions, syncer_rs_causal_acknowledge, syncer_rs_causal_disposition,
    syncer_rs_causal_validate, syncer_rs_free, syncer_rs_merge_json, syncer_rs_merge_json_ex,
    syncer_rs_optimistic_receive, syncer_rs_optimistic_record, syncer_rs_version,
};
use syncer_rs::{CausalEnvelope, VersionRelation, VersionVector};

#[test]
fn ffi_default_merge_and_memory_handoff_work() {
    let base = CString::new(r#"{"a":1}"#).unwrap();
    let incoming = CString::new(r#"{"b":2}"#).unwrap();
    // SAFETY: Inputs are valid NUL-terminated strings and the result is freed
    // with the matching library function.
    let result = unsafe { syncer_rs_merge_json(base.as_ptr(), incoming.as_ptr()) };
    assert!(!result.is_null());
    // SAFETY: The successful result is a valid NUL-terminated library string.
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
        r#"{"a":1,"b":2}"#
    );
    // SAFETY: The pointer came from syncer_rs_merge_json and is freed once.
    unsafe { syncer_rs_free(result) };
}

#[test]
fn ffi_options_and_invalid_values_work() {
    let base = CString::new(r#"{"items":[{"id":1,"a":1}]}"#).unwrap();
    let incoming = CString::new(r#"{"items":[{"id":1,"b":2}]}"#).unwrap();
    let mut options = SyncerRsOptions {
        array_strategy: 4,
        detect_circular_refs: true,
        ..SyncerRsOptions::default()
    };

    // SAFETY: Every pointer remains valid for the duration of the call.
    let result = unsafe { syncer_rs_merge_json_ex(base.as_ptr(), incoming.as_ptr(), &options) };
    assert!(!result.is_null());
    // SAFETY: The successful result is a valid library string.
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
        r#"{"items":[{"id":1,"a":1,"b":2}]}"#
    );
    // SAFETY: The pointer came from this library and is freed once.
    unsafe { syncer_rs_free(result) };

    options.array_strategy = 99;
    // SAFETY: Pointers are valid; the invalid option should return NULL.
    let invalid = unsafe { syncer_rs_merge_json_ex(base.as_ptr(), incoming.as_ptr(), &options) };
    assert!(invalid.is_null());

    options.array_strategy = 4;
    options.abi_version = 1;
    // SAFETY: The struct has the current layout, but the explicit legacy ABI
    // discriminator must be rejected rather than interpreted ambiguously.
    let legacy = unsafe { syncer_rs_merge_json_ex(base.as_ptr(), incoming.as_ptr(), &options) };
    assert!(legacy.is_null());
}

#[test]
fn ffi_version_is_static_semver() {
    // SAFETY: syncer_rs_version returns a static NUL-terminated string.
    let version = unsafe { CStr::from_ptr(syncer_rs_version()) }
        .to_str()
        .unwrap();
    assert_eq!(version.split('.').count(), 3);
}

#[test]
fn ffi_optimistic_write_and_ack_converges_the_vector() {
    let document = CString::new("notes/42").unwrap();
    let mutation = CString::new("mutation-3").unwrap();
    let replica = CString::new("desktop").unwrap();
    let clock = CString::new(r#"{"phone":2}"#).unwrap();
    let payload = CString::new(r#"{"text":"draft","token":"s3cret-value"}"#).unwrap();
    let mut envelope_ptr = std::ptr::null_mut();
    let mut snapshot_ptr = std::ptr::null_mut();

    // SAFETY: Inputs are valid C strings and outputs are freed once.
    let recorded = unsafe {
        syncer_rs_optimistic_record(
            document.as_ptr(),
            mutation.as_ptr(),
            replica.as_ptr(),
            clock.as_ptr(),
            payload.as_ptr(),
            &mut envelope_ptr,
            &mut snapshot_ptr,
        )
    };
    assert_eq!(recorded, SYNCER_RS_OPT_OK);
    assert!(!envelope_ptr.is_null());
    assert!(!snapshot_ptr.is_null());

    // SAFETY: Successful record returns library-owned strings.
    let envelope_json = unsafe { CStr::from_ptr(envelope_ptr) }.to_str().unwrap();
    let snapshot_json = unsafe { CStr::from_ptr(snapshot_ptr) }.to_str().unwrap();
    let envelope: CausalEnvelope<serde_json::Value> = serde_json::from_str(envelope_json).unwrap();
    let snapshot: VersionVector = serde_json::from_str(snapshot_json).unwrap();
    assert_eq!(envelope.clock.relation(&snapshot), VersionRelation::Equal);
    assert_eq!(snapshot.get("desktop"), 1);
    assert_eq!(snapshot.get("phone"), 2);

    let checkpoint = CString::new(r#"{"phone":2}"#).unwrap();
    let mut next_ptr = std::ptr::null_mut();
    // SAFETY: Envelope and checkpoint are valid C strings.
    let received =
        unsafe { syncer_rs_optimistic_receive(envelope_ptr, checkpoint.as_ptr(), &mut next_ptr) };
    assert_eq!(received, SYNCER_RS_OPT_OK);
    // SAFETY: Successful receive returns a library-owned checkpoint.
    let next_json = unsafe { CStr::from_ptr(next_ptr) }.to_str().unwrap();
    let next: VersionVector = serde_json::from_str(next_json).unwrap();
    assert_eq!(snapshot.relation(&next), VersionRelation::Equal);

    unsafe {
        syncer_rs_free(envelope_ptr);
        syncer_rs_free(snapshot_ptr);
        syncer_rs_free(next_ptr);
    }
}

#[test]
fn ffi_optimistic_typed_errors_do_not_ack_conflicts() {
    let document = CString::new("notes/1").unwrap();
    let mutation = CString::new("m-desk").unwrap();
    let replica = CString::new("desktop").unwrap();
    let clock = CString::new(r#"{"phone":2}"#).unwrap();
    let payload = CString::new(r#"{"v":1}"#).unwrap();
    let mut envelope_ptr = std::ptr::null_mut();
    let mut snapshot_ptr = std::ptr::null_mut();
    // SAFETY: Valid C strings for a concurrent receive fixture.
    let recorded = unsafe {
        syncer_rs_optimistic_record(
            document.as_ptr(),
            mutation.as_ptr(),
            replica.as_ptr(),
            clock.as_ptr(),
            payload.as_ptr(),
            &mut envelope_ptr,
            &mut snapshot_ptr,
        )
    };
    assert_eq!(recorded, SYNCER_RS_OPT_OK);

    let concurrent = CString::new(r#"{"phone":3}"#).unwrap();
    let mut next_ptr = std::ptr::null_mut();
    // SAFETY: Envelope from this library; checkpoint is a valid C string.
    let conflict =
        unsafe { syncer_rs_optimistic_receive(envelope_ptr, concurrent.as_ptr(), &mut next_ptr) };
    assert_eq!(conflict, SYNCER_RS_OPT_ERR_CONFLICT);
    assert!(next_ptr.is_null());

    let stale = CString::new(r#"{"phone":2,"desktop":4}"#).unwrap();
    let stale_code =
        unsafe { syncer_rs_optimistic_receive(envelope_ptr, stale.as_ptr(), &mut next_ptr) };
    assert_eq!(stale_code, SYNCER_RS_OPT_ERR_STALE_VECTOR);

    let bad_replica = CString::new("desktop space").unwrap();
    let mut unused_envelope = std::ptr::null_mut();
    let mut unused_snapshot = std::ptr::null_mut();
    let missing = unsafe {
        syncer_rs_optimistic_record(
            document.as_ptr(),
            mutation.as_ptr(),
            bad_replica.as_ptr(),
            clock.as_ptr(),
            payload.as_ptr(),
            &mut unused_envelope,
            &mut unused_snapshot,
        )
    };
    assert_eq!(missing, SYNCER_RS_OPT_ERR_MISSING_REPLICA);
    assert!(unused_envelope.is_null());
    assert!(unused_snapshot.is_null());

    unsafe {
        syncer_rs_free(envelope_ptr);
        syncer_rs_free(snapshot_ptr);
    }
}

#[test]
fn ffi_optimistic_calls_clear_outputs_before_failure() {
    let stale_envelope = CString::new("stale envelope").unwrap().into_raw();
    let stale_snapshot = CString::new("stale snapshot").unwrap().into_raw();
    let mut envelope_out = stale_envelope;
    let mut snapshot_out = stale_snapshot;

    // SAFETY: Both output slots are writable; the null required input must
    // fail closed after clearing caller-visible output pointers.
    let status = unsafe {
        syncer_rs_optimistic_record(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            &mut envelope_out,
            &mut snapshot_out,
        )
    };
    assert_ne!(status, SYNCER_RS_OPT_OK);
    assert!(envelope_out.is_null());
    assert!(snapshot_out.is_null());

    let stale_checkpoint = CString::new("stale checkpoint").unwrap().into_raw();
    let mut checkpoint_out = stale_checkpoint;
    // SAFETY: Output is writable; null inputs must fail closed.
    let status = unsafe {
        syncer_rs_optimistic_receive(std::ptr::null(), std::ptr::null(), &mut checkpoint_out)
    };
    assert_ne!(status, SYNCER_RS_OPT_OK);
    assert!(checkpoint_out.is_null());

    // SAFETY: Clearing the slots does not transfer ownership of the deliberately
    // stale allocations to the library, so this test reclaims each exactly once.
    unsafe {
        drop(CString::from_raw(stale_envelope));
        drop(CString::from_raw(stale_snapshot));
        drop(CString::from_raw(stale_checkpoint));
    }
}

#[test]
fn ffi_one_sided_merge_validates_and_normalizes() {
    let input = CString::new(r#"{ "a": 1 }"#).unwrap();
    // SAFETY: One input may be null and the other is a valid C string.
    let result =
        unsafe { syncer_rs_merge_json_ex(std::ptr::null(), input.as_ptr(), std::ptr::null()) };
    assert!(!result.is_null());
    // SAFETY: The successful result is a valid library string.
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
        r#"{"a":1}"#
    );
    // SAFETY: The pointer came from this library and is freed once.
    unsafe { syncer_rs_free(result) };
}

fn sample_envelope_and_checkpoint() -> (CString, CString) {
    let mut clock =
        syncer_rs::VersionVector::from_entries([("phone".into(), 2)]).expect("checkpoint vector");
    let envelope = syncer_rs::CausalEnvelope::upsert(
        "notes/42",
        "mutation-3",
        "desktop",
        &mut clock,
        serde_json::json!({"text": "ok"}),
    )
    .expect("envelope");
    let envelope_json = CString::new(serde_json::to_string(&envelope).expect("json")).expect("c");
    let checkpoint = CString::new(r#"{"phone":2}"#).expect("checkpoint");
    (envelope_json, checkpoint)
}

#[test]
fn ffi_causal_validate_disposition_and_acknowledge() {
    let (envelope, checkpoint) = sample_envelope_and_checkpoint();
    let stale_error = CString::new("stale error").unwrap().into_raw();
    let mut error = stale_error;
    // SAFETY: Strings are valid and error_out is writable.
    let status = unsafe { syncer_rs_causal_validate(envelope.as_ptr(), &mut error) };
    assert_eq!(status, SYNCER_RS_OK);
    assert!(error.is_null());
    // SAFETY: The API cleared the output slot without taking ownership of the
    // caller's deliberately stale test allocation.
    drop(unsafe { CString::from_raw(stale_error) });

    let mut disposition = -1;
    // SAFETY: Pointers remain valid for the call.
    let status = unsafe {
        syncer_rs_causal_disposition(
            envelope.as_ptr(),
            checkpoint.as_ptr(),
            &mut disposition,
            &mut error,
        )
    };
    assert_eq!(status, SYNCER_RS_OK);
    assert_eq!(disposition, SYNCER_RS_DISP_APPLY);

    let mut joined: *mut std::ffi::c_char = std::ptr::null_mut();
    // SAFETY: Outputs are writable and inputs are valid C strings.
    let status = unsafe {
        syncer_rs_causal_acknowledge(
            envelope.as_ptr(),
            checkpoint.as_ptr(),
            &mut joined,
            &mut error,
        )
    };
    assert_eq!(status, SYNCER_RS_OK);
    assert!(!joined.is_null());
    // SAFETY: joined came from the library.
    let text = unsafe { CStr::from_ptr(joined) }.to_str().expect("utf8");
    let value: serde_json::Value = serde_json::from_str(text).expect("json");
    assert_eq!(value["phone"], 2);
    assert_eq!(value["desktop"], 1);
    unsafe { syncer_rs_free(joined) };
}

#[test]
fn ffi_causal_rejects_null_and_malformed_json() {
    let mut error: *mut std::ffi::c_char = std::ptr::null_mut();
    // SAFETY: A null envelope must fail closed with a typed code.
    let status = unsafe { syncer_rs_causal_validate(std::ptr::null(), &mut error) };
    assert_eq!(status, SYNCER_RS_ERR_NULL);

    let bad = CString::new("{").unwrap();
    // SAFETY: Input is a valid C string of invalid JSON.
    let status = unsafe { syncer_rs_causal_validate(bad.as_ptr(), &mut error) };
    assert_eq!(status, SYNCER_RS_ERR_JSON);
    if !error.is_null() {
        unsafe { syncer_rs_free(error) };
    }
}

#[test]
fn ffi_causal_acknowledge_clears_checkpoint_output_before_failure() {
    let bad = CString::new("{").unwrap();
    let checkpoint = CString::new("{}").unwrap();
    let stale_checkpoint = CString::new("stale checkpoint").unwrap().into_raw();
    let mut checkpoint_out = stale_checkpoint;
    let mut error: *mut std::ffi::c_char = std::ptr::null_mut();

    // SAFETY: Inputs are valid C strings and both output slots are writable.
    let status = unsafe {
        syncer_rs_causal_acknowledge(
            bad.as_ptr(),
            checkpoint.as_ptr(),
            &mut checkpoint_out,
            &mut error,
        )
    };
    assert_eq!(status, SYNCER_RS_ERR_JSON);
    assert!(checkpoint_out.is_null());
    // SAFETY: The API cleared the output slot without taking ownership of the
    // caller's deliberately stale test allocation.
    drop(unsafe { CString::from_raw(stale_checkpoint) });
    if !error.is_null() {
        unsafe { syncer_rs_free(error) };
    }
}
