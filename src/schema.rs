//! Canonical JSON-Schema contracts for the language-neutral engine boundary.
//!
//! Rust callers normally construct [`MergeOptions`](crate::MergeOptions)
//! directly. JSON, Dart, TypeScript, WebAssembly, and service boundaries should
//! use [`parse_merge_options_json`] so the same camelCase keys, integer strategy
//! codes, defaults, and unknown-field rejection are enforced everywhere.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{ArrayMergeStrategy, MergeError, MergeOptions, merge_json};

/// Stable identifier of the canonical merge-options schema.
pub const MERGE_OPTIONS_SCHEMA_ID: &str = "https://opto-sync.dev/schema/merge-options.schema.json";

/// Draft 2020-12 JSON Schema shipped with every crate and Zed package.
pub const MERGE_OPTIONS_JSON_SCHEMA: &str = include_str!("../schema/merge-options.schema.json");

/// Canonical option keys shared by the JSON and WebAssembly boundaries.
pub const MERGE_OPTION_KEYS: [&str; 7] = [
    "arrayStrategy",
    "maxDepth",
    "resolveByTimestamp",
    "detectCircularRefs",
    "lwwKeys",
    "fwwKeys",
    "arrayMatchKeys",
];

/// JSON-Schema representation of [`MergeOptions`].
///
/// The normal Rust structure retains its idiomatic strongly typed enum. This
/// boundary structure deliberately uses the cross-language integer strategy
/// code and camelCase field names. Unknown keys are rejected because silently
/// dropping a misspelled option changes the merged document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct CanonicalMergeOptions {
    /// `0=replace`, `1=append`, `2=union`, `3=merge-by-index`,
    /// `4=merge-by-key`; omission selects `0`.
    #[serde(deserialize_with = "deserialize_i32_integer")]
    pub array_strategy: i32,
    /// Maximum object nesting depth; zero means unlimited.
    #[serde(deserialize_with = "deserialize_u32_integer")]
    pub max_depth: u32,
    /// Whether timestamp selectors may veto an incoming object.
    pub resolve_by_timestamp: bool,
    /// Cross-engine compatibility flag; inert for owned Rust JSON trees.
    pub detect_circular_refs: bool,
    /// Comma-separated last-write-wins selectors.
    pub lww_keys: Option<String>,
    /// Comma-separated first-write-wins selectors.
    pub fww_keys: Option<String>,
    /// Comma-separated identity selectors for merge-by-key arrays.
    pub array_match_keys: Option<String>,
}

impl TryFrom<CanonicalMergeOptions> for MergeOptions {
    type Error = MergeError;

    fn try_from(options: CanonicalMergeOptions) -> Result<Self, Self::Error> {
        let array_strategy = ArrayMergeStrategy::try_from(options.array_strategy)?;
        Ok(Self {
            array_strategy,
            max_depth: options.max_depth,
            resolve_by_timestamp: options.resolve_by_timestamp,
            detect_circular_refs: options.detect_circular_refs,
            lww_keys: options.lww_keys,
            fww_keys: options.fww_keys,
            array_match_keys: options.array_match_keys,
        })
    }
}

impl From<&MergeOptions> for CanonicalMergeOptions {
    fn from(options: &MergeOptions) -> Self {
        Self {
            array_strategy: options.array_strategy as i32,
            max_depth: options.max_depth,
            resolve_by_timestamp: options.resolve_by_timestamp,
            detect_circular_refs: options.detect_circular_refs,
            lww_keys: options.lww_keys.clone(),
            fww_keys: options.fww_keys.clone(),
            array_match_keys: options.array_match_keys.clone(),
        }
    }
}

/// Parses and validates one JSON object against the canonical merge-options
/// contract.
///
/// This typed validator implements the schema's complete value contract while
/// avoiding a second general-purpose schema engine in native and WebAssembly
/// artifacts. The embedded schema remains available for external validators,
/// code generators, and other languages.
pub fn parse_merge_options_json(options_json: &str) -> Result<MergeOptions, MergeError> {
    let value = serde_json::from_str::<serde_json::Value>(options_json).map_err(|error| {
        MergeError::InvalidOptions(format!(
            "value does not conform to {MERGE_OPTIONS_SCHEMA_ID}: {error}"
        ))
    })?;
    if !value.is_object() {
        return Err(MergeError::InvalidOptions(format!(
            "value does not conform to {MERGE_OPTIONS_SCHEMA_ID}: expected an object"
        )));
    }
    let options = serde_json::from_value::<CanonicalMergeOptions>(value).map_err(|error| {
        MergeError::InvalidOptions(format!(
            "value does not conform to {MERGE_OPTIONS_SCHEMA_ID}: {error}"
        ))
    })?;
    options.try_into()
}

/// Reconciles two documents using a schema-validated JSON options object.
pub fn merge_json_with_schema_options(
    base: &str,
    incoming: &str,
    options_json: &str,
) -> Result<String, MergeError> {
    let options = parse_merge_options_json(options_json)?;
    merge_json(base, incoming, &options)
}

fn deserialize_i32_integer<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    struct IntegerVisitor;

    impl<'de> Visitor<'de> for IntegerVisitor {
        type Value = i32;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON integer in the signed 32-bit range")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            i32::try_from(value)
                .map_err(|_| E::custom("integer is outside the signed 32-bit range"))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            i32::try_from(value)
                .map_err(|_| E::custom("integer is outside the signed 32-bit range"))
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_finite()
                && value.fract() == 0.0
                && (i32::MIN as f64..=i32::MAX as f64).contains(&value)
            {
                Ok(value as i32)
            } else {
                Err(E::custom("number is not a signed 32-bit JSON integer"))
            }
        }
    }

    deserializer.deserialize_any(IntegerVisitor)
}

