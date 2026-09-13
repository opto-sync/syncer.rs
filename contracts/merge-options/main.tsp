import "@typespec/json-schema";

using TypeSpec.JsonSchema;

// Independently authored TypeSpec peer of schema/merge-options.schema.json
// (DEN-3959). Neither authority is generated from the other.
// ORESoftware/typespec-json-schema-validator emits a comparison-only Schema B
// from this file and stops on any unexplained difference with the authored
// Draft 2020-12 schema. Semantics follow src/schema.rs CanonicalMergeOptions:
// camelCase keys, integer strategy codes, defaults, unknown keys rejected.
@jsonSchema
namespace OptoSync.Syncer;

@doc("Canonical language-neutral options accepted by opto-sync reconciliation engines. Host-language APIs may expose idiomatic names, but JSON and JavaScript boundaries use these camelCase keys and integer strategy codes.")
model MergeOptions {
  @doc("0=replace, 1=append, 2=union, 3=merge-by-index, 4=merge-by-key.")
  @minValue(0)
  @maxValue(4)
  arrayStrategy?: integer = 0;

  @doc("Maximum object nesting depth; zero means unlimited.")
  @minValue(0)
  @maxValue(4294967295)
  maxDepth?: integer = 0;

  resolveByTimestamp?: boolean = false;

  @doc("Compatibility option shared with syncer.c. Owned JSON trees cannot contain pointer cycles, so this does not change syncer.rs merge output.")
  detectCircularRefs?: boolean = false;

  @doc("Comma-separated last-write-wins selectors; null uses updatedAt.")
  lwwKeys?: string | null = null;

  @doc("Comma-separated first-write-wins selectors; null disables FWW.")
  fwwKeys?: string | null = null;

  @doc("Comma-separated merge-by-key identity selectors; null uses id.")
  arrayMatchKeys?: string | null = null;
}
