//! Stable C ABI used by Flutter/Dart FFI and other native hosts.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::{ArrayMergeStrategy, MergeError, MergeOptions, VERSION, merge_optional_json};

/// Current layout version of [`SyncerRsOptions`].
pub const SYNCER_RS_ABI_VERSION: u32 = 1;

/// C-compatible options. Keep this synchronized with `include/syncer_rs.h`.
#[derive(Debug)]
#[repr(C)]
pub struct SyncerRsOptions {
    pub abi_version: u32,
    pub array_strategy: i32,
    pub max_depth: u32,
    pub resolve_by_timestamp: bool,
    pub lww_keys: *const c_char,
    pub fww_keys: *const c_char,
    pub array_match_keys: *const c_char,
}

impl Default for SyncerRsOptions {
    fn default() -> Self {
        Self {
            abi_version: SYNCER_RS_ABI_VERSION,
            array_strategy: ArrayMergeStrategy::Replace as i32,
            max_depth: 0,
            resolve_by_timestamp: false,
            lww_keys: ptr::null(),
            fww_keys: ptr::null(),
            array_match_keys: ptr::null(),
        }
    }
}

/// Reconciles two required JSON strings with default options.
///
/// Returns `NULL` on malformed input or panic. Release successful results with
/// [`syncer_rs_free`].
///
/// # Safety
///
/// Each non-null input must point to a readable, NUL-terminated C string and
/// remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_merge_json(
    base: *const c_char,
    incoming: *const c_char,
) -> *mut c_char {
    // SAFETY: Validation and pointer reads are centralized in merge_from_ffi.
    unsafe { merge_from_ffi(base, incoming, ptr::null()) }
}

/// Reconciles optional JSON strings using C-compatible options.
///
/// One input may be `NULL`, in which case the other is validated and
/// normalized. Both inputs being `NULL` is an error.
///
/// # Safety
///
/// Each non-null JSON input must point to a readable, NUL-terminated C string.
/// A non-null `options` pointer must reference a properly aligned, fully
/// initialized [`SyncerRsOptions`]. All referenced memory must remain valid for
/// the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_merge_json_ex(
    base: *const c_char,
    incoming: *const c_char,
    options: *const SyncerRsOptions,
) -> *mut c_char {
    // SAFETY: Validation and pointer reads are centralized in merge_from_ffi.
    unsafe { merge_from_ffi(base, incoming, options) }
}

unsafe fn merge_from_ffi(
    base: *const c_char,
    incoming: *const c_char,
    options: *const SyncerRsOptions,
) -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Foreign pointers are read only after null checks. Valid,
        // NUL-terminated strings remain the caller's responsibility.
        let base = unsafe { optional_string(base) }?;
        // SAFETY: Same contract as `base`.
        let incoming = unsafe { optional_string(incoming) }?;
        // SAFETY: A non-null pointer must reference this ABI's full struct.
        let options = unsafe { options_from_ffi(options) }.map_err(|_| ())?;
        let result = merge_optional_json(base.as_deref(), incoming.as_deref(), &options)
            .map_err(|_| ())?
            .ok_or(())?;
        CString::new(result).map(CString::into_raw).map_err(|_| ())
    }))
    .ok()
    .and_then(Result::ok)
    .unwrap_or(ptr::null_mut())
}

unsafe fn optional_string(pointer: *const c_char) -> Result<Option<String>, ()> {
    if pointer.is_null() {
        return Ok(None);
    }
    // SAFETY: The caller promises a readable NUL-terminated C string.
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(|value| Some(value.to_owned()))
        .map_err(|_| ())
}

unsafe fn options_from_ffi(pointer: *const SyncerRsOptions) -> Result<MergeOptions, MergeError> {
    if pointer.is_null() {
        return Ok(MergeOptions::default());
    }

    // SAFETY: The caller promises a properly aligned SyncerRsOptions.
    let options = unsafe { &*pointer };
    if options.abi_version != SYNCER_RS_ABI_VERSION {
        return Err(MergeError::InvalidOptions(format!(
            "ABI version {} is unsupported; expected {}",
            options.abi_version, SYNCER_RS_ABI_VERSION
        )));
    }

    Ok(MergeOptions {
        array_strategy: ArrayMergeStrategy::try_from(options.array_strategy)?,
        max_depth: options.max_depth,
        resolve_by_timestamp: options.resolve_by_timestamp,
        // SAFETY: Optional strings follow the same C string contract.
        lww_keys: unsafe { optional_string(options.lww_keys) }
            .map_err(|_| MergeError::InvalidOptions("lww_keys is not UTF-8".to_owned()))?,
        // SAFETY: Optional strings follow the same C string contract.
        fww_keys: unsafe { optional_string(options.fww_keys) }
            .map_err(|_| MergeError::InvalidOptions("fww_keys is not UTF-8".to_owned()))?,
        // SAFETY: Optional strings follow the same C string contract.
        array_match_keys: unsafe { optional_string(options.array_match_keys) }
            .map_err(|_| MergeError::InvalidOptions("array_match_keys is not UTF-8".to_owned()))?,
    })
}

/// Releases a string returned by this library. Passing `NULL` is allowed.
///
/// # Safety
///
/// `pointer` must be null or a live pointer returned by a `syncer_rs_merge_*`
/// function from this exact library instance. It must not have been freed
/// previously.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        // SAFETY: This function may only receive pointers returned by
        // CString::into_raw from this library.
        drop(unsafe { CString::from_raw(pointer) });
    }
}

/// Returns a static `major.minor.patch` version string. Do not free it.
#[unsafe(no_mangle)]
pub extern "C" fn syncer_rs_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

const _: &str = VERSION;
