//! Stable C ABI used by Flutter/Dart FFI and other native hosts.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::{
    merge_optional_json, ArrayMergeStrategy, CausalDisposition, CausalEnvelope,
    CausalEnvelopeError, MergeError, MergeOptions, VersionVector, VersionVectorError, VERSION,
};

/// Success from a typed causal FFI call.
pub const SYNCER_RS_OK: i32 = 0;
/// A required pointer was null.
pub const SYNCER_RS_ERR_NULL: i32 = 1;
/// A C string was not valid UTF-8.
pub const SYNCER_RS_ERR_UTF8: i32 = 2;
/// The JSON payload was malformed or the wrong shape.
pub const SYNCER_RS_ERR_JSON: i32 = 3;
/// The envelope schema version is unsupported.
pub const SYNCER_RS_ERR_SCHEMA: i32 = 4;
/// The document id failed validation.
pub const SYNCER_RS_ERR_DOCUMENT: i32 = 5;
/// The mutation id failed validation.
pub const SYNCER_RS_ERR_MUTATION: i32 = 6;
/// The actor replica has no positive counter.
pub const SYNCER_RS_ERR_ACTOR: i32 = 7;
/// Version-vector validation or merge failed.
pub const SYNCER_RS_ERR_VECTOR: i32 = 8;
/// The Rust side panicked; treat as a failed call.
pub const SYNCER_RS_ERR_PANIC: i32 = 9;

/// [`CausalDisposition::Duplicate`]
pub const SYNCER_RS_DISP_DUPLICATE: i32 = 0;
/// [`CausalDisposition::Stale`]
pub const SYNCER_RS_DISP_STALE: i32 = 1;
/// [`CausalDisposition::Apply`]
pub const SYNCER_RS_DISP_APPLY: i32 = 2;
/// [`CausalDisposition::ResolveConcurrent`]
pub const SYNCER_RS_DISP_CONCURRENT: i32 = 3;

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
/// or `syncer_rs_causal_*` function from this exact library instance. It must
/// not have been freed previously.
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

fn write_cstring(out: *mut *mut c_char, message: &str) -> Result<(), i32> {
    if out.is_null() {
        return Ok(());
    }
    let encoded = CString::new(message).map_err(|_| SYNCER_RS_ERR_JSON)?;
    // SAFETY: `out` is a writable pointer supplied by the caller.
    unsafe {
        *out = encoded.into_raw();
    }
    Ok(())
}

fn required_string(pointer: *const c_char) -> Result<String, i32> {
    if pointer.is_null() {
        return Err(SYNCER_RS_ERR_NULL);
    }
    // SAFETY: The caller promises a readable NUL-terminated C string.
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| SYNCER_RS_ERR_UTF8)
}

fn causal_error_code(error: CausalEnvelopeError) -> i32 {
    match error {
        CausalEnvelopeError::UnsupportedSchema(_) => SYNCER_RS_ERR_SCHEMA,
        CausalEnvelopeError::InvalidDocumentId => SYNCER_RS_ERR_DOCUMENT,
        CausalEnvelopeError::InvalidMutationId => SYNCER_RS_ERR_MUTATION,
        CausalEnvelopeError::MissingActorCounter(_) => SYNCER_RS_ERR_ACTOR,
        CausalEnvelopeError::VersionVector(_) => SYNCER_RS_ERR_VECTOR,
    }
}

fn vector_error_code(_error: VersionVectorError) -> i32 {
    SYNCER_RS_ERR_VECTOR
}

fn parse_envelope(json: &str) -> Result<CausalEnvelope<serde_json::Value>, i32> {
    let envelope: CausalEnvelope<serde_json::Value> =
        serde_json::from_str(json).map_err(|_| SYNCER_RS_ERR_JSON)?;
    envelope.validate().map_err(causal_error_code)?;
    Ok(envelope)
}

fn parse_checkpoint(json: &str) -> Result<VersionVector, i32> {
    serde_json::from_str(json).map_err(|_| SYNCER_RS_ERR_JSON)
}

