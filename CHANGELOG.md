# Changelog

All notable changes to `syncer-rs`. Versions follow the repository convention
in `AGENTS.md`: while the crate is `0.x`, a breaking change takes a minor bump.

## 0.2.1

### Added

- Canonical Draft 2020-12 merge-options schema, embedded in the crate and
  enforced by `parse_merge_options_json` and
  `merge_json_with_schema_options`.
- Injection-based `MergeObservationSink`, observed merge entry points, stable
  error codes, and a payload-safe observation schema for Ores structured
  logging adapters.

### Changed

- The WebAssembly options boundary now reuses the same canonical Rust type and
  option-key list as the JSON validator, removing a duplicate contract that
  could drift.
- Cargo and Zed package versions are aligned at `0.2.1`.

### Packaging

- The Zed manifest declares `ores-otel/ores-interfaces` and
  `oresoftware/next-loggers` using their canonical polyglot package
  coordinates. No unverified native-registry dependency is introduced.

## 0.2.0

### Breaking

- **The WebAssembly `mergeJsonWithOptions` now rejects unknown option keys.**
  Previously an unrecognized key was silently dropped. Because the Rust and C
  ABI surfaces spell the same option `array_strategy` while the JavaScript
  surface expects `arrayStrategy`, a caller porting between them received a
  **replace** merge with no diagnostic — a wrong document rather than an error.
  Unknown keys now throw and name the offending key.

  Callers passing extra keys must remove them. Only the six documented options
  are accepted: `arrayStrategy`, `maxDepth`, `resolveByTimestamp`, `lwwKeys`,
  `fwwKeys`, `arrayMatchKeys`.

  `#[serde(deny_unknown_fields)]` is not sufficient on its own here:
  `serde_wasm_bindgen` resolves struct fields by direct property lookup, so
  unknown keys never reach the generated visitor and the attribute has no
  effect at the wasm boundary. The keys are checked explicitly.

### Fixed

- **`mergeJsonWithOptions(base, incoming, undefined)` and `(…, null)` no longer
  fail.** Both previously threw `invalid type: unit value, expected struct
  WasmMergeOptions` even though the options struct is `#[serde(default)]`.
  Omitting the argument, or passing `undefined`/`null`, now takes the defaults.

### Added

- Wasm host conformance suite: one corpus (`tests/wasm/cases.mjs`) executed
  under both Node (`tests/wasm/run-node.mjs`) and real Chromium
  (`tests/wasm/browser.spec.mjs`, Playwright). Both hosts load the same
  `--target web` artifact, so host divergence is itself detectable. Covers the
  options contract, all five array strategies, LWW/FWW vetoes, yyjson byte
  parity, int64 values beyond `Number.MAX_SAFE_INTEGER`, and error paths.
- Native tests for the wasm option-name and option-value contract, including a
  guard that keeps the accepted-key list in sync with the struct fields.
- `make pkg-web`, `make test-wasm`, `make test-browser`, `make test-all`.

### CI

- `cargo test` now also runs with `--features wasm`; the feature-gated
  `src/wasm.rs` was previously never compiled during tests.
- `native` job extended to a Linux/macOS/Windows matrix. The C ABI smoke test
  runs on Linux and macOS; Windows runs the Rust build and tests only, pending
  a verified MSVC recipe for linking the cdylib import library.
- All actions pinned to commit SHAs, matching the rest of the portfolio.
- Added `permissions: contents: read`, `persist-credentials: false`,
  `concurrency` with cancel-in-progress, and per-job `timeout-minutes`.
- Added a version-pinned `cargo audit` dependency-advisory job.
- Added `.github/workflows/wasm-browser.yml` for the browser suite, retaining
  the Playwright report on failure.

## 0.1.0

- Initial release: Rust library, stable C ABI, and WebAssembly exports over one
  JSON reconciliation core with yyjson-canonical output.
