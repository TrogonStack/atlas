# Event Modeling service

A Rust gRPC service that exposes the Event Modeling schema (`event_model.proto`)
as a queryable, denormalized keyspace, plus an MCP stdio adapter that lets an
LLM drive it.

There is no second data model. MCP tools call gRPC; the service is the source
of truth; the canonical proto contract lives at
`proto/event_model.proto`.

## What this is

- A keyspace over the entity kinds defined in `event_model.proto` (events,
  commands, read models, processors, UIs, personas, swimlanes, the slice
  kinds, storyboards, event models, strategic knowledge entities, schema
  entities, screens, and language entities).
- The 24-RPC `EventModelService` surface: direct CRUD, batch mutate,
  graph queries (impact, supersession), projections (slice, storyboard,
  event model), structural diff, AI analysis (data-flow inference and
  information-completeness gaps), full-text search, and a change feed.
- A git side-channel that mirrors every successful mutation to a working tree
  on disk (one commit per `PutEntity` / `DeleteEntity` / `BatchMutate`), so
  history is human-auditable without becoming the system of record.
- A stdio MCP adapter (`trogon-atlas-mcp`) exposing every gRPC method as an MCP
  tool with JSON Schema parameters.

## What this is not

- Not a runtime event bus, message broker, or workflow engine.
- Not a package manager or dependency resolver. `(namespace, slug, version)`
  is identity, not a wire-resolved package.
- Not an IAM or tenant boundary; author identity is taken from gRPC metadata
  and treated as advisory.
- Not opinionated about your domain: this service does not know what an
  "order" is; it only knows what an Event is.

## Crates

The implementation lives under `rustworkspace/`:

| Crate | Purpose |
| --- | --- |
| `trogon-atlas-proto` | Generated `prost`/`tonic` types from `event_model.proto`. |
| `trogon-atlas-store` | Storage trait + in-memory backend + NATS JetStream KV backend. |
| `trogon-atlas-server` | gRPC service implementation, graph indices, projections, validation, diff, search, AI analysis, git mirror, CLI binary. |
| `trogon-atlas-mcp` | stdio MCP adapter that turns each RPC into an MCP tool. |

## Run it

Both binaries take `--help`. The minimum useful invocation:

```sh
# In one terminal, start the gRPC server on :50069 with the in-memory store
# and mirror every mutation into ./mirror as a git tree.
cargo run -p trogon-atlas-server -- \
  --listen 0.0.0.0:50069 \
  --store memory \
  --git-mirror ./mirror

# In another, start the MCP stdio adapter pointing at it. An MCP client
# (Claude Code, Inspector, etc.) speaks JSON-RPC over this process's stdio.
cargo run -p trogon-atlas-mcp -- --grpc http://127.0.0.1:50069
```

Re-import a previously mirrored tree:

```sh
cargo run -p trogon-atlas-server -- import-git ./mirror
```

Container artifacts live next to this README:

```sh
# Build static-musl images and stand up server + NATS + MCP adapter.
docker compose -f experiments/eventmodeling-data/docker-compose.yml up --build
```

The compose file is the recommended deployment shape: NATS JetStream is the
store (data survives restarts via the `nats-data` volume), the server's git
mirror is persisted on `trogon-atlas-mirror`, every service is set to
`restart: unless-stopped`, NATS reports a healthcheck against its monitoring
port, and the server's Prometheus exporter is exposed on `:9100`.

`devops/docker/images/trogon-atlas-server/Dockerfile` and
`devops/docker/images/trogon-atlas-mcp/Dockerfile` are multi-stage builds that compile
each binary against `*-unknown-linux-musl` and ship from Alpine 3.20. Both
images run the binary under `tini` as a non-root user. The server image also
bundles `git` (required by the git-mirror side-channel).

## Author a model

