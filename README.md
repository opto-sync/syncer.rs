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
cargo build --release --target wasm32-unknown-unknown --features wasm
```

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
