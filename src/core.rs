use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

/// Controls how arrays are reconciled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum ArrayMergeStrategy {
    /// The incoming array replaces the base array.
    #[default]
    Replace = 0,
    /// Incoming elements are concatenated after base elements.
    Append = 1,
    /// Structurally new incoming elements are appended.
    Union = 2,
    /// Elements at the same index are reconciled; the longer tail is kept.
    MergeByIndex = 3,
    /// Object elements are matched by identity key and reconciled.
    MergeByKey = 4,
}

impl TryFrom<i32> for ArrayMergeStrategy {
    type Error = MergeError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Replace),
            1 => Ok(Self::Append),
            2 => Ok(Self::Union),
            3 => Ok(Self::MergeByIndex),
            4 => Ok(Self::MergeByKey),
            value => Err(MergeError::InvalidArrayStrategy(value)),
        }
    }
}

/// Options shared by every host-language surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MergeOptions {
    pub array_strategy: ArrayMergeStrategy,
    /// Maximum object nesting depth to reconcile. `0` means unlimited.
    pub max_depth: u32,
    /// Enables timestamp veto rules on object nodes.
    pub resolve_by_timestamp: bool,
    /// Compatibility option shared with pointer-based host engines.
    ///
    /// Rust JSON strings and `serde_json::Value` use owned acyclic trees, so
    /// this flag cannot change merge output. It is retained at every boundary
    /// so one cross-language options object does not silently lose a key.
    pub detect_circular_refs: bool,
    /// Comma-separated LWW selectors. `None` uses `"updatedAt"`.
    pub lww_keys: Option<String>,
    /// Comma-separated FWW selectors. `None` disables FWW.
    pub fww_keys: Option<String>,
    /// Comma-separated identity keys for `MergeByKey`. `None` uses `"id"`.
    pub array_match_keys: Option<String>,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            array_strategy: ArrayMergeStrategy::Replace,
            max_depth: 0,
            resolve_by_timestamp: false,
            detect_circular_refs: false,
            lww_keys: None,
            fww_keys: None,
            array_match_keys: None,
        }
    }
}

/// A malformed input or unsupported option.
#[derive(Debug)]
pub enum MergeError {
    InvalidBase(serde_json::Error),
    InvalidIncoming(serde_json::Error),
    Serialization(serde_json::Error),
    InvalidArrayStrategy(i32),
    InvalidOptions(String),
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase(error) => write!(f, "base is not valid JSON: {error}"),
            Self::InvalidIncoming(error) => write!(f, "incoming is not valid JSON: {error}"),
            Self::Serialization(error) => write!(f, "could not serialize merged JSON: {error}"),
            Self::InvalidArrayStrategy(value) => {
                write!(
                    f,
                    "array strategy {value} is outside the supported range 0..=4"
                )
            }
            Self::InvalidOptions(message) => write!(f, "invalid merge options: {message}"),
        }
    }
}

impl std::error::Error for MergeError {}

/// Reconciles two JSON strings and returns normalized, compact JSON.
pub fn merge_json(
    base: &str,
    incoming: &str,
    options: &MergeOptions,
) -> Result<String, MergeError> {
    merge_optional_json(Some(base), Some(incoming), options)?
        .ok_or_else(|| MergeError::InvalidOptions("both inputs were absent".to_owned()))
}

/// Reconciles optional JSON strings.
///
/// A one-sided merge validates and normalizes the side that is present.
/// `Ok(None)` is returned only when both sides are absent.
pub fn merge_optional_json(
    base: Option<&str>,
    incoming: Option<&str>,
    options: &MergeOptions,
) -> Result<Option<String>, MergeError> {
    let mut base_value = match base {
        Some(json) => Some(parse_json(json).map_err(MergeError::InvalidBase)?),
        None => None,
    };
    let incoming_value = match incoming {
        Some(json) => Some(parse_json(json).map_err(MergeError::InvalidIncoming)?),
        None => None,
    };

    let merged = match (&mut base_value, incoming_value.as_ref()) {
        (None, None) => return Ok(None),
        (Some(value), None) => value.clone(),
        (None, Some(value)) => value.clone(),
        (Some(base), Some(incoming)) => {
            merge_value(base, incoming, options, 0);
            base.clone()
        }
    };

    crate::canonical::to_canonical_string(&merged)
        .map(Some)
        .map_err(MergeError::Serialization)
}

