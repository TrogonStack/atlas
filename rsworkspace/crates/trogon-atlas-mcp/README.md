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
