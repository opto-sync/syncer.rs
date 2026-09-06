//! Rust-native JSON reconciliation for native callers, C-compatible FFI, and
//! WebAssembly.
//!
//! The merge is a pure function: the base document is reconciled with an
//! incoming document according to [`MergeOptions`]. There is no clock, I/O, or
//! global merge state in the core.

mod canonical;
pub mod causal;
mod core;
pub mod observability;
pub mod optimistic;
pub mod schema;

#[cfg(not(target_arch = "wasm32"))]
pub mod ffi;

#[cfg(feature = "wasm")]
mod wasm;

pub use crate::causal::{
    CAUSAL_ENVELOPE_JSON_SCHEMA, CAUSAL_ENVELOPE_SCHEMA_ID, CAUSAL_SCHEMA_VERSION,
    CausalDisposition, CausalEnvelope, CausalEnvelopeError, CausalOperation, MAX_CAUSAL_REPLICAS,
    VersionRelation, VersionVector, VersionVectorError,
};
pub use crate::core::{
    ArrayMergeStrategy, MergeError, MergeOptions, merge_json, merge_optional_json, merge_values,
};
pub use crate::observability::{
    MERGE_OBSERVATION_JSON_SCHEMA, MERGE_OBSERVATION_SCHEMA_ID, MERGE_OBSERVATION_SCHEMA_VERSION,
    MergeErrorCode, MergeObservation, MergeObservationSink, MergeOperation, MergeOutcome,
    merge_json_observed, merge_optional_json_observed,
};
pub use crate::optimistic::{
    OptimisticAck, OptimisticError, OptimisticWrite, acknowledge_resolved, receive_and_ack,
    record_delete, record_upsert, same_transaction_pair,
};
pub use crate::schema::{
    CanonicalMergeOptions, MERGE_OPTION_KEYS, MERGE_OPTIONS_JSON_SCHEMA, MERGE_OPTIONS_SCHEMA_ID,
    merge_json_with_schema_options, parse_merge_options_json,
};

/// Version shared by the Rust, C ABI, and WebAssembly surfaces.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
