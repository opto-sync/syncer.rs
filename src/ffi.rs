//! Stable C ABI used by Flutter/Dart FFI and other native hosts.

use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::{
    ArrayMergeStrategy, CausalEnvelope, MergeError, MergeOptions, OptimisticError, VERSION,
    VersionVector, merge_optional_json, receive_and_ack, record_upsert,
};

/// Success from an optimistic FFI call.
pub const SYNCER_RS_OPT_OK: i32 = 0;
/// Concurrent clocks; the host must resolve before acknowledging.
pub const SYNCER_RS_OPT_ERR_CONFLICT: i32 = 1;
/// Replica id is invalid or the envelope clock has no actor counter.
pub const SYNCER_RS_OPT_ERR_MISSING_REPLICA: i32 = 2;
/// Incoming clock is behind the checkpoint, or a stored pair drifted.
pub const SYNCER_RS_OPT_ERR_STALE_VECTOR: i32 = 3;
/// JSON, UTF-8, null pointer, or other input validation failed.
pub const SYNCER_RS_OPT_ERR_INVALID: i32 = 4;
/// The Rust side panicked; treat as a failed call.
pub const SYNCER_RS_OPT_ERR_PANIC: i32 = 5;

/// Current layout version of [`SyncerRsOptions`].
pub const SYNCER_RS_ABI_VERSION: u32 = 2;

/// C-compatible options. Keep this synchronized with `include/syncer_rs.h`.
#[derive(Debug)]
#[repr(C)]
pub struct SyncerRsOptions {
    pub abi_version: u32,
    pub array_strategy: i32,
    pub max_depth: u32,
    pub resolve_by_timestamp: bool,
    pub detect_circular_refs: bool,
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
            detect_circular_refs: false,
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

    // Read only the stable integer prefix before interpreting the complete
    // structure. ABI v1 used the byte now occupied by detect_circular_refs as
    // padding, so constructing a v2 Rust reference first could inspect an
    // uninitialized/invalid bool before we had a chance to reject the version.
    // SAFETY: Every supported options structure starts with a readable u32 ABI
    // discriminator. read_unaligned also avoids relying on foreign alignment.
    let abi_version = unsafe { ptr::read_unaligned(pointer.cast::<u32>()) };
    if abi_version != SYNCER_RS_ABI_VERSION {
        return Err(MergeError::InvalidOptions(format!(
            "ABI version {abi_version} is unsupported; expected {SYNCER_RS_ABI_VERSION}"
        )));
    }

    // SAFETY: The accepted v2 discriminator promises a fully initialized v2
    // SyncerRsOptions with valid field representations.
    let options = unsafe { &*pointer };

