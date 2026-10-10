# trogon-atlas-mcp

Model Context Protocol (MCP) adapter: a stdio bridge that exposes
each `EventModelService` gRPC RPC as an MCP tool that an AI agent
can call directly.

## Running

```sh
# Defaults: gRPC at http://127.0.0.1:50069, no auth.
trogon-atlas-mcp

# Pointing at a remote server with a bearer token.
trogon-atlas-mcp \
  --grpc https://trogon-atlas.example.com:443 \
  --auth-token "$TROGON_ATLAS_AUTH_TOKEN"
```

The adapter speaks MCP over stdio (the canonical transport). It is
typically launched as a sub-process by an MCP-aware host (Claude Code,
custom agent harness, etc.) rather than invoked directly.

For validation repairs, use the
[repair playbook](../../../docs/how-to/repair-validation-findings.md), which
covers dependency discovery, guarded mutations, verification, and preserving
the procedure for future runs.

## Configuration

| Flag / Env var | Default | Description |
|---|---|---|
| `--grpc` / `TROGON_ATLAS_GRPC` | `http://127.0.0.1:50069` | gRPC endpoint. `https://` endpoints use TLS with native roots. |
| `--auth-token` / `TROGON_ATLAS_AUTH_TOKEN` | _(none)_ | Shared bearer token matching the server's `TROGON_ATLAS_AUTH_TOKEN`. The value is redacted from logs and `--help` output. |
| `--max-message-bytes` / `TROGON_ATLAS_MCP_MAX_MESSAGE_BYTES` | `4194304` (4 MiB) | Maximum gRPC message size for encoding and decoding. Must match the server's cap; set both sides in lock-step when raising the limit. Startup fails with a clear error if the env var is set to a non-numeric value. |

## What the tools return

Every read RPC's response is wrapped in a stable JSON envelope:

```json
{ "binpb_base64": "<base64-encoded protobuf>" }
```

Clients decode the protobuf (or use the matching `*_json` tool when a
JSON view is easier to consume). The previous behavior, embedding
Rust's `Debug` output alongside the binpb, was removed because that
format is not wire-stable.

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
