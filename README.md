# syncer.rs

One Rust-native JSON reconciliation core with three host surfaces:

- normal Rust library calls for backends and native applications;
- a stable C ABI for Flutter/Dart FFI and other native languages;
- WebAssembly exports for Node and browsers.

`syncer.rs` complements `syncer.c`; it does not replace it. The C repository
can continue to target the broadest language and runtime set, while this
repository gives Rust-based systems one memory-safe core with no C build step.

## Merge behavior

The base document is deep-merged with the incoming document. Incoming scalars
win. Arrays support replace, append, structural union, merge-by-index, and
identity-keyed reconciliation. Optional LWW/FWW timestamp selectors can veto an
incoming object node as a whole.

The strategy values are a cross-language contract:

| Value | Strategy |
|---:|---|
| 0 | replace |
| 1 | append |
| 2 | union |
| 3 | merge by index |
| 4 | merge by key |

`MERGE_BY_KEY` uses `id` by default. Numeric `42` and string `"42"` identify
the same record. Timestamp selectors may be direct keys such as `updatedAt` or
relative RFC 6901 pointers such as `#/_sync/updatedAt`.

A practical sync policy is merge-by-key with timestamp resolution enabled,
`lwwKeys` set to `updatedAt,syncedAt`, and no FWW selector. FWW is a whole-node
veto: if the incoming FWW timestamp is later, the entire incoming object loses.
It does not merely protect the timestamp field, so use it only when the first
writer should own that complete node.

## Rust

```rust
use syncer_rs::{ArrayMergeStrategy, MergeOptions, merge_json};

let options = MergeOptions {
    array_strategy: ArrayMergeStrategy::MergeByKey,
    resolve_by_timestamp: true,
    lww_keys: Some("updatedAt,syncedAt".into()),
    array_match_keys: Some("id".into()),
    ..MergeOptions::default()
};

let merged = merge_json(base_json, incoming_json, &options)?;
```

Call `merge_values` when the backend already has `serde_json::Value` instances.

## Flutter/Dart FFI

Build the dynamic library:

```sh
cargo build --release
```

The public header is `include/syncer_rs.h`. Flutter bindings call
`syncer_rs_merge_json` or `syncer_rs_merge_json_ex`, convert the returned UTF-8
string, and always release it with `syncer_rs_free`. A null result indicates
invalid JSON or options.

The release library is:

- macOS: `target/release/libsyncer_rs.dylib`
- Linux/Android: `target/release/libsyncer_rs.so`
- Windows: `target/release/syncer_rs.dll`
- iOS static linking: `target/release/libsyncer_rs.a`

## Node and browser Wasm

```sh
wasm-pack build --release --target bundler -- --features wasm
```

```js
import init, { mergeJson, mergeJsonWithOptions } from "./pkg/syncer_rs.js";

await init();
const merged = mergeJsonWithOptions(baseJson, incomingJson, {
  arrayStrategy: 4,
  resolveByTimestamp: true,
  lwwKeys: "updatedAt,syncedAt",
  arrayMatchKeys: "id",
});
```

Use `--target nodejs` instead of `bundler` for a CommonJS-oriented Node
package, or `--target web` for direct browser module loading.

### Options are camelCase, and unknown keys are rejected

The options object accepts exactly these keys:

| Key | Type | Default |
|---|---|---|
| `arrayStrategy` | `0..=4` | `0` (replace) |
| `maxDepth` | integer, `0` = unlimited | `0` |
| `resolveByTimestamp` | boolean | `false` |
| `lwwKeys` | comma-separated string | `"updatedAt"` when resolving |
| `fwwKeys` | comma-separated string | disabled |
| `arrayMatchKeys` | comma-separated string | `"id"` |

Anything else throws. This is deliberate: the Rust and C ABI surfaces name the
same option `array_strategy`, and silently ignoring that spelling produced a
**replace** merge with no diagnostic — a wrong document rather than an error.

`options` may be omitted, `undefined`, or `null` to take the defaults:

```js
mergeJsonWithOptions(base, incoming);            // defaults
mergeJsonWithOptions(base, incoming, undefined); // defaults
mergeJsonWithOptions(base, incoming, {});        // defaults
mergeJsonWithOptions(base, incoming, { array_strategy: 1 }); // throws
```

Both functions take and return JSON **strings**. Do not round-trip documents
through `JSON.parse`/`JSON.stringify` on the way in or out: JavaScript numbers
cannot represent an int64 HLC timestamp such as `1689464777831256277`, and the
string boundary is what preserves it.

## Canonical JSON Schema boundary

