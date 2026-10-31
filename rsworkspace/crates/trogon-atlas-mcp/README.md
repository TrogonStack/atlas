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

The [MCP tool reference](../../../docs/reference/mcp-tools.md) lists every
tool with its minimum role, effect, gRPC methods and parameters. It is
generated from the tool registry; regenerate it with
`mise run rust:mcp-reference` after adding, removing or changing a tool.

## Discovering a manifest schema

`get_entity_schema` returns the JSON Schema for one entity kind's manifest,
cut from the same generator as `trogon-atlas schema`, so the tool and the
CLI never describe different shapes. `schema_sha256` identifies the schema
so a client can cache it. The response also carries a minimal skeleton, the
JSON pointers of the placeholders in it, and the validation rules whose
subject is that kind.

The skeleton satisfies the schema only (`skeleton_validity: "schema_only"`).
Replace every placeholder and add the references and context the kind
needs; apply_manifests refuses a manifest that still holds a placeholder.
Tenant types from a registered type library are not inlined; resolve them
with `resolve_type`. An unknown kind is `invalid_params`; `list_entity_kinds`
names every kind.

Send `x-trogon-atlas-profile: <profile>` to narrow a session to one task's
tools, or fix every session to one with the server's `--mcp-profile`. See
[MCP tool profiles](../../../docs/reference/mcp-tool-profiles.md).

Send `x-trogon-atlas-branch: <name>` on a request to scope its tool calls
to that branch. Omit it for baseline.

For validation repairs, use the
[repair playbook](../../../docs/how-to/repair-validation-findings.md), which
covers dependency discovery, guarded mutations, verification, and preserving
the procedure for future runs.

## Task prompts

The server instructions stay within `SERVER_INSTRUCTIONS_BUDGET_BYTES`
because every session receives them. Task procedures, such as the storyboard
authoring rules or the validation repair procedure, are MCP prompts instead:
a session fetches the one for its task with `prompts/get`. Each prompt lives
in `prompts/` and declares the tools its body names. Crate tests fail when a
prompt names a tool it does not declare, declares one the registry lacks, or
is hidden by the profile its task belongs to, and when the instructions name a tool some profile
hides. The [MCP tool reference](../../../docs/reference/mcp-tools.md#task-prompts)
lists the prompts.

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
