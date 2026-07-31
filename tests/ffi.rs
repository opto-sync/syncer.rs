use std::ffi::{CStr, CString};

use syncer_rs::ffi::{
    SyncerRsOptions, syncer_rs_free, syncer_rs_merge_json, syncer_rs_merge_json_ex,
    syncer_rs_version,
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
