/**
 * The WebAssembly conformance corpus.
 *
 * This file is the single source of truth for both host runtimes:
 *
 *   - `run-node.mjs`      executes it under Node;
 *   - `browser.spec.mjs`  executes the same cases inside real Chromium.
 *
 * Keeping one corpus is the point. `src/wasm.rs` is the only merge surface in
 * this repository whose behavior depends on a JavaScript host, so Node passing
 * and a browser failing (or the reverse) is exactly the class of defect these
 * tests exist to catch. Add cases here, never to a single runner.
 *
 * Each case is `{ name, base, incoming, options?, expect | throws }`:
 *   - `options` omitted entirely calls the two-argument `mergeJson`;
 *   - `options: undefined | null | {...}` calls `mergeJsonWithOptions`;
 *   - `expect` is the exact expected output string (byte comparison);
 *   - `throws` is a substring required to appear in the thrown message.
 */

/** Array strategy values are a cross-language contract; see README.md. */
export const STRATEGY = Object.freeze({
  REPLACE: 0,
  APPEND: 1,
  UNION: 2,
  MERGE_BY_INDEX: 3,
  MERGE_BY_KEY: 4,
});

const ARRAY_BASE = '{"items":[{"id":1,"name":"one"},2,3]}';
const ARRAY_INCOMING = '{"items":[{"id":1,"active":true},3,4]}';

export const cases = [
  // ---- the two-argument default surface -----------------------------------
  {
    name: 'default merge deep-merges objects and incoming scalars win',
    base: '{"a":1,"nested":{"keep":true,"replace":"old"}}',
    incoming: '{"nested":{"replace":"new","add":2},"b":3}',
    expect: '{"a":1,"nested":{"keep":true,"replace":"new","add":2},"b":3}',
  },

  // ---- absent / empty options ---------------------------------------------
  // Regression: `undefined` and `null` previously failed with
  // "invalid type: unit value, expected struct CanonicalMergeOptions", even though
  // the struct is `#[serde(default)]`.
  {
    name: 'options may be an empty object',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: {},
    expect: ARRAY_INCOMING,
  },
  {
    name: 'options may be undefined',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: undefined,
    expect: ARRAY_INCOMING,
  },
  {
    name: 'options may be null',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: null,
    expect: ARRAY_INCOMING,
  },
  {
    name: 'an array is not an options object',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: [],
    throws: 'expected an object, got an array',
  },

  // ---- the documented array strategies ------------------------------------
  {
    name: 'strategy 0 replaces',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: { arrayStrategy: STRATEGY.REPLACE },
    expect: '{"items":[{"id":1,"active":true},3,4]}',
  },
  {
    name: 'strategy 1 appends',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: { arrayStrategy: STRATEGY.APPEND },
    expect: '{"items":[{"id":1,"name":"one"},2,3,{"id":1,"active":true},3,4]}',
  },
  {
    name: 'strategy 2 unions structurally new elements',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: { arrayStrategy: STRATEGY.UNION },
    expect: '{"items":[{"id":1,"name":"one"},2,3,{"id":1,"active":true},4]}',
  },
  {
    name: 'strategy 3 merges by index',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: { arrayStrategy: STRATEGY.MERGE_BY_INDEX },
    expect: '{"items":[{"id":1,"name":"one","active":true},3,4]}',
  },
  {
    name: 'strategy 4 merges by identity key',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: { arrayStrategy: STRATEGY.MERGE_BY_KEY },
    expect: '{"items":[{"id":1,"name":"one","active":true},2,3,4]}',
  },
  {
    name: 'merge-by-key treats numeric 42 and string "42" as one record',
    base: '{"rows":[{"id":42,"left":1}]}',
    incoming: '{"rows":[{"id":"42","right":2}]}',
    options: { arrayStrategy: STRATEGY.MERGE_BY_KEY, arrayMatchKeys: 'id' },
    expect: '{"rows":[{"id":"42","left":1,"right":2}]}',
  },

  // ---- timestamp vetoes ----------------------------------------------------
  {
    name: 'a stale LWW node is vetoed whole',
    base: '{"doc":{"updatedAt":200,"value":"base"}}',
    incoming: '{"doc":{"updatedAt":100,"value":"stale","added":true}}',
    options: { resolveByTimestamp: true, lwwKeys: 'updatedAt' },
    expect: '{"doc":{"updatedAt":200,"value":"base"}}',
  },
  {
    name: 'a later FWW node loses the whole node',
    base: '{"doc":{"createdAt":100,"value":"first"}}',
    incoming: '{"doc":{"createdAt":200,"value":"recreated"}}',
    options: { resolveByTimestamp: true, fwwKeys: 'createdAt' },
    expect: '{"doc":{"createdAt":100,"value":"first"}}',
  },

  // ---- depth ---------------------------------------------------------------
  {
    name: 'maxDepth replaces the boundary subtree',
    base: '{"a":{"b":{"base":true,"same":"old"}}}',
    incoming: '{"a":{"b":{"incoming":true,"same":"new"}}}',
    options: { maxDepth: 2 },
    expect: '{"a":{"b":{"incoming":true,"same":"new"}}}',
  },
  {
    name: 'detectCircularRefs is accepted and inert for owned JSON trees',
    base: '{"nested":{"left":true}}',
    incoming: '{"nested":{"right":true}}',
    options: { detectCircularRefs: true },
    expect: '{"nested":{"left":true,"right":true}}',
  },

  // ---- cross-engine byte parity -------------------------------------------
  // The canonical writer must match the C core's yyjson output exactly, or
  // documents merged by different engines stop being byte-identical.
  {
    name: 'canonical doubles keep yyjson scientific form',
    base: 'null',
    incoming: '9e29',
    expect: '9e29',
  },
  {
    name: 'canonical doubles keep yyjson fixed form',
    base: 'null',
    incoming: '1e11',
    expect: '100000000000.0',
  },
  {
    name: 'negative zero keeps its sign',
    base: 'null',
    incoming: '-0.0',
    expect: '-0.0',
  },

  // ---- JavaScript number precision ----------------------------------------
  // This is the case that most justifies a browser test. The value exceeds
  // Number.MAX_SAFE_INTEGER, so a host that round-tripped it through
  // JSON.parse would silently corrupt it. The wasm boundary takes and returns
  // strings, so the digits must survive exactly.
  {
    name: 'int64 timestamps survive the string boundary without precision loss',
    base: 'null',
    incoming: '{"updatedAt":1689464777831256277}',
    expect: '{"updatedAt":1689464777831256277}',
  },
  {
    name: 'int64 LWW comparison is exact at one-unit resolution',
    base: '{"doc":{"updatedAt":1689464777831256277,"value":"exact"}}',
    incoming: '{"doc":{"updatedAt":1689464777831256276,"value":"stale"}}',
    options: { resolveByTimestamp: true, lwwKeys: 'updatedAt' },
    expect: '{"doc":{"updatedAt":1689464777831256277,"value":"exact"}}',
  },

  // ---- rejected input ------------------------------------------------------
  {
    name: 'invalid base JSON is an error',
    base: '{oops',
    incoming: '{}',
    throws: 'base is not valid JSON',
  },
  {
    name: 'invalid incoming JSON is an error',
    base: '{}',
    incoming: '[oops',
    throws: 'incoming is not valid JSON',
  },
  {
    name: 'an out-of-range strategy is an error',
    base: '{}',
    incoming: '{}',
    options: { arrayStrategy: 99 },
    throws: 'outside the supported range',
  },
  {
    name: 'a negative strategy is an error',
    base: '{}',
    incoming: '{}',
    options: { arrayStrategy: -1 },
    throws: 'outside the supported range',
  },
  {
    name: 'a wrongly typed strategy is an error',
    base: '{}',
    incoming: '{}',
    options: { arrayStrategy: '1' },
    throws: 'invalid merge options',
  },
  {
    name: 'a wrongly typed detectCircularRefs flag is an error',
    base: '{}',
    incoming: '{}',
    options: { detectCircularRefs: 'yes' },
    throws: 'invalid merge options',
  },

  // ---- unknown options are rejected, not ignored --------------------------
  // Regression: `serde_wasm_bindgen` resolves struct fields by direct property
  // lookup, so `deny_unknown_fields` never fires at the wasm boundary. These
  // previously returned a silently wrong merge result.
  {
    name: 'the Rust/C snake_case spelling is rejected, not silently ignored',
    base: ARRAY_BASE,
    incoming: ARRAY_INCOMING,
    options: { array_strategy: STRATEGY.APPEND },
    throws: 'unknown merge option `array_strategy`',
  },
  {
    name: 'a misspelled option is rejected',
    base: '{"a":1}',
    incoming: '{"b":2}',
    options: { arrayStrategyy: 1 },
    throws: 'unknown merge option `arrayStrategyy`',
  },
  {
    name: 'an unrelated key is rejected',
    base: '{"a":1}',
    incoming: '{"b":2}',
    options: { bogusKey: 123 },
    throws: 'unknown merge option `bogusKey`',
  },
];