fn disposition_code(disposition: CausalDisposition) -> i32 {
    match disposition {
        CausalDisposition::Duplicate => SYNCER_RS_DISP_DUPLICATE,
        CausalDisposition::Stale => SYNCER_RS_DISP_STALE,
        CausalDisposition::Apply => SYNCER_RS_DISP_APPLY,
        CausalDisposition::ResolveConcurrent => SYNCER_RS_DISP_CONCURRENT,
    }
}

fn catch_causal(work: impl FnOnce() -> Result<i32, i32>) -> i32 {
    catch_unwind(AssertUnwindSafe(work))
        .unwrap_or(Ok(SYNCER_RS_ERR_PANIC))
        .unwrap_or_else(|code| code)
}

/// Validates a causal envelope JSON string.
///
/// Returns [`SYNCER_RS_OK`] on success. On failure, writes a diagnostic to
/// `error_out` when that pointer is non-null and returns a typed error code.
/// Release `error_out` with [`syncer_rs_free`].
///
/// # Safety
///
/// `envelope_json` must be a readable NUL-terminated C string. `error_out`,
/// when non-null, must point to writable storage for one `char *`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_causal_validate(
    envelope_json: *const c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    catch_causal(|| {
        let json = required_string(envelope_json)?;
        match parse_envelope(&json) {
            Ok(_) => Ok(SYNCER_RS_OK),
            Err(code) => {
                let _ = write_cstring(error_out, "causal envelope failed validation");
                Err(code)
            }
        }
    })
}

/// Classifies `envelope_json` against a durable checkpoint JSON object.
///
/// On success writes a [`SYNCER_RS_DISP_*`] code to `disposition_out`.
///
/// # Safety
///
/// All non-null C strings must be readable and NUL-terminated. Output pointers
/// must be writable for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_causal_disposition(
    envelope_json: *const c_char,
    checkpoint_json: *const c_char,
    disposition_out: *mut i32,
    error_out: *mut *mut c_char,
) -> i32 {
    catch_causal(|| {
        if disposition_out.is_null() {
            return Err(SYNCER_RS_ERR_NULL);
        }
        let envelope = match parse_envelope(&required_string(envelope_json)?) {
            Ok(envelope) => envelope,
            Err(code) => {
                let _ = write_cstring(error_out, "causal envelope failed validation");
                return Err(code);
            }
        };
        let checkpoint = match parse_checkpoint(&required_string(checkpoint_json)?) {
            Ok(checkpoint) => checkpoint,
            Err(code) => {
                let _ = write_cstring(error_out, "causal checkpoint failed validation");
                return Err(code);
            }
        };
        // SAFETY: Caller supplied a writable i32.
        unsafe {
            *disposition_out = disposition_code(envelope.disposition_against(&checkpoint));
        }
        Ok(SYNCER_RS_OK)
    })
}

/// Merges an accepted envelope's clock into `checkpoint_json`.
///
/// Writes the joined checkpoint JSON to `checkpoint_out`. Release it with
/// [`syncer_rs_free`].
///
/// # Safety
///
/// Same pointer contract as [`syncer_rs_causal_disposition`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syncer_rs_causal_acknowledge(
    envelope_json: *const c_char,
    checkpoint_json: *const c_char,
    checkpoint_out: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    catch_causal(|| {
        if checkpoint_out.is_null() {
            return Err(SYNCER_RS_ERR_NULL);
        }
        let envelope = match parse_envelope(&required_string(envelope_json)?) {
            Ok(envelope) => envelope,
            Err(code) => {
                let _ = write_cstring(error_out, "causal envelope failed validation");
                return Err(code);
            }
        };
        let mut checkpoint = match parse_checkpoint(&required_string(checkpoint_json)?) {
            Ok(checkpoint) => checkpoint,
            Err(code) => {
                let _ = write_cstring(error_out, "causal checkpoint failed validation");
                return Err(code);
            }
        };
        if let Err(error) = envelope.acknowledge_into(&mut checkpoint) {
            let _ = write_cstring(error_out, &error.to_string());
            return Err(vector_error_code(error));
        }
        let encoded = serde_json::to_string(&checkpoint).map_err(|_| SYNCER_RS_ERR_JSON)?;
        write_cstring(checkpoint_out, &encoded)?;
        Ok(SYNCER_RS_OK)
    })
}