fn deserialize_u32_integer<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    struct IntegerVisitor;

    impl<'de> Visitor<'de> for IntegerVisitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a non-negative JSON integer in the unsigned 32-bit range")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(value)
                .map_err(|_| E::custom("integer is outside the unsigned 32-bit range"))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            u32::try_from(value)
                .map_err(|_| E::custom("integer is outside the unsigned 32-bit range"))
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_finite() && value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&value)
            {
                Ok(value as u32)
            } else {
                Err(E::custom("number is not an unsigned 32-bit JSON integer"))
            }
        }
    }

    deserializer.deserialize_any(IntegerVisitor)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn embedded_schema_is_the_canonical_draft_2020_12_contract() {
        let schema: Value = serde_json::from_str(MERGE_OPTIONS_JSON_SCHEMA)
            .expect("embedded merge-options schema must be valid JSON");
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(schema["$id"], MERGE_OPTIONS_SCHEMA_ID);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["arrayStrategy"]["minimum"], 0);
        assert_eq!(schema["properties"]["arrayStrategy"]["maximum"], 4);
        assert_eq!(schema["properties"]["maxDepth"]["maximum"], u32::MAX as u64);

        let schema_keys = schema["properties"]
            .as_object()
            .expect("schema properties must be an object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            schema_keys,
            MERGE_OPTION_KEYS.into_iter().collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn empty_and_complete_objects_follow_cross_language_defaults() {
        assert_eq!(
            parse_merge_options_json("{}").expect("empty options are valid"),
            MergeOptions::default()
        );

        let options = parse_merge_options_json(
            r#"{"arrayStrategy":4,"maxDepth":3,"resolveByTimestamp":true,"detectCircularRefs":true,
                "lwwKeys":"updatedAt","fwwKeys":"createdAt","arrayMatchKeys":"uuid,id"}"#,
        )
        .expect("complete canonical options are valid");
        assert_eq!(options.array_strategy, ArrayMergeStrategy::MergeByKey);
        assert_eq!(options.max_depth, 3);
        assert!(options.resolve_by_timestamp);
        assert!(options.detect_circular_refs);
        assert_eq!(options.lww_keys.as_deref(), Some("updatedAt"));
        assert_eq!(options.fww_keys.as_deref(), Some("createdAt"));
        assert_eq!(options.array_match_keys.as_deref(), Some("uuid,id"));

        let mathematical_integers =
            parse_merge_options_json(r#"{"arrayStrategy":4.0,"maxDepth":3e0}"#)
                .expect("JSON Schema integers may use a zero fractional or exponent form");
        assert_eq!(
            mathematical_integers.array_strategy,
            ArrayMergeStrategy::MergeByKey
        );
        assert_eq!(mathematical_integers.max_depth, 3);
    }

    #[test]
    fn validator_rejects_unknown_keys_types_ranges_and_non_objects() {
        for invalid in [
            r#"{"array_strategy":1}"#,
            r#"{"arrayStrategyy":1}"#,
            r#"{"arrayStrategy":-1}"#,
            r#"{"arrayStrategy":5}"#,
            r#"{"arrayStrategy":"1"}"#,
            r#"{"arrayStrategy":1.5}"#,
            r#"{"maxDepth":4294967296}"#,
            r#"{"maxDepth":1.5}"#,
            r#"{"resolveByTimestamp":"yes"}"#,
            r#"{"detectCircularRefs":"yes"}"#,
            "null",
            "[]",
        ] {
            assert!(
                parse_merge_options_json(invalid).is_err(),
                "invalid options unexpectedly passed: {invalid}"
            );
        }
    }

    #[test]
    fn canonical_options_serialize_with_integer_strategy_codes() {
        let defaults = serde_json::to_value(CanonicalMergeOptions::default())
            .expect("default canonical options must serialize");
        assert_eq!(defaults["arrayStrategy"], 0);
        assert_eq!(defaults["detectCircularRefs"], false);

        let options = MergeOptions {
            array_strategy: ArrayMergeStrategy::MergeByIndex,
            max_depth: 2,
            resolve_by_timestamp: true,
            detect_circular_refs: true,
            lww_keys: Some("updatedAt".to_owned()),
            fww_keys: None,
            array_match_keys: Some("id".to_owned()),
        };
        let wire = serde_json::to_value(CanonicalMergeOptions::from(&options))
            .expect("canonical options must serialize");
        assert_eq!(wire["arrayStrategy"], 3);
        assert_eq!(wire["maxDepth"], 2);
        assert_eq!(wire["resolveByTimestamp"], true);
        assert_eq!(wire["detectCircularRefs"], true);
        assert_eq!(wire["fwwKeys"], Value::Null);
    }

    #[test]
    fn schema_options_drive_the_normal_merge_core() {
        let merged = merge_json_with_schema_options(
            r#"{"items":[{"id":1,"left":true}]}"#,
            r#"{"items":[{"id":1,"right":true}]}"#,
            r#"{"arrayStrategy":4}"#,
        )
        .expect("schema-validated merge must succeed");
        assert_eq!(
            serde_json::from_str::<Value>(&merged).expect("merge output must be JSON"),
            json!({"items": [{"id": 1, "left": true, "right": true}]})
        );
    }
}