/**
 * Runs the corpus against a bound wasm module.
 *
 * `api` is `{ mergeJson, mergeJsonWithOptions }`. Returns one plain,
 * structured-cloneable result per case so the browser harness can hand the
 * outcome back to Playwright without losing information.
 *
 * The runner is shared so the Node and browser paths cannot diverge in how
 * they interpret a case — in particular, whether `options` was absent versus
 * explicitly `undefined`, a distinction that does not survive serialization.
 */
export function runCasesDetailed(api) {
  return cases.map((testCase) => {
    const { name, base, incoming, expect, throws } = testCase;
    const usesOptions = 'options' in testCase;

    let actual;
    let thrown;
    try {
      actual = usesOptions
        ? api.mergeJsonWithOptions(base, incoming, testCase.options)
        : api.mergeJson(base, incoming);
    } catch (error) {
      thrown = String(error?.message ?? error);
    }

    if (throws !== undefined) {
      if (thrown === undefined) {
        return { name, ok: false, detail: `expected a throw containing ${JSON.stringify(throws)}, got ${JSON.stringify(actual)}` };
      }
      if (!thrown.includes(throws)) {
        return { name, ok: false, detail: `expected message containing ${JSON.stringify(throws)}, got ${JSON.stringify(thrown)}` };
      }
      return { name, ok: true, detail: '' };
    }

    if (thrown !== undefined) {
      return { name, ok: false, detail: `unexpected throw ${JSON.stringify(thrown)}` };
    }
    if (actual !== expect) {
      return { name, ok: false, detail: `expected ${JSON.stringify(expect)}, got ${JSON.stringify(actual)}` };
    }
    return { name, ok: true, detail: '' };
  });
}

/** Failure descriptions only, for the plain Node runner. */
export function runCases(api) {
  return runCasesDetailed(api)
    .filter((result) => !result.ok)
    .map((result) => `${result.name}: ${result.detail}`);
}
