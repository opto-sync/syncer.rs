use std::ffi::{CStr, CString};

use syncer_rs::ffi::{
    SYNCER_RS_DISP_APPLY, SYNCER_RS_ERR_JSON, SYNCER_RS_ERR_NULL, SYNCER_RS_OK, SyncerRsOptions,
    syncer_rs_causal_acknowledge, syncer_rs_causal_disposition, syncer_rs_causal_validate,
    syncer_rs_free, syncer_rs_merge_json, syncer_rs_merge_json_ex, syncer_rs_version,
};

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
    let mut clock = syncer_rs::VersionVector::from_entries([("phone".into(), 2)])
        .expect("checkpoint vector");
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
    let mut error: *mut std::ffi::c_char = std::ptr::null_mut();
    // SAFETY: Strings are valid and error_out is writable.
    let status = unsafe { syncer_rs_causal_validate(envelope.as_ptr(), &mut error) };
    assert_eq!(status, SYNCER_RS_OK);
    assert!(error.is_null());

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
