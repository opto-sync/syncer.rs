use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use syncer_rs::{ArrayMergeStrategy, MergeOptions, merge_json, merge_values};

fn options(strategy: ArrayMergeStrategy) -> MergeOptions {
    MergeOptions {
        array_strategy: strategy,
        ..MergeOptions::default()
    }
}

#[test]
fn objects_deep_merge_and_incoming_scalars_win() {
    let merged = merge_json(
        r#"{"a":1,"nested":{"keep":true,"replace":"old"}}"#,
        r#"{"nested":{"replace":"new","add":2},"b":3}"#,
        &MergeOptions::default(),
    )
    .unwrap();

    assert_eq!(
        serde_json::from_str::<Value>(&merged).unwrap(),
        json!({
            "a": 1,
            "nested": {"keep": true, "replace": "new", "add": 2},
            "b": 3
        })
    );
}

#[test]
fn every_array_strategy_has_its_documented_behavior() {
    let base = r#"{"items":[{"id":1,"name":"one"},2,3]}"#;
    let incoming = r#"{"items":[{"id":1,"active":true},3,4]}"#;

    assert_eq!(
        merge_json(base, incoming, &options(ArrayMergeStrategy::Replace)).unwrap(),
        r#"{"items":[{"id":1,"active":true},3,4]}"#
    );
    assert_eq!(
        merge_json(base, incoming, &options(ArrayMergeStrategy::Append)).unwrap(),
        r#"{"items":[{"id":1,"name":"one"},2,3,{"id":1,"active":true},3,4]}"#
    );
    assert_eq!(
        merge_json(base, incoming, &options(ArrayMergeStrategy::Union)).unwrap(),
        r#"{"items":[{"id":1,"name":"one"},2,3,{"id":1,"active":true},4]}"#
    );
    assert_eq!(
        merge_json(base, incoming, &options(ArrayMergeStrategy::MergeByIndex)).unwrap(),
        r#"{"items":[{"id":1,"name":"one","active":true},3,4]}"#
    );
    assert_eq!(
        merge_json(base, incoming, &options(ArrayMergeStrategy::MergeByKey)).unwrap(),
        r#"{"items":[{"id":1,"name":"one","active":true},2,3,4]}"#
    );
}

#[test]
fn merge_by_key_matches_numeric_and_string_identifiers() {
    let options = MergeOptions {
        array_strategy: ArrayMergeStrategy::MergeByKey,
        array_match_keys: Some("uuid,id".to_owned()),
        ..MergeOptions::default()
    };
    let merged = merge_json(
        r#"{"rows":[{"uuid":"u1","id":1,"a":true},{"id":42,"left":1}]}"#,
        r#"{"rows":[{"uuid":"u1","id":999,"b":true},{"id":"42","right":2}]}"#,
        &options,
    )
    .unwrap();

    assert_eq!(
        merged,
        r#"{"rows":[{"uuid":"u1","id":999,"a":true,"b":true},{"id":"42","left":1,"right":2}]}"#
    );
}

#[test]
fn lww_and_fww_are_whole_node_vetoes() {
    let lww = MergeOptions {
        resolve_by_timestamp: true,
        lww_keys: Some("updatedAt".to_owned()),
        ..MergeOptions::default()
    };
    let base = json!({"doc": {"updatedAt": 200, "value": "base"}});
    let stale = json!({"doc": {"updatedAt": 100, "value": "stale", "added": true}});
    assert_eq!(merge_values(base.clone(), &stale, &lww), base);

    let fww = MergeOptions {
        resolve_by_timestamp: true,
        fww_keys: Some("createdAt".to_owned()),
        ..MergeOptions::default()
    };
    let base = json!({"doc": {"createdAt": 100, "updatedAt": 100, "value": "first"}});
    let recreated = json!({"doc": {"createdAt": 200, "updatedAt": 999, "value": "recreated"}});
    assert_eq!(merge_values(base.clone(), &recreated, &fww), base);
}

#[test]
fn max_depth_replaces_the_boundary_subtree() {
    let options = MergeOptions {
        max_depth: 2,
        ..MergeOptions::default()
    };
    let merged = merge_json(
        r#"{"a":{"b":{"base":true,"same":"old"}}}"#,
        r#"{"a":{"b":{"incoming":true,"same":"new"}}}"#,
        &options,
    )
    .unwrap();
    assert_eq!(merged, r#"{"a":{"b":{"incoming":true,"same":"new"}}}"#);
}

#[test]
fn invalid_json_is_an_error() {
    assert!(merge_json("{oops", "{}", &MergeOptions::default()).is_err());
    assert!(merge_json("{}", "[oops", &MergeOptions::default()).is_err());
}

#[test]
fn root_arrays_and_scalars_follow_the_same_rules() {
    assert_eq!(
        merge_json(
            r#"[{"left":true},2]"#,
            r#"[{"right":true},3,4]"#,
            &options(ArrayMergeStrategy::MergeByIndex),
        )
        .unwrap(),
        r#"[{"left":true,"right":true},3,4]"#
    );
    assert_eq!(
        merge_json("42", r#""incoming""#, &MergeOptions::default()).unwrap(),
        r#""incoming""#
    );
}

#[test]
fn timestamp_formats_keep_int64_precision() {
    let options = MergeOptions {
        resolve_by_timestamp: true,
        lww_keys: Some("updatedAt".to_owned()),
        ..MergeOptions::default()
    };
    let base = r#"{"doc":{"updatedAt":1689464777831256277,"value":"exact-int64"}}"#;
    let incoming = r#"{"doc":{"updatedAt":1689464777831256276,"value":"stale"}}"#;
    assert_eq!(merge_json(base, incoming, &options).unwrap(), base);

    let base = r#"{"doc":{"updatedAt":"9","value":"old"}}"#;
    let incoming = r#"{"doc":{"updatedAt":"10","value":"new"}}"#;
    assert_eq!(merge_json(base, incoming, &options).unwrap(), incoming);
}

#[test]
fn merge_by_key_without_identity_uses_union_semantics() {
    let options = options(ArrayMergeStrategy::MergeByKey);
    let once = merge_json(
        r#"{"items":[{"kind":"note","value":1}]}"#,
        r#"{"items":[{"value":1,"kind":"note"},{"kind":"other"}]}"#,
        &options,
    )
    .unwrap();
    let twice = merge_json(
        &once,
        r#"{"items":[{"value":1,"kind":"note"},{"kind":"other"}]}"#,
        &options,
    )
    .unwrap();
    assert_eq!(once, twice);
}

#[test]
fn circular_reference_detection_flag_is_inert_for_owned_json_trees() {
    let base = r#"{"nested":{"left":true},"items":[1,2]}"#;
    let incoming = r#"{"nested":{"right":true},"items":[2,3]}"#;
    let disabled = MergeOptions {
        array_strategy: ArrayMergeStrategy::Union,
        detect_circular_refs: false,
        ..MergeOptions::default()
    };
    let enabled = MergeOptions {
        detect_circular_refs: true,
        ..disabled.clone()
    };

    assert_eq!(
        merge_json(base, incoming, &disabled).unwrap(),
        merge_json(base, incoming, &enabled).unwrap()
    );
}
