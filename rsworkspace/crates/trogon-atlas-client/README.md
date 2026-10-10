# trogon-atlas-client

One implementation of the write-discipline (create/configure/unchanged
classification, semantic no-op detection, YAML manifest parsing, export)
shared by every surface that talks to the trogon-atlas gRPC service:
`trogon-atlas` and the MCP adapter both build on this instead of duplicating it.

## Modules

- `client`: gRPC connect (bearer auth interceptor, x-request-id
  correlation, configurable message size cap and request-id prefix).
- `manifest`: YAML CRD-style manifest parsing into typed `pb::Entity`.
- `apply`: classify manifests against live server state and apply only
  real changes through one atomic `BatchMutate`.
- `export`: render live store state back into manifest YAML.
