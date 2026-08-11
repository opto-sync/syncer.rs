use wasm_bindgen::prelude::*;

use crate::schema::{CanonicalMergeOptions, MERGE_OPTION_KEYS};
use crate::{MergeOptions, merge_json};

/// Reads the options argument, treating `undefined` and `null` as "defaults".
///
/// `CanonicalMergeOptions` is `#[serde(default)]`, so an absent option object is
/// meaningful; without this the idiomatic JavaScript calls
/// `mergeJsonWithOptions(base, incoming)` and `(base, incoming, undefined)`
/// both failed with a `invalid type: unit value` deserialization error.
fn read_options(options: JsValue) -> Result<MergeOptions, JsError> {
    let parsed: CanonicalMergeOptions = if options.is_undefined() || options.is_null() {
        CanonicalMergeOptions::default()
    } else {
        reject_unknown_keys(&options)?;
        serde_wasm_bindgen::from_value(options)
            .map_err(|error| JsError::new(&format!("invalid merge options: {error}")))?
    };
    MergeOptions::try_from(parsed).map_err(|error| JsError::new(&error.to_string()))
}

/// Fails on any own enumerable key that is not a documented merge option.
///
/// Non-object values are left alone so that `serde_wasm_bindgen` produces the
/// more precise "invalid type" diagnostic for them.
fn reject_unknown_keys(options: &JsValue) -> Result<(), JsError> {
    if js_sys::Array::is_array(options) {
        return Err(JsError::new(
            "invalid merge options: expected an object, got an array",
        ));
    }
    let Some(object) = options.dyn_ref::<js_sys::Object>() else {
        return Ok(());
    };
    for key in js_sys::Object::keys(object).iter() {
        let Some(key) = key.as_string() else {
            continue;
        };
        if !MERGE_OPTION_KEYS.contains(&key.as_str()) {
            return Err(JsError::new(&format!(
                "unknown merge option `{key}`; expected one of: {}. \
                 Merge options use camelCase, unlike the Rust and C ABI field names.",
                MERGE_OPTION_KEYS.join(", ")
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
/// These cover the value conversion and the field spelling accepted by the
/// generated `Deserialize` impl. They deliberately do **not** stand in for the
/// unknown-key rejection: that is enforced by [`reject_unknown_keys`] against
/// a live `JsValue` and is only observable in a real host, so it is covered by
/// the corpus in `tests/wasm/cases.mjs`, which runs under both Node
/// (`tests/wasm/run-node.mjs`) and Chromium (`tests/wasm/browser.spec.mjs`).
#[cfg(test)]
mod tests {
    use crate::schema::{CanonicalMergeOptions, MERGE_OPTION_KEYS};
    use crate::{ArrayMergeStrategy, MergeOptions};

    fn parse(json: &str) -> Result<MergeOptions, String> {
        let raw: CanonicalMergeOptions =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        MergeOptions::try_from(raw).map_err(|error| error.to_string())
    }

    #[test]
    fn an_empty_object_is_the_default_merge() {
        let options = parse("{}").expect("empty options are valid");
        assert_eq!(options.array_strategy, ArrayMergeStrategy::Replace);
        assert_eq!(options.max_depth, MergeOptions::default().max_depth);
        assert!(!options.resolve_by_timestamp);
        assert!(!options.detect_circular_refs);
        assert_eq!(options.lww_keys, None);
    }

    #[test]
    fn options_are_named_in_camel_case() {
        let options = parse(
            r#"{"arrayStrategy":4,"maxDepth":3,"resolveByTimestamp":true,"detectCircularRefs":true,
                "lwwKeys":"updatedAt","fwwKeys":"createdAt","arrayMatchKeys":"id"}"#,
        )
        .expect("camelCase options are valid");
        assert_eq!(options.array_strategy, ArrayMergeStrategy::MergeByKey);
        assert_eq!(options.max_depth, 3);
        assert!(options.resolve_by_timestamp);
        assert!(options.detect_circular_refs);
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
        assert!(parse(r#"{"detectCircularRefs":"yes"}"#).is_err());
    }

    /// The wasm boundary rejects unknown keys against [`MERGE_OPTION_KEYS`] rather
    /// than via `deny_unknown_fields`, so the list has to be kept in sync by
    /// hand. serde's own "expected one of ..." diagnostic is the source of
    /// truth for what the struct actually accepts.
    #[test]
    fn option_keys_match_the_deserialized_fields() {
        let error = serde_json::from_str::<CanonicalMergeOptions>(r#"{"__unknown__":1}"#)
            .expect_err("deny_unknown_fields must reject this")
            .to_string();

        for key in MERGE_OPTION_KEYS {
            assert!(
                error.contains(key),
                "`{key}` is in MERGE_OPTION_KEYS but is not a CanonicalMergeOptions field: {error}"
            );
        }
        assert_eq!(
            error.matches('`').count() / 2,
            MERGE_OPTION_KEYS.len() + 1, // the expected fields, plus `__unknown__`
            "CanonicalMergeOptions has a field missing from MERGE_OPTION_KEYS: {error}"
        );
    }
}