The fast path is to drive `trogon-atlas-mcp` from an LLM. The slow path is to
craft `Entity` messages directly via gRPC. A fully populated e-commerce model
(2 swimlanes, 2 personas, 3 events, 3 commands, 2 read models, 2 UIs, 2
processors, 7 slices, 1 storyboard, 1 event model) is built programmatically
in `atlas/rsworkspace/crates/trogon-atlas-server/tests/phase10_ecommerce_fixture.rs` and
serves as the reference for what a complete model looks like.

Each mutation flows through the same path:

1. Construct an `Entity` (one of the canonical oneof variants).
2. Call `PutEntity` (or pack into a `BatchMutate`). Provide `if_match` from a
   prior etag to opt into optimistic concurrency.
3. The store records the change. If a git mirror is configured, the write is
   committed to disk with author metadata from gRPC headers
   (`x-trogon-atlas-author-name`, `x-trogon-atlas-author-email`).
4. `ListChanges` exposes the change feed; `GetImpact`, projections, and the
   AI RPCs reflect the new state on the next call.

## Drive it via MCP

`trogon-atlas-mcp` exposes every gRPC method as an MCP tool. Read tools return
both a human-readable Debug dump and `binpb_base64` so callers can decode the
exact proto bytes. Mutations have two surfaces:

- **JSON** (no protobuf tooling required): `get_entity_json` -> edit the
  proto3 JSON -> `put_entity_json` / `batch_mutate_json`.
- **binpb**: `get_entity` -> modify `binpb_base64` -> `put_entity` /
  `batch_mutate`.

## Configuration

`trogon-atlas-server` honors both CLI flags and environment variables:

| Flag | Env | Default |
| --- | --- | --- |
| `--listen` | `TROGON_ATLAS_LISTEN` | `0.0.0.0:50069` |
| `--store` | `TROGON_ATLAS_STORE` | `memory` (ephemeral; use `nats` in production) |
| `--nats-url` | `TROGON_ATLAS_NATS_URL` | `nats://127.0.0.1:4222` |
| `--nats-bucket` | `TROGON_ATLAS_NATS_BUCKET` | `trogon-atlas-entities` |
| `--nats-stream` | `TROGON_ATLAS_NATS_STREAM` | `TROGON_ATLAS_CHANGES` |
| `--nats-subject-root` | `TROGON_ATLAS_NATS_SUBJECT_ROOT` | `atlas.changes` |
| `--changes-max-age-days` | `TROGON_ATLAS_CHANGES_MAX_AGE_DAYS` | `90` (0 = forever) |
| `--changes-max-msgs` | `TROGON_ATLAS_CHANGES_MAX_MSGS` | `1000000` (0 = unlimited) |
| `--changes-max-bytes` | `TROGON_ATLAS_CHANGES_MAX_BYTES` | `0` (unlimited) |
| `--git-mirror` | `TROGON_ATLAS_GIT_MIRROR` | unset |
| `--git-author-name` | `TROGON_ATLAS_GIT_AUTHOR_NAME` | `trogon-atlas-server` |
| `--git-author-email` | `TROGON_ATLAS_GIT_AUTHOR_EMAIL` | `trogon-atlas@local` |
| `--metrics-listen` | `TROGON_ATLAS_METRICS_LISTEN` | unset (disabled) |
| `--auth-token` | `TROGON_ATLAS_AUTH_TOKEN` | unset (auth disabled, dev only) |
| `--tls-cert` / `--tls-key` | `TROGON_ATLAS_TLS_CERT` / `TROGON_ATLAS_TLS_KEY` | unset (plaintext) |
| `--max-message-bytes` | `TROGON_ATLAS_MAX_MESSAGE_BYTES` | `4194304` |
| `--concurrency-limit` | `TROGON_ATLAS_CONCURRENCY_LIMIT` | `32` |
| `--llm` | `TROGON_ATLAS_LLM` | `disabled` |
| `--llm-api-key` | `ANTHROPIC_API_KEY` | unset |
| `--llm-model` | `TROGON_ATLAS_LLM_MODEL` | provider default (Sonnet-class) |
| `--llm-base-url` | `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` |
| `--llm-prompt-budget` | `TROGON_ATLAS_LLM_PROMPT_BUDGET` | `24000` |

