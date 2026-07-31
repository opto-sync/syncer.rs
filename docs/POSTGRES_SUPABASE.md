# PostgreSQL and Supabase integration

Database-side reconciliation must call the same merge core; a trigger or stored
procedure must not grow an independent implementation of the merge rules.

## SQL contract

A PostgreSQL extension around `syncer.rs` should expose this function:

```sql
syncer_merge_jsonb(
  base jsonb,
  incoming jsonb,
  options jsonb default '{}'::jsonb
) returns jsonb
```

The extension wrapper serializes each `jsonb` input once, calls
`syncer_rs::merge_json`, and converts the compact result back to `jsonb`.
`options` uses the same camel-case names as the Wasm API:

```json
{
  "arrayStrategy": 4,
  "resolveByTimestamp": true,
  "lwwKeys": "updatedAt,syncedAt,#/_sync/updatedAt",
  "fwwKeys": null,
  "arrayMatchKeys": "id",
  "maxDepth": 0
}
```

The PostgreSQL wrapper is intentionally a separate integration crate. It can
use `pgrx` while depending on this package as a normal Rust library; no merge
rules belong in that wrapper.

## Atomic record reconciliation

Once the extension provides `syncer_merge_jsonb`, a transaction-safe RPC can
lock, reconcile, and update a record:

```sql
create or replace function reconcile_document(
  p_id uuid,
  p_incoming jsonb
) returns jsonb
language plpgsql
security invoker
as $$
declare
  v_current jsonb;
  v_merged jsonb;
begin
  select data
    into v_current
    from documents
   where id = p_id
   for update;

  if not found then
    raise exception 'document % not found', p_id;
  end if;

  v_merged := syncer_merge_jsonb(
    v_current,
    p_incoming,
    jsonb_build_object(
      'arrayStrategy', 4,
      'resolveByTimestamp', true,
      'lwwKeys', 'updatedAt,syncedAt,#/_sync/updatedAt',
      'arrayMatchKeys', 'id'
    )
  );

  update documents
     set data = v_merged
   where id = p_id;

  return v_merged;
end;
$$;
```

Keep row-level security enabled and grant `execute` only to the roles that may
reconcile that table.

## Trigger shape

A table that receives incoming JSON in the same row can use a `before update`
trigger:

```sql
create or replace function reconcile_document_update()
returns trigger
language plpgsql
as $$
begin
  new.data := syncer_merge_jsonb(
    old.data,
    new.data,
    '{"arrayStrategy":4,"resolveByTimestamp":true,"lwwKeys":"updatedAt,syncedAt","arrayMatchKeys":"id"}'::jsonb
  );
  return new;
end;
$$;

create trigger documents_reconcile_before_update
before update of data on documents
for each row execute function reconcile_document_update();
```

Use either this trigger or an explicit reconciliation RPC for a write path, not
both. The explicit RPC is usually easier to audit because callers opt into the
merge and the row lock is visible in one function.

## Deployment boundary

Managed database plans vary in which native PostgreSQL extensions they permit.
Where the Rust extension cannot be installed, keep the SQL contract but perform
the same atomic read/merge/compare-and-set loop in a trusted server or Supabase
Edge Function. Do not replace the native core with a partial PL/pgSQL merge.

