# Opto Sync bounded causal checkpoint admission: bounded formal model and draft AttoLang oracle

Run `node formal/attolang/model_check.mjs`. The checker explores the
entire finite state graph and requires positive and adversarial witnesses.
A deliberately unsafe transition must be rejected (negative control).

The `.atto` module is a source-level pure policy oracle following the draft
AttoLang syntax. **Neither this Node checker nor this CI workflow compiles
or executes AttoLang, drives the production implementation, or proves
cross-backend parity.** This is abstract model proof only.

This is a scalar ordering projection over versions 0..3 with synthetic out-of-range acknowledgement 4. It does not prove real version-vector concurrency, durable SQLite transactions, clock comparison, C ABI, or WASM parity.

Promotion requires production trace/refinement replay against exact-head
code with nonzero executed steps and compiler-produced `.atto` guests
independently checked on Java, LLVM, Wasm, Forge and .NET.
