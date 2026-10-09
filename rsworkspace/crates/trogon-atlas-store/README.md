# trogon-atlas-store

The `Store` trait and its production implementation: `NatsStore` (NATS JetStream KV).

`NatsStore` is the only backend shipped in this crate. It is durable and
exposes the change feed directly out of JetStream.

`NatsStore` itself supports multi-process readers and uses CAS-safe writes
(`if_match` / ETag semantics) to guard against concurrent updates at the
store layer. However, `trogon-atlas-server` adds an in-process snapshot cache
(`EventModelServiceImpl::snapshot_cache`) that is invalidated only by
local mutations. A second writer process bypasses this cache and its writes
will leave the cache stale until a local mutation triggers `invalidate_snapshot`.
The consequence is that the in-process view drifts from the true store state:
read RPCs (projections, impact graphs, scenario validation) will silently
serve stale data until the next local mutation forces a refresh. The server
therefore assumes a single writer instance; run multiple replicas only in
read-only mode or accept the staleness window for concurrent writers.

## When to implement your own

A third-party backend should:

1. Depend on `trogon-atlas-core` for `refs::*` and
   `system_meta::stamp_system`. Do NOT take a dep on `trogon-atlas-store`.
2. Implement the `Store` trait declared in `store.rs`.
3. Apply `stamp_system` before every write so client-provided
   `system` blocks are always overwritten and incarnation identity
   stays canonical.
4. Honor the `if_match` ETag semantics (`StoreError::EtagMismatch`).
5. Emit `ChangeRecord`s on the broadcast channel so the change-feed
   RPCs work.

## Modules

- `store`: `Store` trait, `MutationOp`, `MutationOutcome`,
  `ListFilter`, `ChangeKind`, `ChangeRecord`, `StoredEntity`.
- `error`: `StoreError`, `StoreResult`.
- `key`: internal entity-key encoding used by `NatsStore`.
- `nats`: `NatsStore`, `NatsStoreConfig`.

`refs` and `stamp_system` are re-exported here for compatibility with
historical import paths; new code should go through `trogon-atlas-core`
directly.
