//! Differential runner for the syncer.c JSONL corpus.
//!
//! cargo run --release --example jsonl_runner -- corpus.jsonl results-rust-native.jsonl

use std::env;
use std::fs;
use std::process::ExitCode;

use syncer_rs::{ArrayMergeStrategy, MergeOptions, merge_json};

const PREFIX: &str = r#"{"base":"#;
const MARKER: &str = r#","incoming":"#;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let input = arguments
        .next()
        .ok_or_else(|| "usage: jsonl_runner <input.jsonl> <output.jsonl>".to_owned())?;
    let output = arguments
        .next()
        .ok_or_else(|| "usage: jsonl_runner <input.jsonl> <output.jsonl>".to_owned())?;

    let options = MergeOptions {
        array_strategy: ArrayMergeStrategy::MergeByKey,
        max_depth: 0,
        resolve_by_timestamp: true,
        lww_keys: Some("updatedAt,syncedAt,#/_sync/updatedAt".to_owned()),
        fww_keys: Some("createdAt".to_owned()),
        array_match_keys: Some("id".to_owned()),
    };

    let corpus = fs::read_to_string(&input).map_err(|error| format!("{input}: {error}"))?;
    let mut results = String::new();

    for (index, line) in corpus.lines().enumerate() {
        let marker = line
            .find(MARKER)
            .ok_or_else(|| format!("line {} has no incoming marker", index + 1))?;
        if !line.starts_with(PREFIX) || !line.ends_with('}') {
            return Err(format!("line {} is malformed", index + 1));
        }
        let base = &line[PREFIX.len()..marker];
        let incoming = &line[marker + MARKER.len()..line.len() - 1];
        let merged = merge_json(base, incoming, &options)
            .map_err(|error| format!("line {}: {error}", index + 1))?;
        results.push_str(&merged);
        results.push('\n');
    }

    fs::write(&output, results).map_err(|error| format!("{output}: {error}"))
}
