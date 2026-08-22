# Formal-methods boundary

`syncer.rs` uses different methods for different kinds of claims. A JSON Schema
cannot prove temporal or algebraic behavior, and a bounded code proof does not
prove storage, networking, or another runtime.

## Causal ordering

Kani executes three harnesses against pure functions used by the production
`VersionVector` and `CausalEnvelope` paths:

- the per-replica counter join is commutative, idempotent, and an upper bound
  for every pair of `u64` counters;
- the two-replica partial-order classifier is dual for every four-counter
  combination (`Before` reverses to `After`, while `Equal` and `Concurrent`
  are symmetric);
- all four version relations map to the intended causal dispositions.

These are exhaustive, bit-precise scalar proofs over the complete `u64`
domains; the harnesses have no loop-unwinding assumptions. They do not
symbolically execute Rust's heap-backed `BTreeMap<String, u64>` implementation.
Native exhaustive examples and unit tests cover map construction, iteration,
serialization, invalid identifiers, monotonic observation, and merge replay.
Keeping that boundary explicit avoids treating an incomplete heap exploration
as proof evidence.

Run the proof locally with the pinned verifier used by CI:

```bash
cargo install --locked kani-verifier --version 0.67.0
cargo kani setup
cargo kani --output-format terse
```

## Wire schema

[`schema/causal-envelope.schema.json`](../schema/causal-envelope.schema.json) is
the Draft 2020-12 transport contract for causal mutations. CI compiles it in
strict mode and production tests require its schema version, operation tags,
counter range, and replica-count bound to match the Rust constants and Serde
wire format.

Two semantic constraints deliberately remain in `CausalEnvelope::validate`
because portable JSON Schema cannot express them exactly:

- document and mutation identifiers are bounded by UTF-8 bytes, not Unicode
  code points;
- the dynamic property `clock[replicaId]` must exist and be positive.

Consumers must therefore use both layers at a trust boundary: schema validation
for portable shape and typed validation for cross-field and byte-level rules.

## Claim discipline

A green formal-methods workflow establishes the named Kani properties and JSON
Schema agreement for the committed sources and pinned tools. It does not prove
the entire reconciliation algorithm, JavaScript or Dart host behavior, network
liveness, database durability, or WebAssembly engine correctness. Those claims
remain covered by differential, native, C ABI, Node, and real-browser tests.

The next useful proof slices are:

1. a finite Quint model for multi-replica causal delivery, duplicate replay,
   tombstones, and concurrent resolution;
2. implementation-trace replay from that model through Rust, TypeScript, and
   Dart clients;
3. Loom exploration for any shared in-process synchronization introduced into
   this crate;
4. bounded proofs for timestamp selector and array-identity helpers where the
   production function can be kept allocation-free.
