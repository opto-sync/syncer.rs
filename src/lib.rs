//! Rust-native JSON reconciliation for native callers, C-compatible FFI, and
//! WebAssembly.
//!
//! The merge is a pure function: the base document is reconciled with an
//! incoming document according to [`MergeOptions`]. There is no clock, I/O, or
//! global merge state in the core.

mod canonical;
pub mod causal;
mod core;

#[cfg(not(target_arch = "wasm32"))]
pub mod ffi;

#[cfg(feature = "wasm")]
mod wasm;

pub use crate::causal::{
    CAUSAL_SCHEMA_VERSION, CausalDisposition, CausalEnvelope, CausalEnvelopeError,
    CausalOperation, MAX_CAUSAL_REPLICAS, VersionRelation, VersionVector,
    VersionVectorError,
};
pub use crate::core::{
    ArrayMergeStrategy, MergeError, MergeOptions, merge_json, merge_optional_json, merge_values,
};

/// Version shared by the Rust, C ABI, and WebAssembly surfaces.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
