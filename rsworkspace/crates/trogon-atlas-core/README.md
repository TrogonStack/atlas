# trogon-atlas-core

Schema-derived helpers that sit one layer above the generated proto
types and one layer below every storage backend, server, and tool.

If you're implementing a third-party storage backend (sqlite, postgres,
filesystem), depend on this crate plus `trogon-atlas-proto`, not
`trogon-atlas-store`. You get the graph-walking primitives and the
incarnation-id policy without pulling in `async-nats` and the trait the
server-side adapters implement.

## Modules

- `refs`: `outbound_refs`, `entity_id`, `entity_kind`, `OutboundRef`.
  Given an `Entity`, enumerate every `(target, path)` pair where the
  entity points at another addressable thing. Used by the reverse-
  reference index, `GetIncomingReferences`, `GetImpact`, and validation
  pass after validation pass.
- `system_meta`: `stamp_system(prev, entity)`. Mints / preserves the
  server-owned `SystemMeta` block (UUIDv7 + ISO-8601 `created_at`,
  Kubernetes `metadata.uid` style). Custom storage backends should call
  this before writing so the contract stays uniform.

The crate has zero runtime dependencies outside `uuid` and `chrono`.