`--store=nats` requires a reachable JetStream server at `--nats-url` and is
the recommended setting whenever data must outlive the process. `memory`
is suitable for local development, CI, and one-shot fixture replay; the
server logs a prominent warning when it starts ephemeral.

`trogon-atlas-mcp` is configured with `--grpc` / `TROGON_ATLAS_GRPC` (use an
`https://` endpoint for TLS) and `--auth-token` / `TROGON_ATLAS_AUTH_TOKEN`
to match the server's shared bearer token.

### Seeding and restore

`trogon-atlas-server seed --fixture <path>` applies a `BatchMutateRequest`
fixture (`.binpb` or `.textproto`) to the configured store; pass
`--store nats` to seed a live deployment. `trogon-atlas-server import-git
<path>` replays a git mirror working tree into the configured store; see
`RUNBOOK.md` for backup/restore procedures.

## Observability

Both binaries emit JSON structured logs to stderr via `tracing-subscriber`.
The server installs a `trace_fn` on the tonic transport so every RPC opens a
span named `rpc` with the gRPC method path attached. Set `RUST_LOG` (e.g.
`RUST_LOG=info,trogon_atlas_server=debug`) to control verbosity.

Set `OTEL_EXPORTER_OTLP_ENDPOINT` (e.g. `http://otel-collector:4317`) to
export the `rpc` and `llm.complete` spans over OTLP/gRPC; without it spans
are only visible in the logs.

When `--metrics-listen=0.0.0.0:9100` is set, the server installs a global
Prometheus recorder and exposes `/metrics`. Emitted families:
`rpc_requests_total{service,method,code}`,
`rpc_request_duration_seconds{service,method}`,
`rpc_errors_total{service,method,code}` for the subset where `code != "ok"`,
`rpc_stream_terminations_total{method,outcome}` for server-streaming RPCs,
`changes_stream_messages` / `changes_stream_bytes` /
`changes_stream_oldest_age_seconds` gauges polled every 30s from the
configured store (NATS only; `MemStore` reports nothing), and
`llm_requests_total` / `llm_request_duration_seconds` /
`llm_tokens_total` / `llm_prompt_truncations_total` when the LLM path is
enabled. A standard gRPC health service (`grpc.health.v1.Health`) is
served alongside the main service for readiness probes.

Suggested alerts on the change-stream gauges:

- `changes_stream_oldest_age_seconds > $((85 * 24 * 3600))` when the
  configured `--changes-max-age-days=90`: retention is about to bite.
- `changes_stream_messages > 0.9 * $TROGON_ATLAS_CHANGES_MAX_MSGS`: same
  for the message cap.
- `changes_stream_oldest_age_seconds == 0` for more than an hour while
  the model is being actively edited: the publisher loop or the NATS
  connection is broken.

## Status

| Phase | Item | Status |
| --- | --- | --- |
| 0 | Scaffolding, codegen, skeleton server | done |
| 1 | Core read/write RPCs | done |
| 2 | Graph indices, `GetImpact`, supersession | done |
| 3 | Projections + `ExtractSubgraph` | done |
| 4 | `BatchMutate` + `ListChanges` | done |
| 5 | `ValidateEventModel` + `DiffEntities` | done |
| 6 | `SearchEntities` (Tantivy in-memory index) | done |
| 7 | AI analysis (`InferDataFlow`, `CheckInformationCompleteness`) | done |
| 8 | Git export side-channel | done |
| 9 | MCP adapter | done |
| 10 | Fixture model + e2e tests, observability, README, container artifacts | done |

The Tantivy search index is long-lived: it is seeded from a store snapshot
on first search and updated incrementally with every mutation, so search
cost no longer grows with model size per request.

## Documentation

The docs follow Diátaxis. How-to content lives at this directory's
root, `RUNBOOK.md` is the only how-to shipped today, and it sits next
to the `compose.yaml` it operates on so an operator finds both
files together. If a second how-to is added, move both into
`docs/how-to/`.

- `RUNBOOK.md` (how-to): operations, backup/restore.
