# Integration tests

Every test in this directory backs `EventModelServiceImpl` with an
ephemeral NATS JetStream instance via `trogon-atlas-testsupport`. The
exceptions are noted below.

## What each file covers

| File                              | Surface                                     | Extras needed |
|-----------------------------------|---------------------------------------------|---------------|
| `phase1.rs`                       | CRUD, list, pagination                      | none          |
| `phase8_git_mirror.rs`            | Git mirror put/batch/delete                 | `git` binary on `$PATH` |
| `phase10_ecommerce_fixture.rs`    | Full ecommerce fixture round-trip           | none          |
| `scenario_validation.rs`          | Scenario validation rejection paths         | none          |
| `stream_changes.rs`               | Streaming change-feed RPC                   | none          |
| `llm_http.rs`                     | Anthropic HTTP client (success / 429 / 5xx) | `wiremock` (dev-dep) |
| `metrics_exporter.rs`             | Prometheus exporter wiring                  | none          |

## NATS-backed tests

`trogon-atlas-store/tests/nats_integration.rs`, the store conformance suite,
and this crate's integration tests that need a live store all start a NATS
JetStream container automatically via testcontainers
(`trogon_atlas_testsupport::shared()`). The only requirement is a running
Docker daemon; no environment variables are involved. Without Docker these
tests panic at startup with a "is Docker running?" message.

```sh
cargo test -p trogon-atlas-store
```

## CI assumptions

A fresh clone running `cargo test --workspace -p trogon-atlas-server \
-p trogon-atlas-store -p trogon-atlas-mcp -p trogon-atlas-core \
-p trogon-atlas-proto` should:

* Compile (proto codegen is committed under each proto crate's `src/gen/`).
* Pass every test listed above **except** any that imports a binary or
  service the runner does not provide.

The current external assumptions are: a working C++ toolchain
(transitively via `prost`) and a `git` binary. NATS is **not** assumed.
