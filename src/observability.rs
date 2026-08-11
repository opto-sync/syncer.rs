//! Injection-based structured observations for reconciliation attempts.
//!
//! The merge core never installs a global logger or OpenTelemetry provider.
//! Applications inject a [`MergeObservationSink`] and adapt the payload to the
//! application-owned Ores/OpenTelemetry adapter. Request/trace context remains
//! application-owned, and a logging failure cannot change a merge result.

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde::Serialize;

use crate::{MergeError, MergeOptions, merge_json, merge_optional_json};

/// Wire discriminator for structured merge observations.
pub const MERGE_OBSERVATION_SCHEMA_VERSION: &str = "opto-sync.merge-observation.v1";

/// Stable identifier of the canonical observation schema.
pub const MERGE_OBSERVATION_SCHEMA_ID: &str =
    "https://opto-sync.dev/schema/merge-observation.schema.json";

/// Draft 2020-12 JSON Schema shipped with every crate and Zed package.
pub const MERGE_OBSERVATION_JSON_SCHEMA: &str =
    include_str!("../schema/merge-observation.schema.json");

/// Public merge entry point that produced an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeOperation {
    /// [`merge_json`] with two required documents.
    MergeJson,
    /// [`merge_optional_json`] with zero, one, or two documents.
    MergeOptionalJson,
}

/// Result class for a reconciliation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeOutcome {
    /// The engine returned a valid result.
    Succeeded,
    /// The engine rejected input or options.
    Rejected,
}

/// Stable, language-neutral error code suitable for logs and metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeErrorCode {
    InvalidBaseJson,
    InvalidIncomingJson,
    SerializationFailed,
    InvalidArrayStrategy,
    InvalidOptions,
}

impl From<&MergeError> for MergeErrorCode {
    fn from(error: &MergeError) -> Self {
        match error {
            MergeError::InvalidBase(_) => Self::InvalidBaseJson,
            MergeError::InvalidIncoming(_) => Self::InvalidIncomingJson,
            MergeError::Serialization(_) => Self::SerializationFailed,
            MergeError::InvalidArrayStrategy(_) => Self::InvalidArrayStrategy,
            MergeError::InvalidOptions(_) => Self::InvalidOptions,
        }
    }
}

/// Payload-safe structured event for Ores logging and OTEL adapters.
///
/// Document bodies, timestamp-selector values, identities, and request context
/// are intentionally absent. An Ores logger applies its application-owned
/// `RequestContext`/`LogContext` when the sink records this event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeObservation {
    pub schema_version: &'static str,
    pub operation: MergeOperation,
    pub outcome: MergeOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<MergeErrorCode>,
    pub array_strategy: i32,
    pub max_depth: u32,
    pub resolve_by_timestamp: bool,
    pub detect_circular_refs: bool,
    pub base_present: bool,
    pub incoming_present: bool,
}

/// Application-owned adapter for structured logging.
///
/// Implementations should forward the serialized event to
/// an Ores-compatible sink and let the application attach its current shared
/// request/trace context. Panics are isolated so observability cannot alter the
/// result of a deterministic merge.
pub trait MergeObservationSink {
    fn record(&self, observation: &MergeObservation);
}

impl<F> MergeObservationSink for F
where
    F: Fn(&MergeObservation),
{
    fn record(&self, observation: &MergeObservation) {
        self(observation);
    }
}

/// Reconciles required documents and records one payload-safe observation.
pub fn merge_json_observed<S>(
    base: &str,
    incoming: &str,
    options: &MergeOptions,
    sink: &S,
) -> Result<String, MergeError>
where
    S: MergeObservationSink + ?Sized,
{
    let result = merge_json(base, incoming, options);
    record_result(
        sink,
        MergeOperation::MergeJson,
        true,
        true,
        options,
        &result,
    );
    result
}

/// Reconciles optional documents and records one payload-safe observation.
pub fn merge_optional_json_observed<S>(
    base: Option<&str>,
    incoming: Option<&str>,
    options: &MergeOptions,
    sink: &S,
) -> Result<Option<String>, MergeError>
where
    S: MergeObservationSink + ?Sized,
{
    let result = merge_optional_json(base, incoming, options);
    record_result(
        sink,
        MergeOperation::MergeOptionalJson,
        base.is_some(),
        incoming.is_some(),
        options,
        &result,
    );
    result
}

