# trogon-atlas-proto

Generated gRPC bindings + reflection metadata for the `trogonatlas.api.eventmodel.v1alpha1.EventModelService`
service and the `trogonatlas.{type,annotation,eventmodel,catalog}.v1alpha1` packages it
builds on, all re-exported at the crate root. Every other crate in this stack depends on it. The wire schema
itself lives at `proto`, shared with the TypeScript workspace.
`mise run proto:generate` regenerates `src/gen/` from it with buf; the
output is committed, and CI fails when it is out of date.

## Contents

- `../../../proto/trogonatlas/` holds the canonical schema. Edit it, then run
  `mise run proto:generate` and commit `src/gen/`.
- `src/lib.rs`: re-exports the `tonic`-generated types, plus three
  schema-derived helpers that live at the top of every dependent's
  reach: `EntityKey`, `IdKey`, and the `canonical` module
  (`id_string`, `kind_short`, `parse_kind`, `field_types_compatible`,
  `ALL_KINDS`).
- `FILE_DESCRIPTOR_SET`: the encoded file descriptor set, embedded for
  any tool that needs proto reflection (the MCP adapter uses this for
  JSON ⇆ binpb transcoding).

## Notes

- Package names (`trogonatlas.<area>.v1alpha1`) are the wire identity downstream
  tools encode into type URLs and gRPC paths; moving a message between packages
  changes its type URL.
- `EntityKey` (kind + namespace + slug + version) and `IdKey`
  (namespace + slug + version) are the two newtype keys used by every
  graph/validation pass in `trogon-atlas-core` and `trogon-atlas-server`.
  Always go through them instead of building bare tuples.
- Add new entity kinds in three places, in order: the proto enum,
  `canonical::kind_short`'s match arm, and `canonical::ALL_KINDS`. All
  other callers (`parse_kind`, validators, server-side conversions)
  read from those three sources.
