# Causal envelopes

`syncer-rs` answers two separate questions:

1. **Ordering:** did a mutation happen before, after, or concurrently with the receiver's checkpoint?
2. **Reconciliation:** when a value should be applied or a concurrent conflict must be resolved, how do the JSON values merge?

The existing `merge_*` functions implement reconciliation. The `causal` module implements ordering without a wall clock, network call, database, or global process state.

## Wire shape

```json
{
  "schemaVersion": "opto-sync.causal.v1",
  "documentId": "notes/42",
  "mutationId": "01JZ...",
  "replicaId": "phone-a1b2",
  "clock": {
    "phone-a1b2": 7,
    "desktop-c3d4": 2
  },
  "operation": {
    "kind": "upsert",
    "value": {
      "title": "offline edit"
    }
  }
}
```

A delete is an ordered tombstone:

```json
{
  "kind": "delete"
}
```

The constructor increments the originating replica's counter and snapshots the resulting vector into the envelope. The caller persists both the envelope and its local `VersionVector` in the same local transaction as the optimistic write.

## Receiver algorithm

1. Deserialize the envelope and call `validate()`.
2. Call `disposition_against(checkpoint)`.
3. Handle the result:
   - `duplicate`: acknowledge idempotently without applying the payload again.
   - `stale`: ignore the payload but retain normal mutation-id audit data.
   - `apply`: apply/delete the document, then call `acknowledge_into(checkpoint)` in the same durable transaction.
   - `resolveConcurrent`: run the product's conflict policy, persist the resolved value, then acknowledge the envelope clock.
4. Return the durable checkpoint to the client.

Do not acknowledge a concurrent envelope before its value conflict is durably resolved. Doing so would suppress a retry while leaving the document unresolved.

## Bounds and portability

- Replica IDs are 1–128 ASCII bytes using letters, digits, `.`, `_`, `:`, or `-`.
- Version vectors contain at most 1,024 replicas.
- Counters are positive unsigned 64-bit integers. Zero entries are rejected and omitted.
- Document IDs are at most 512 bytes; mutation IDs are at most 256 bytes.
- The implementation uses ordered maps so Rust, WASM, C-facing wrappers, and generated clients serialize deterministic object key order.

Long-lived installations should rotate or compact inactive replica IDs at the application layer before approaching the replica bound. Compaction requires a server-confirmed causal checkpoint; clients must not drop arbitrary vector entries independently.

## Relationship to optimistic writes

A local-first client should render its rebased `localView`, not the last remote payload. The causal envelope accompanies each queued mutation and provides ordering/deduplication metadata; it does not replace the queue, checkpoint, tombstone retention, or JSON merge policy.
