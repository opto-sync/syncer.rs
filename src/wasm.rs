use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::{ArrayMergeStrategy, MergeOptions, merge_json};

/// Every accepted key on the JavaScript options object.
///
/// `deny_unknown_fields` alone is **not** sufficient here: `serde_wasm_bindgen`
/// resolves struct fields by direct property lookup, so unknown keys never
/// reach the generated visitor and the attribute silently has no effect at the
/// wasm boundary. The keys are therefore checked explicitly against this list.
/// It must stay in sync with the `WasmMergeOptions` fields below; the
/// `option_keys_match_the_deserialized_fields` test enforces that.
const OPTION_KEYS: [&str; 6] = [
    "arrayStrategy",
    "maxDepth",
    "resolveByTimestamp",
    "lwwKeys",
    "fwwKeys",
    "arrayMatchKeys",
];

/// The JavaScript-facing merge options object.
///
/// Field names are the camelCase forms of [`MergeOptions`]. Unknown keys are
/// rejected rather than ignored: this is a reconciliation core, and a silently
/// dropped option changes the merge *result* instead of failing. In particular
/// a caller porting from the Rust or C ABI naming (`array_strategy`) would
/// otherwise receive a `Replace` merge with no diagnostic at all.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct WasmMergeOptions {
    array_strategy: Option<i32>,
    max_depth: u32,
    resolve_by_timestamp: bool,
    lww_keys: Option<String>,
    fww_keys: Option<String>,
    array_match_keys: Option<String>,
}

impl TryFrom<WasmMergeOptions> for MergeOptions {
    /// Plain-text so the conversion is unit-testable off the wasm target;
    /// the exported functions wrap it in a `JsError` at the boundary.
    type Error = String;