[`schema/merge-options.schema.json`](schema/merge-options.schema.json) is the
Draft 2020-12 source of truth for option names, types, defaults, and strategy
codes at language-neutral boundaries. The Rust crate embeds the exact file as
`MERGE_OPTIONS_JSON_SCHEMA`; schema registries and code generators therefore do
not need a separate network fetch.

Rust services receiving options as JSON should use the strict validator rather
than deserializing an ad-hoc local type:

```rust
use syncer_rs::merge_json_with_schema_options;

let merged = merge_json_with_schema_options(
    base_json,
    incoming_json,
    r#"{"arrayStrategy":4,"resolveByTimestamp":true}"#,
)?;
```

Unknown keys, snake_case wire keys, invalid strategy codes, wrong types, and a
`maxDepth` outside the unsigned 32-bit range are rejected. The WebAssembly
surface uses this same Rust boundary type and key list, so its contract cannot
silently drift from the schema validator.

## Ores logging and shared context

The deterministic engine performs no I/O and installs no global logger or
OpenTelemetry provider. `merge_json_observed` and
`merge_optional_json_observed` accept an application-owned
`MergeObservationSink` and emit one structured, payload-safe event after each
attempt. Documents, identity values, selectors, and request context are never
placed in that event.

Applications adapt the event to the Rust target of
`oresoftware/next-loggers`; that logger attaches its current Ores
`RequestContext`/`LogContext`. A broken sink is fail-open and cannot change the
merge result. The event wire contract is
[`schema/merge-observation.schema.json`](schema/merge-observation.schema.json)
and is embedded as `MERGE_OBSERVATION_JSON_SCHEMA`.

The Zed manifest declares the shared integration boundaries using their
canonical package coordinates:

- [`ores-otel/ores-interfaces`](https://github.com/ores-otel/ores-interfaces)
  `^0.1.0`;
- `oresoftware/next-loggers` `^0.1.0`, published from
  [`ores-otel/ores.otel.log`](https://github.com/ores-otel/ores.otel.log).

These remain Zed dependencies rather than guessed Cargo registry crates. This
keeps Rust, Dart, and TypeScript consumers on the same polyglot source packages
and lets each consumer select the appropriate native target.

## PostgreSQL and Supabase

The intended database extension exposes
`syncer_merge_jsonb(base, incoming, options)` and delegates directly to this
crate. Stored procedures and triggers can then reconcile rows atomically
without creating a SQL-only semantics fork. See
[`docs/POSTGRES_SUPABASE.md`](docs/POSTGRES_SUPABASE.md).

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --features wasm   # src/wasm.rs is feature-gated
cargo build --release --target wasm32-unknown-unknown --features wasm
```

### Wasm host conformance

Compiling for `wasm32-unknown-unknown` proves the crate builds; it does not
instantiate the module or exercise the wasm-bindgen glue. The behavior a
JavaScript caller actually observes is covered by one corpus,
[`tests/wasm/cases.mjs`](tests/wasm/cases.mjs), executed in two hosts:

```sh
make test-wasm      # the corpus under Node
make test-browser   # the same corpus in real Chromium, via Playwright
make test-all       # cargo + Node + Chromium
```

Both hosts load the same `--target web` artifact, so Node passing while a
browser fails is itself a detectable regression. Add cases to `cases.mjs`
rather than to either runner. CI runs both in
[`.github/workflows/wasm-browser.yml`](.github/workflows/wasm-browser.yml).

To compare against the `syncer.c` differential corpus:

```sh
cargo run --release --example jsonl_runner -- \
  ../syncer.c/test-differential/corpus.jsonl \
  /tmp/results-rust-native.jsonl

node ../syncer.c/test-differential/compare.js \
  --corpus ../syncer.c/test-differential/corpus.jsonl \
  c=../syncer.c/test-differential/results-c.jsonl \
  rust-native=/tmp/results-rust-native.jsonl
```

When this repo is checked out as a sibling of `syncer.c`,
`syncer.c/test-differential/run_all.sh` picks it up automatically as a sixth
implementation (`rustnative`) and additionally runs `rustnative-fuzz`, a
randomized C-vs-Rust differential over all array strategies and options.

### Byte parity with the C core

Merge output is **byte-identical** to yyjson's writer, which the whole
binding ecosystem treats as canonical. Two things make that true here and
must not regress:

- `src/canonical.rs` serializes doubles in yyjson's format (fixed notation
  for decimal exponents in `[-6, 20]`, always with a fractional digit;
  otherwise scientific with no `+` and no zero-padding, e.g. `2e34`).
- serde_json's `float_roundtrip` feature is enabled so parsing is correctly
  rounded (the default fast path is up to 1 ULP off on inputs like `9e29`).