/// Reconciles already-deserialized values without any host-language boundary.
pub fn merge_values(mut base: Value, incoming: &Value, options: &MergeOptions) -> Value {
    merge_value(&mut base, incoming, options, 0);
    base
}

fn parse_json(json: &str) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(json);
    deserializer.disable_recursion_limit();
    let value = Value::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

fn merge_value(base: &mut Value, incoming: &Value, options: &MergeOptions, depth: u32) {
    if options.resolve_by_timestamp && should_reject_by_timestamp(base, incoming, options) {
        return;
    }

    match (base, incoming) {
        (Value::Object(base_object), Value::Object(incoming_object)) => {
            merge_objects(base_object, incoming_object, options, depth);
        }
        (Value::Array(base_array), Value::Array(incoming_array)) => {
            merge_arrays(base_array, incoming_array, options, depth);
        }
        (base, incoming) => *base = incoming.clone(),
    }
}

fn merge_objects(
    base: &mut Map<String, Value>,
    incoming: &Map<String, Value>,
    options: &MergeOptions,
    depth: u32,
) {
    for (key, incoming_value) in incoming {
        let Some(base_value) = base.get_mut(key) else {
            base.insert(key.clone(), incoming_value.clone());
            continue;
        };

        let child_depth = depth.saturating_add(1);
        if options.max_depth != 0 && child_depth >= options.max_depth {
            *base_value = incoming_value.clone();
            continue;
        }

        merge_value(base_value, incoming_value, options, child_depth);
    }
}

fn merge_arrays(base: &mut Vec<Value>, incoming: &[Value], options: &MergeOptions, depth: u32) {
    match options.array_strategy {
        ArrayMergeStrategy::Replace => *base = incoming.to_vec(),
        ArrayMergeStrategy::Append => base.extend_from_slice(incoming),
        ArrayMergeStrategy::Union => {
            for value in incoming {
                if !base
                    .iter()
                    .any(|candidate| values_deep_equal(candidate, value))
                {
                    base.push(value.clone());
                }
            }
        }
        ArrayMergeStrategy::MergeByIndex => {
            for (index, incoming_value) in incoming.iter().enumerate() {
                let Some(base_value) = base.get_mut(index) else {
                    base.push(incoming_value.clone());
                    continue;
                };

                match (base_value, incoming_value) {
                    (Value::Object(base_object), Value::Object(incoming_object)) => {
                        let reject = options.resolve_by_timestamp
                            && should_reject_objects(base_object, incoming_object, options);
                        if !reject {
                            merge_objects(
                                base_object,
                                incoming_object,
                                options,
                                depth.saturating_add(1),
                            );
                        }
                    }
                    (base_value, incoming_value) => *base_value = incoming_value.clone(),
                }
            }
        }
        ArrayMergeStrategy::MergeByKey => {
            let match_keys = comma_separated(options.array_match_keys.as_deref().unwrap_or("id"));

            for incoming_value in incoming {
                let Some((identity_key, identity)) =
                    identity_of(incoming_value, match_keys.iter().copied())
                else {
                    if !base
                        .iter()
                        .any(|candidate| values_deep_equal(candidate, incoming_value))
                    {
                        base.push(incoming_value.clone());
                    }
                    continue;
                };

                let matched_index = base.iter().position(|candidate| {
                    candidate
                        .as_object()
                        .and_then(|object| object.get(identity_key))
                        .is_some_and(|candidate_identity| {
                            identity_values_equal(candidate_identity, identity)
                        })
                });

                let Some(index) = matched_index else {
                    base.push(incoming_value.clone());
                    continue;
                };

                let Some(base_object) = base[index].as_object_mut() else {
                    continue;
                };
                let incoming_object = incoming_value
                    .as_object()
                    .expect("an identity can only come from an object");

                let reject = options.resolve_by_timestamp
                    && should_reject_objects(base_object, incoming_object, options);
                if !reject {
                    merge_objects(
                        base_object,
                        incoming_object,
                        options,
                        depth.saturating_add(1),
                    );
                }
            }
        }
    }
}

