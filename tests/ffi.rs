use std::ffi::{CStr, CString};

use syncer_rs::ffi::{
    SYNCER_RS_OPT_ERR_CONFLICT, SYNCER_RS_OPT_ERR_MISSING_REPLICA, SYNCER_RS_OPT_ERR_STALE_VECTOR,
    SYNCER_RS_OPT_OK, SyncerRsOptions, syncer_rs_free, syncer_rs_merge_json,
    syncer_rs_merge_json_ex, syncer_rs_optimistic_receive, syncer_rs_optimistic_record,
    syncer_rs_version,
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
