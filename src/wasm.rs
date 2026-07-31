use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::{ArrayMergeStrategy, MergeOptions, merge_json};

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct WasmMergeOptions {
    array_strategy: Option<i32>,
    max_depth: u32,
    resolve_by_timestamp: bool,
    lww_keys: Option<String>,
    fww_keys: Option<String>,
    array_match_keys: Option<String>,
}

impl TryFrom<WasmMergeOptions> for MergeOptions {
    type Error = JsError;

    fn try_from(options: WasmMergeOptions) -> Result<Self, Self::Error> {
        let array_strategy = match options.array_strategy {
            Some(value) => ArrayMergeStrategy::try_from(value)
                .map_err(|error| JsError::new(&error.to_string()))?,
            None => ArrayMergeStrategy::Replace,
        };
        Ok(Self {
            array_strategy,
            max_depth: options.max_depth,
            resolve_by_timestamp: options.resolve_by_timestamp,
            lww_keys: options.lww_keys,
            fww_keys: options.fww_keys,
            array_match_keys: options.array_match_keys,
        })
    }
}

/// Default merge for Node and browser callers.
#[wasm_bindgen(js_name = mergeJson)]
pub fn merge_json_wasm(base: &str, incoming: &str) -> Result<String, JsError> {
    merge_json(base, incoming, &MergeOptions::default())
        .map_err(|error| JsError::new(&error.to_string()))
}

/// Configurable merge for Node and browser callers.
#[wasm_bindgen(js_name = mergeJsonWithOptions)]
pub fn merge_json_with_options_wasm(
    base: &str,
    incoming: &str,
    options: JsValue,
) -> Result<String, JsError> {
    let options: WasmMergeOptions = serde_wasm_bindgen::from_value(options)
        .map_err(|error| JsError::new(&format!("invalid merge options: {error}")))?;
    let options = MergeOptions::try_from(options)?;
    merge_json(base, incoming, &options).map_err(|error| JsError::new(&error.to_string()))
}