fn comma_separated(value: &str) -> Vec<&str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn identity_of<'a>(
    value: &'a Value,
    keys: impl Iterator<Item = &'a str>,
) -> Option<(&'a str, &'a Value)> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(identity) = object.get(key) {
            return Some((key, identity));
        }
    }
    None
}

fn identity_values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right))
            if integer_value(left).is_some() && integer_value(right).is_some() =>
        {
            integer_value(left) == integer_value(right)
        }
        (Value::String(left), Value::String(right)) => left == right,
        (Value::String(left), Value::Number(right)) if integer_value(right).is_some() => {
            left == &integer_value(right).expect("checked above").to_string()
        }
        (Value::Number(left), Value::String(right)) if integer_value(left).is_some() => {
            &integer_value(left).expect("checked above").to_string() == right
        }
        _ => serde_json::to_string(left).ok() == serde_json::to_string(right).ok(),
    }
}

fn values_deep_equal(left: &Value, right: &Value) -> bool {
    let mut stack = vec![(left, right)];

    while let Some((left, right)) = stack.pop() {
        match (left, right) {
            (Value::Object(left), Value::Object(right)) => {
                if left.len() != right.len() {
                    return false;
                }
                for (key, right_value) in right {
                    let Some(left_value) = left.get(key) else {
                        return false;
                    };
                    stack.push((left_value, right_value));
                }
            }
            (Value::Array(left), Value::Array(right)) => {
                if left.len() != right.len() {
                    return false;
                }
                stack.extend(left.iter().zip(right));
            }
            (Value::Number(left), Value::Number(right)) => {
                if !numbers_equal(left, right) {
                    return false;
                }
            }
            _ if left == right => {}
            _ => return false,
        }
    }

    true
}

fn numbers_equal(left: &Number, right: &Number) -> bool {
    match (integer_value(left), integer_value(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left.as_f64() == right.as_f64(),
    }
}

fn integer_value(number: &Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

fn should_reject_by_timestamp(base: &Value, incoming: &Value, options: &MergeOptions) -> bool {
    let (Some(base), Some(incoming)) = (base.as_object(), incoming.as_object()) else {
        return false;
    };
    should_reject_objects(base, incoming, options)
}

fn should_reject_objects(
    base: &Map<String, Value>,
    incoming: &Map<String, Value>,
    options: &MergeOptions,
) -> bool {
    if let Some(fww_keys) = options.fww_keys.as_deref()
        && selectors_reject(base, incoming, fww_keys, true)
    {
        return true;
    }

    let lww_keys = options.lww_keys.as_deref().unwrap_or("updatedAt");
    selectors_reject(base, incoming, lww_keys, false)
}

fn selectors_reject(
    base: &Map<String, Value>,
    incoming: &Map<String, Value>,
    selectors: &str,
    first_write_wins: bool,
) -> bool {
    comma_separated(selectors).into_iter().any(|selector| {
        let base_timestamp = resolve_selector(base, selector);
        let incoming_timestamp = resolve_selector(incoming, selector);

        let Some(ordering) = base_timestamp
            .zip(incoming_timestamp)
            .and_then(|(base, incoming)| compare_timestamps(base, incoming))
        else {
            return false;
        };

        if first_write_wins {
            ordering == Ordering::Less
        } else {
            ordering == Ordering::Greater
        }
    })
}

fn resolve_selector<'a>(object: &'a Map<String, Value>, selector: &str) -> Option<&'a Value> {
    let Some(pointer) = selector.strip_prefix("#/") else {
        return object.get(selector);
    };

    let mut segments = pointer.split('/');
    let first = decode_pointer_segment(segments.next()?)?;
    let mut current = object.get(&first)?;

    for raw_segment in segments {
        let segment = decode_pointer_segment(raw_segment)?;
        current = match current {
            Value::Object(object) => object.get(&segment)?,
            Value::Array(array) => {
                if segment.len() > 1 && segment.starts_with('0') {
                    return None;
                }
                array.get(segment.parse::<usize>().ok()?)?
            }
            _ => return None,
        };
    }

    Some(current)
}

