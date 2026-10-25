# Verify a connected architecture graph

Use this procedure to inspect the same disposable graph through the backend,
CLI, MCP and Studio. The [coverage reference](../reference/architecture-resource-coverage.md#composed-graph)
defines its resources and limits. Docker must be available for the test's NATS
Store. Run commands from the repository root and serialize Cargo work with
other contributors using this checkout.

## Exercise the graph

```sh
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml \
  -p trogon-atlas-server --test connected_architecture_fixture
```

This checks native selectors, reference discovery, source adoption, observation
reorder and same-store manifest reapply. The live harness is ignored during
normal testing.

## Expose the exercised graph

In one terminal, start the bounded manual harness:

```sh
mise exec -- cargo test --manifest-path rsworkspace/Cargo.toml \
  -p trogon-atlas-server --test connected_architecture_fixture \
  serve_connected_architecture_fixture -- --ignored --exact --nocapture
```

Wait for `READY`. Retain the reported namespace and endpoints; namespace IDs
are generated. The model slug is `model`, version `1`. Default listeners are
gRPC `http://127.0.0.1:19473` and MCP `http://127.0.0.1:19474/mcp`.
Both require the synthetic test bearer `connected-fixture-admin`. This token
belongs only to the disposable fixture, not a deployment.

`ATLAS_CONNECTED_FIXTURE_GRPC_PORT` and `ATLAS_CONNECTED_FIXTURE_MCP_PORT`
override the ports; `0` requests an available port. The harness reports the
actual endpoints. `ATLAS_CONNECTED_FIXTURE_SECONDS` sets a lifetime between
1 and 600 seconds, defaulting to 600. The deadline includes listener startup
verification. Interrupting the harness closes its listeners; the test Store
is disposable. Do not seed or restart an existing deployment for this proof.

## Inspect with CLI and MCP

Build the CLI, then replace `<namespace>` with the READY value:

```sh
mise exec -- cargo build --manifest-path rsworkspace/Cargo.toml \
  -p trogon-atlas-cli --bin trogon-atlas
mise exec -- rsworkspace/target/debug/trogon-atlas \
  --endpoint http://127.0.0.1:19473 --auth-token connected-fixture-admin \
  --format json describe component/<namespace>/receipt-service@1
mise exec -- rsworkspace/target/debug/trogon-atlas \
  --endpoint http://127.0.0.1:19473 --auth-token connected-fixture-admin \
  --format json inspect-data-product data_product/<namespace>/receipts@1
```

Connect an MCP client using Streamable HTTP and the same bearer. Initialize
the connection and discover `tools/list` before invoking tools. Query
`get_entity_json` for `automation_slice`, slug `observe`, exact namespace and
version `1`. Its equal ReadModel endpoints must retain distinct connection IDs.
Use `inspect_data_product` with kind `data_product`, slug `receipts` and the
same exact identity. Compare its returned ports, access selections and checked
scope with CLI inspection.

For `list_subject_assignments`, `get_document_detail` and `get_source_run`,
`tools/list` identifies the `request_json` envelope. Its fields follow the
canonical [responsibility requests](../../proto/trogonatlas/api/eventmodel/v1alpha1/responsibility.proto),
[document requests](../../proto/trogonatlas/api/eventmodel/v1alpha1/decision_documents.proto)
and [source requests](../../proto/trogonatlas/api/eventmodel/v1alpha1/source_integration.proto).
Build requests from returned selectors,
including owner UID, local identity and digest pins. Do not guess them from
display names. The source registration/run IDs are `receipt-source` and
`receipt-title-update`; the retained outcome must be APPLIED. Product output
access must resolve independently of its declared lineage and undeclared
provider. Compare CLI/MCP response data for the same request and scope.

Export the fixture namespace, run `diff` against that export and preview reapply
with `--dry-run`. Preserve the command results and original export. These client
checks establish export, comparison and validation-only preview. The fixture's
`assert_roundtrip` establishes persisted same-store no-op reapply. Neither proves
new-store selector rebinding or a new source publication through either client.

## Inspect with Studio

Build and start the repository's bridge against the fixture in another terminal:

```sh
mise exec -- pnpm --dir tsworkspace/webapps/studio build
mise exec -- env TROGON_ATLAS_GRPC=http://127.0.0.1:19473 \
  TROGON_ATLAS_AUTH_TOKEN=connected-fixture-admin \
  TROGON_ATLAS_AUTH_PASSTHROUGH=true PORT=19475 HOST=127.0.0.1 \
  node tsworkspace/webapps/studio/server/index.mjs
```

Open `http://127.0.0.1:19475/em/<namespace>/model`, enter the fixture key when
prompted and reload the model if needed. The bridge serves the built Studio
and forwards each browser credential to the fixture backend. Stop this bridge
after the run.

Use `?focus=data_product:<namespace>/receipts`,
`?focus=decision:<namespace>/delivery-choice`,
`?focus=document:<namespace>/delivery-note` and
`?focus=channel:<namespace>/delivery` to inspect those resources. Confirm the
receipt title is `Source receipt` and its local documentation remains intact.
Inspect exact port responsibility, accepted rationale, document content and
anchor, repeated observation selections and source run records.

Capture screenshots, rendered text and API/console diagnostics with the exact
model identity and candidate build evidence. The fixture intentionally exposes
`SLICE_NOT_IN_STORYBOARD` and `SLICE_NO_SCENARIOS`; it is a bounded resource graph,
not a complete behavioral example. The stock Studio e2e suite uses a fake gRPC
backend and does not substitute for this real-store inspection.