    Ok(MergeOptions {
        array_strategy: ArrayMergeStrategy::try_from(options.array_strategy)?,
        max_depth: options.max_depth,
        resolve_by_timestamp: options.resolve_by_timestamp,
        detect_circular_refs: options.detect_circular_refs,
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
/// or `syncer_rs_optimistic_*` function from this exact library instance. It
/// must not have been freed previously.
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

fn required_utf8(pointer: *const c_char) -> Result<String, i32> {
    if pointer.is_null() {
        return Err(SYNCER_RS_OPT_ERR_INVALID);
    }
    // SAFETY: The caller promises a readable NUL-terminated C string.
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| SYNCER_RS_OPT_ERR_INVALID)
}

fn write_out(out: *mut *mut c_char, message: &str) -> Result<(), i32> {
    if out.is_null() {
        return Err(SYNCER_RS_OPT_ERR_INVALID);
    }
    let encoded = CString::new(message).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
    // SAFETY: `out` is a writable pointer supplied by the caller.
    unsafe {
        *out = encoded.into_raw();
    }
    Ok(())
}

fn optimistic_code(error: &OptimisticError) -> i32 {
    match error {
        OptimisticError::Conflict { .. } => SYNCER_RS_OPT_ERR_CONFLICT,
        OptimisticError::MissingReplica { .. } => SYNCER_RS_OPT_ERR_MISSING_REPLICA,
        OptimisticError::StaleVector => SYNCER_RS_OPT_ERR_STALE_VECTOR,
        OptimisticError::Envelope(_) | OptimisticError::VersionVector(_) => {
            SYNCER_RS_OPT_ERR_INVALID
        }
    }
}

fn catch_optimistic(work: impl FnOnce() -> Result<i32, i32>) -> i32 {
    catch_unwind(AssertUnwindSafe(work))
        .unwrap_or(Ok(SYNCER_RS_OPT_ERR_PANIC))
        .unwrap_or_else(|code| code)
}

/// Records an optimistic upsert for Flutter/Dart FFI and Rust desktop hosts.
///
/// On success, `envelope_out` and `snapshot_out` receive JSON that must be
/// persisted in the same local transaction. Release both with
/// [`syncer_rs_free`]. Diagnostics name error kinds only; payloads are never
/// copied into error strings.
///
/// # Safety
///
/// Every input must be a readable NUL-terminated C string. Output pointers
/// must be writable for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_optimistic_record(
    document_id: *const c_char,
    mutation_id: *const c_char,
    replica_id: *const c_char,
    clock_json: *const c_char,
    payload_json: *const c_char,
    envelope_out: *mut *mut c_char,
    snapshot_out: *mut *mut c_char,
) -> i32 {
    catch_optimistic(|| {
        let document_id = required_utf8(document_id)?;
        let mutation_id = required_utf8(mutation_id)?;
        let replica_id = required_utf8(replica_id)?;
        let clock_json = required_utf8(clock_json)?;
        let payload_json = required_utf8(payload_json)?;
        let clock: VersionVector =
            serde_json::from_str(&clock_json).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        let payload: serde_json::Value =
            serde_json::from_str(&payload_json).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        let write = record_upsert(document_id, mutation_id, replica_id, &clock, payload)
            .map_err(|error| optimistic_code(&error))?;
        let envelope =
            serde_json::to_string(write.envelope()).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        let snapshot =
            serde_json::to_string(write.snapshot()).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        write_out(envelope_out, &envelope)?;
        if let Err(code) = write_out(snapshot_out, &snapshot) {
            // SAFETY: write_out just stored a library-owned CString here.
            unsafe {
                let leaked = *envelope_out;
                *envelope_out = ptr::null_mut();
                if !leaked.is_null() {
                    drop(CString::from_raw(leaked));
                }
            }
            return Err(code);
        }
        Ok(SYNCER_RS_OPT_OK)
    })
}

/// Receives an envelope against a durable checkpoint JSON object.
///
/// On Apply or Duplicate, writes the next checkpoint to `checkpoint_out`.
/// Concurrent clocks return [`SYNCER_RS_OPT_ERR_CONFLICT`] without merging.
/// Stale clocks return [`SYNCER_RS_OPT_ERR_STALE_VECTOR`]. Release
/// `checkpoint_out` with [`syncer_rs_free`].
///
/// # Safety
///
/// Same pointer contract as [`syncer_rs_optimistic_record`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_optimistic_receive(
    envelope_json: *const c_char,
    checkpoint_json: *const c_char,
    checkpoint_out: *mut *mut c_char,
) -> i32 {
    catch_optimistic(|| {
        let envelope_json = required_utf8(envelope_json)?;
        let checkpoint_json = required_utf8(checkpoint_json)?;
        let envelope: CausalEnvelope<serde_json::Value> =
            serde_json::from_str(&envelope_json).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        let checkpoint: VersionVector =
            serde_json::from_str(&checkpoint_json).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        let ack =
            receive_and_ack(&envelope, &checkpoint).map_err(|error| optimistic_code(&error))?;
        let encoded =
            serde_json::to_string(ack.checkpoint()).map_err(|_| SYNCER_RS_OPT_ERR_INVALID)?;
        write_out(checkpoint_out, &encoded)?;
        Ok(SYNCER_RS_OPT_OK)
    })
}
