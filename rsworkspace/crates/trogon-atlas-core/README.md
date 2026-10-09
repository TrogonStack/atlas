# trogon-atlas-core

Schema-derived helpers that sit one layer above the generated proto
types and one layer below every storage backend, server, and tool.

If you're implementing a third-party storage backend (sqlite, postgres,
filesystem), depend on this crate plus `trogon-atlas-proto`, not
`trogon-atlas-store`. You get the graph-walking primitives and the
incarnation-id policy without pulling in `async-nats` and the trait the
server-side adapters implement.

## Modules

- `content_hash`: a content hash per entity and per whole-model snapshot.
- `id_component`: `validate_id_component`, the shared rule for what makes
  a safe namespace or slug component.
- `namespace`: namespace identity, display name, and hierarchy position.
- `operation_id`: idempotency-key primitives behind `OperationReceipt`.
- `refs`: `outbound_refs`, `retarget_refs`, `entity_id`, `entity_kind`,
  `OutboundRef`. Given an `Entity`, enumerate every `(target, path)` pair
  where the entity points at another addressable thing. Used by the
  reverse-reference index, `GetIncomingReferences`, `GetImpact`, and
  validation pass after validation pass.
- `schema`: the field list behind an entity's `schema` Any.
- `semantic_eq`: `semantically_equal`, the equality behind idempotent puts.
- `system_meta`: `stamp_system(prev, entity)`. Mints / preserves the
  server-owned `SystemMeta` block (UUIDv7 + ISO-8601 `created_at`,
  Kubernetes `metadata.uid` style). Custom storage backends should call
  this before writing so the contract stays uniform.
- `transcode`: the single proto to JSON transcode path.
- `writer_lease`: `WriterRole` and `Epoch`, the single-writer vocabulary.

## Dependencies

Beyond `trogon-atlas-proto`, the crate depends on `prost`, `prost-types`,
`prost-reflect`, `serde_json`, `sha2`, `uuid`, `chrono`, `thiserror`, and
`tracing`. It does not depend on `async-nats` or `trogon-atlas-store`; the
only async stack it carries is `tonic`, which comes with the generated
types in `trogon-atlas-proto`.