fn record_result<T, S>(
    sink: &S,
    operation: MergeOperation,
    base_present: bool,
    incoming_present: bool,
    options: &MergeOptions,
    result: &Result<T, MergeError>,
) where
    S: MergeObservationSink + ?Sized,
{
    let (outcome, error_code) = match result {
        Ok(_) => (MergeOutcome::Succeeded, None),
        Err(error) => (MergeOutcome::Rejected, Some(MergeErrorCode::from(error))),
    };
    let observation = MergeObservation {
        schema_version: MERGE_OBSERVATION_SCHEMA_VERSION,
        operation,
        outcome,
        error_code,
        array_strategy: options.array_strategy as i32,
        max_depth: options.max_depth,
        resolve_by_timestamp: options.resolve_by_timestamp,
        detect_circular_refs: options.detect_circular_refs,
        base_present,
        incoming_present,
    };

    // A logger is diagnostic infrastructure, not part of reconciliation. Even
    // a faulty injected adapter must not convert a valid merge into a failure.
    let _ = catch_unwind(AssertUnwindSafe(|| sink.record(&observation)));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::{Value, json};

    use super::*;
    use crate::ArrayMergeStrategy;

    #[test]
    fn successful_observation_is_schema_shaped_and_payload_safe() {
        let recorded = RefCell::new(None);
        let options = MergeOptions {
            array_strategy: ArrayMergeStrategy::MergeByKey,
            resolve_by_timestamp: true,
            detect_circular_refs: true,
            lww_keys: Some("privateUpdatedAt".to_owned()),
            ..MergeOptions::default()
        };
        let merged = merge_json_observed(
            r#"{"secret":"base","items":[]}"#,
            r#"{"secret":"incoming","items":[]}"#,
            &options,
            &|observation: &MergeObservation| {
                recorded.replace(Some(observation.clone()));
            },
        )
        .expect("merge should succeed");
        assert!(merged.contains("incoming"));

        let observation = recorded
            .into_inner()
            .expect("sink must receive one observation");
        assert_eq!(observation.outcome, MergeOutcome::Succeeded);
        assert_eq!(observation.error_code, None);
        assert_eq!(observation.array_strategy, 4);
        assert!(observation.resolve_by_timestamp);
        assert!(observation.detect_circular_refs);

        let encoded = serde_json::to_string(&observation)
            .expect("observation must serialize for structured logging");
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("privateUpdatedAt"));
    }

    #[test]
    fn rejected_input_has_a_stable_error_code() {
        let recorded = RefCell::new(None);
        assert!(
            merge_json_observed(
                "{bad",
                "{}",
                &MergeOptions::default(),
                &|observation: &MergeObservation| {
                    recorded.replace(Some(observation.clone()));
                },
            )
            .is_err()
        );
        let observation = recorded.into_inner().expect("rejection must be observed");
        assert_eq!(observation.outcome, MergeOutcome::Rejected);
        assert_eq!(
            observation.error_code,
            Some(MergeErrorCode::InvalidBaseJson)
        );
    }

    #[test]
    fn a_panicking_sink_cannot_change_the_merge_result() {
        let merged = merge_json_observed(
            r#"{"left":true}"#,
            r#"{"right":true}"#,
            &MergeOptions::default(),
            &|_: &MergeObservation| panic!("broken logger"),
        )
        .expect("logging failure must be isolated");
        assert_eq!(merged, r#"{"left":true,"right":true}"#);
    }

    #[test]
    fn embedded_observation_schema_matches_serialized_variants() {
        let schema: Value = serde_json::from_str(MERGE_OBSERVATION_JSON_SCHEMA)
            .expect("embedded observation schema must be valid JSON");
        assert_eq!(schema["$id"], MERGE_OBSERVATION_SCHEMA_ID);
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["errorCode"]["enum"],
            json!([
                "invalidBaseJson",
                "invalidIncomingJson",
                "serializationFailed",
                "invalidArrayStrategy",
                "invalidOptions"
            ])
        );

        let rejected = MergeObservation {
            schema_version: MERGE_OBSERVATION_SCHEMA_VERSION,
            operation: MergeOperation::MergeOptionalJson,
            outcome: MergeOutcome::Rejected,
            error_code: Some(MergeErrorCode::InvalidOptions),
            array_strategy: 0,
            max_depth: 0,
            resolve_by_timestamp: false,
            detect_circular_refs: false,
            base_present: false,
            incoming_present: false,
        };
        assert_eq!(
            serde_json::to_value(rejected).expect("observation must serialize"),
            json!({
                "schemaVersion": "opto-sync.merge-observation.v1",
                "operation": "mergeOptionalJson",
                "outcome": "rejected",
                "errorCode": "invalidOptions",
                "arrayStrategy": 0,
                "maxDepth": 0,
                "resolveByTimestamp": false,
                "detectCircularRefs": false,
                "basePresent": false,
                "incomingPresent": false
            })
        );
    }
}
