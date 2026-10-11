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

Every tool returns its result as JSON in `structuredContent`, with the same
JSON in a text block for clients that read only text. A tool that wraps one
RPC returns that RPC's response message as proto3 JSON, the mapping
manifests and the `*_json` inputs also use, so an agent reads field names
and enum names directly. For example, `put_entity_json` returns:

```json
{ "entity": { "event": { "id": { "namespace": "shop", "slug": "order-placed", "version": "1" }, "title": "Order placed" } }, "etag": "..." }
```

A tenant-typed `Any` renders through the type libraries of the namespaces
the response names.

`get_entity` takes `format: "binpb"` to return
`{ "binpb_base64": "<base64-encoded protobuf>" }` instead, for the
`put_entity` / `batch_mutate` path that takes protobuf in. The `json` flag
that some tools used to take is still accepted and has no effect.

## Error contract

A tool that runs and fails returns an `isError` result, so the client
shows it to the model. Its JSON is `message`, `category` and
`next_action`, plus `code`, `next_action_detail` and `operation_id` when
they apply. This includes bad input, not-found, stale plans and the
server's authorization refusals. Only failures before a tool runs are
JSON-RPC errors: an unknown tool name, and an `Authorization` or branch
header the endpoint cannot read.

Internal helpers (`descriptor_for`, `message_from_json`, ...) use `anyhow`
for ergonomic context-attachment. The public dispatch surface goes through
`McpError` (see `error.rs`), which classifies every failure:
client-shape failures (`UnknownKind`, `InvalidBase64`, ...) are
`validation`; unclassified faults are `internal`.

## Adding new kinds

`parse_kind` is driven by `trogon_atlas_proto::canonical::parse_kind`,
which iterates `ALL_KINDS`. Adding a new proto enum variant
automatically extends the MCP layer's accepted kind strings.
