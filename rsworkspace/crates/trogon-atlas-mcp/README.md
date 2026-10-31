# trogon-atlas-mcp

Model Context Protocol (MCP) tools over Streamable HTTP that expose each
`EventModelService` gRPC RPC as an MCP tool that an AI agent can call
directly. This crate is a library: `trogon-atlas-server` serves it at `/mcp`
on the same listener as gRPC and runs every tool call through the same
authentication and authorization as a gRPC caller.

## Connecting

Enable it on the server with `TROGON_ATLAS_MCP` (see the
[server README](../trogon-atlas-server/README.md)), then point an MCP client
at `http://<host>:<grpc-port>/mcp`. The endpoint is stateless, so any replica
can answer any request.

The endpoint holds no credential of its own. Each MCP request acts as its
own `Authorization: Bearer <token>` header, so the server authenticates and
authorizes the calling client, and `tools/list` shows only the tools that
client's role can call. A request without the header is refused unless the
server allows anonymous callers; a header that is not a bearer token is
rejected.

Send `x-trogon-atlas-branch: <name>` on a request to scope its tool calls
to that branch. Omit it for baseline.

For validation repairs, use the
[repair playbook](../../../docs/how-to/repair-validation-findings.md), which
covers dependency discovery, guarded mutations, verification, and preserving
the procedure for future runs.

## What the tools return

Every read RPC's response is wrapped in a stable JSON envelope:

```json
{ "binpb_base64": "<base64-encoded protobuf>" }
```

Clients decode the protobuf (or use the matching `*_json` tool when a
JSON view is easier to consume). The previous behavior, embedding
Rust's `Debug` output alongside the binpb, was removed because that
format is not wire-stable.

Mutating entity tools (`put_entity`, `put_entity_json`, `batch_mutate`,
`batch_mutate_json`, `delete_entity`, `delete_by_query`,
`retarget_references`, `revert_changeset`, `apply_manifests`) add the scope
the write landed in, because a branch write keeps no durable receipt:

```json
{ "scope": "branch", "branch": "acme-del", "binpb_base64": "..." }
```

`scope` is `baseline` and `branch` is `null` when no branch applies.

The entity protection tools (`protect_entity`, `unprotect_entity`,
`get_entity_protection`, `list_entity_protections`) return plain JSON instead
of `binpb_base64` and carry no `scope`, because protection is baseline-only
and the same on every branch. `protect_entity` and `unprotect_entity` refuse
to run while a branch is set. A listing reports `verdict: "partial"` when the
server could not read every row or enumerate the whole registry. See
`docs/reference/entity-protection-commands.md`.

`put_entity`, `put_entity_json`, `delete_entity`, `batch_mutate` and
`batch_mutate_json` also return the server's validation findings as a JSON
`issues` array next to `binpb_base64`, so an agent reads them without decoding
the protobuf. Findings
are non-blocking, strategic ones included (BoundedContext, Domain and Subdomain
rules), and `validate_only` dry runs return them too:

```json
{
  "scope": "baseline",
  "branch": null,
  "issues": [
    {
      "severity": "warning",
      "code": "DOMAIN_IN_CONTEXT_NAMESPACE",
      "message": "...",
      "subject": "domain:shop/shop@1",
      "field": null
    }
  ],
  "binpb_base64": "..."
}
```

`severity` is `error`, `warning` or `info`; `subject` and `field` are `null`
when the finding names no entity or path. Batch findings also carry
`op_index` (zero-based position of the put or delete op) and `op_entity` (the
entity that op wrote or deleted). `delete_entity` returns what the delete leaves
behind, such as `REALIZES_DANGLING` after force-deleting a realized Subdomain,
and `mode: dry_run` previews it. `merge_branch` adds the same `issues` array from the response's
post-merge validation. Fix every warning and error that comes back.

`apply_manifests` returns the same objects in `issues`, so one shape covers
every mutating tool. `op_index` is the position among the changed manifests
sent in the batch (unchanged manifests are skipped) and `op_entity` is the
manifest's entity id.

Strategic findings also appear once on the store-level report of
`validate_project` (an empty `event_model_id`). After editing only Project,
Domain, Subdomain or BoundedContext entities, run `validate_project` with the
namespace or project scope, since there may be no EventModel to validate.

## Error contract

Internal helpers (`descriptor_for`, `message_from_json`,
`message_to_json`, ...) use `anyhow` for ergonomic context-attachment.
The public dispatch surface goes through `McpError` (see `error.rs`)
which maps structurally to the MCP error codes: client-shape failures
(`UnknownKind`, `InvalidBase64`, ...) surface as `invalid_params`;
everything else as `internal_error`.

## Adding new kinds

`parse_kind` is driven by `trogon_atlas_proto::canonical::parse_kind`,
which iterates `ALL_KINDS`. Adding a new proto enum variant
automatically extends the MCP layer's accepted kind strings.