    fn try_from(options: WasmMergeOptions) -> Result<Self, Self::Error> {
        let array_strategy = match options.array_strategy {
            Some(value) => ArrayMergeStrategy::try_from(value).map_err(|e| e.to_string())?,
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

/// Reads the options argument, treating `undefined` and `null` as "defaults".
///
/// `WasmMergeOptions` is `#[serde(default)]`, so an absent option object is
/// meaningful; without this the idiomatic JavaScript calls
/// `mergeJsonWithOptions(base, incoming)` and `(base, incoming, undefined)`
/// both failed with a `invalid type: unit value` deserialization error.
fn read_options(options: JsValue) -> Result<MergeOptions, JsError> {
    let parsed: WasmMergeOptions = if options.is_undefined() || options.is_null() {
        WasmMergeOptions::default()
    } else {
        reject_unknown_keys(&options)?;
        serde_wasm_bindgen::from_value(options)
            .map_err(|error| JsError::new(&format!("invalid merge options: {error}")))?
    };
    MergeOptions::try_from(parsed).map_err(|error| JsError::new(&error))
}

/// Fails on any own enumerable key that is not a documented merge option.
///
/// Non-object values are left alone so that `serde_wasm_bindgen` produces the
/// more precise "invalid type" diagnostic for them.
fn reject_unknown_keys(options: &JsValue) -> Result<(), JsError> {
    let Some(object) = options.dyn_ref::<js_sys::Object>() else {
        return Ok(());
    };
    for key in js_sys::Object::keys(object).iter() {
        let Some(key) = key.as_string() else {
            continue;
        };
        if !OPTION_KEYS.contains(&key.as_str()) {
            return Err(JsError::new(&format!(
                "unknown merge option `{key}`; expected one of: {}. \
                 Merge options use camelCase, unlike the Rust and C ABI field names.",
                OPTION_KEYS.join(", ")
            )));
        }
    }
    Ok(())
}

/// Default merge for Node and browser callers.
#[wasm_bindgen(js_name = mergeJson)]
pub fn merge_json_wasm(base: &str, incoming: &str) -> Result<String, JsError> {
    merge_json(base, incoming, &MergeOptions::default())
        .map_err(|error| JsError::new(&error.to_string()))
}

/// Configurable merge for Node and browser callers.
///
/// `options` may be omitted, `undefined`, or `null` to use the defaults.
#[wasm_bindgen(js_name = mergeJsonWithOptions)]
pub fn merge_json_with_options_wasm(
    base: &str,
    incoming: &str,
    options: JsValue,
) -> Result<String, JsError> {
    let options = read_options(options)?;
    merge_json(base, incoming, &options).map_err(|error| JsError::new(&error.to_string()))
}

/// The option-name and option-value contract, exercised without a JS runtime.
///
/// `serde_wasm_bindgen` and `serde_json` drive the same generated `Deserialize`
/// impl, so the accepted key spelling and the unknown-key rejection are the
/// same facts here as in a browser. The end-to-end browser behavior is covered
/// by `tests/browser/` and the Node harness in `tests/wasm_node.mjs`.
#[cfg(test)]
mod tests {
    use super::WasmMergeOptions;
    use crate::{ArrayMergeStrategy, MergeOptions};

    fn parse(json: &str) -> Result<MergeOptions, String> {
        let raw: WasmMergeOptions =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        MergeOptions::try_from(raw)
    }

    #[test]
    fn an_empty_object_is_the_default_merge() {
        let options = parse("{}").expect("empty options are valid");
        assert_eq!(options.array_strategy, ArrayMergeStrategy::Replace);
        assert_eq!(options.max_depth, MergeOptions::default().max_depth);
        assert!(!options.resolve_by_timestamp);
        assert_eq!(options.lww_keys, None);
    }

    #[test]
    fn options_are_named_in_camel_case() {
        let options = parse(
            r#"{"arrayStrategy":4,"maxDepth":3,"resolveByTimestamp":true,
                "lwwKeys":"updatedAt","fwwKeys":"createdAt","arrayMatchKeys":"id"}"#,
        )
        .expect("camelCase options are valid");
        assert_eq!(options.array_strategy, ArrayMergeStrategy::MergeByKey);
        assert_eq!(options.max_depth, 3);
        assert!(options.resolve_by_timestamp);
        assert_eq!(options.lww_keys.as_deref(), Some("updatedAt"));
        assert_eq!(options.fww_keys.as_deref(), Some("createdAt"));
        assert_eq!(options.array_match_keys.as_deref(), Some("id"));
    }

    /// Regression: this previously parsed, dropped the key, and returned a
    /// `Replace` merge — a wrong result rather than an error.
    #[test]
    fn the_rust_and_c_snake_case_spelling_is_rejected_not_ignored() {
        let error = parse(r#"{"array_strategy":1}"#).expect_err("snake_case must not be accepted");
        assert!(
            error.contains("array_strategy"),
            "error should name the offending key, got: {error}"
        );
    }

    #[test]
    fn a_misspelled_option_is_rejected() {
        assert!(parse(r#"{"arrayStrategyy":1}"#).is_err());
        assert!(parse(r#"{"lww_keys":"updatedAt"}"#).is_err());
        assert!(parse(r#"{"bogus":true}"#).is_err());
    }

    #[test]
    fn every_documented_strategy_value_converts() {
        let expected = [
            ArrayMergeStrategy::Replace,
            ArrayMergeStrategy::Append,
            ArrayMergeStrategy::Union,
            ArrayMergeStrategy::MergeByIndex,
            ArrayMergeStrategy::MergeByKey,
        ];
        for (value, strategy) in expected.into_iter().enumerate() {
            let options = parse(&format!(r#"{{"arrayStrategy":{value}}}"#))
                .unwrap_or_else(|error| panic!("strategy {value} should convert: {error}"));
            assert_eq!(options.array_strategy, strategy);
        }
    }

    #[test]
    fn out_of_range_strategy_values_are_rejected() {
        for value in ["-1", "5", "99"] {
            let error = parse(&format!(r#"{{"arrayStrategy":{value}}}"#))
                .expect_err("out-of-range strategy must fail");
            assert!(
                error.contains("outside the supported range"),
                "unexpected error for {value}: {error}"
            );
        }
    }

    #[test]
    fn a_wrongly_typed_strategy_is_rejected() {
        assert!(parse(r#"{"arrayStrategy":"1"}"#).is_err());
        assert!(parse(r#"{"resolveByTimestamp":"yes"}"#).is_err());
    }

    /// The wasm boundary rejects unknown keys against [`OPTION_KEYS`] rather
    /// than via `deny_unknown_fields`, so the list has to be kept in sync by
    /// hand. serde's own "expected one of ..." diagnostic is the source of
    /// truth for what the struct actually accepts.
    #[test]
    fn option_keys_match_the_deserialized_fields() {
        let error = serde_json::from_str::<WasmMergeOptions>(r#"{"__unknown__":1}"#)
            .expect_err("deny_unknown_fields must reject this")
            .to_string();

        for key in super::OPTION_KEYS {
            assert!(
                error.contains(key),
                "`{key}` is in OPTION_KEYS but is not a WasmMergeOptions field: {error}"
            );
        }
        assert_eq!(
            error.matches('`').count() / 2,
            super::OPTION_KEYS.len() + 1, // the expected fields, plus `__unknown__`
            "WasmMergeOptions has a field missing from OPTION_KEYS: {error}"
        );
    }
}