fn decode_pointer_segment(segment: &str) -> Option<String> {
    let mut decoded = String::with_capacity(segment.len());
    let mut characters = segment.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match characters.next()? {
            '0' => decoded.push('~'),
            '1' => decoded.push('/'),
            _ => return None,
        }
    }
    Some(decoded)
}

fn compare_timestamps(base: &Value, incoming: &Value) -> Option<Ordering> {
    match (base, incoming) {
        (Value::Number(base), Value::Number(incoming)) => {
            match (integer_value(base), integer_value(incoming)) {
                (Some(base), Some(incoming)) => Some(base.cmp(&incoming)),
                _ => base.as_f64()?.partial_cmp(&incoming.as_f64()?),
            }
        }
        (Value::String(base), Value::String(incoming)) => {
            Some(compare_timestamp_strings(base, incoming))
        }
        (Value::Number(base), Value::String(incoming)) if integer_value(base).is_some() => {
            Some(compare_timestamp_strings(
                &integer_value(base).expect("checked above").to_string(),
                incoming,
            ))
        }
        (Value::String(base), Value::Number(incoming)) if integer_value(incoming).is_some() => {
            Some(compare_timestamp_strings(
                base,
                &integer_value(incoming).expect("checked above").to_string(),
            ))
        }
        _ => None,
    }
}

fn compare_timestamp_strings(left: &str, right: &str) -> Ordering {
    if is_ascii_digits(left) && is_ascii_digits(right) {
        let left = left.trim_start_matches('0');
        let right = right.trim_start_matches('0');
        let left = if left.is_empty() { "0" } else { left };
        let right = if right.is_empty() { "0" } else { right };
        left.len().cmp(&right.len()).then_with(|| left.cmp(right))
    } else {
        left.cmp(right)
    }
}

fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timestamp_digit_strings_compare_numerically() {
        assert_eq!(compare_timestamp_strings("9", "10"), Ordering::Less);
        assert_eq!(compare_timestamp_strings("00010", "10"), Ordering::Equal);
    }

    #[test]
    fn structural_equality_ignores_object_key_order() {
        let left = parse_json(r#"{"id":"c","v":3}"#).unwrap();
        let right = parse_json(r#"{"v":3.0,"id":"c"}"#).unwrap();
        assert!(values_deep_equal(&left, &right));
    }

    #[test]
    fn json_pointer_timestamp_selector_works() {
        let options = MergeOptions {
            resolve_by_timestamp: true,
            lww_keys: Some("#/_sync/updatedAt".to_owned()),
            ..MergeOptions::default()
        };
        let base = json!({"doc": {"_sync": {"updatedAt": 20}, "value": "base"}});
        let incoming = json!({"doc": {"_sync": {"updatedAt": 10}, "value": "incoming"}});
        assert_eq!(merge_values(base.clone(), &incoming, &options), base);
    }

    #[test]
    fn json_pointer_decodes_slashes_tildes_and_array_indices() {
        let value = json!({"metadata": {"a/b": [{"~stamp": 42}]}});
        let object = value.as_object().unwrap();
        assert_eq!(
            resolve_selector(object, "#/metadata/a~1b/0/~0stamp"),
            Some(&json!(42))
        );
        assert_eq!(resolve_selector(object, "#/metadata/a~2b"), None);
        assert_eq!(resolve_selector(object, "#/metadata/a~1b/00"), None);
    }
}
