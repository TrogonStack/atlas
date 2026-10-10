# trogon-atlas-server

The gRPC server. Implements every RPC on the
`EventModelService` defined in `trogon-atlas-proto`, backed by any
`Store` implementation from `trogon-atlas-store` (or your own).

## Binary

`trogon-atlas-server` is both a library and a binary. The binary wires
up tracing, the OpenTelemetry exporter, the Prometheus metrics
endpoint, the configured store, the optional LLM analyzer, and an
optional git mirror; then serves tonic over the configured listen
address.

Configuration is CLI-flags + env-vars; run `trogon-atlas-server --help`
for the full list. The key ones:

| Flag                              | Env                                  | Default                |
|-----------------------------------|--------------------------------------|------------------------|
| `--listen`                        | `TROGON_ATLAS_LISTEN`                  | `0.0.0.0:50069`        |
| `--store`                         | `TROGON_ATLAS_STORE`                   | `nats`                 |
| `--nats-url`                      | `TROGON_ATLAS_NATS_URL`                | `nats://127.0.0.1:4222`|
| `--llm`                           | `TROGON_ATLAS_LLM`                     | `disabled`             |
| `--llm-api-key`                   | `TROGON_ATLAS_LLM_API_KEY`             | (fallback: `ANTHROPIC_API_KEY`) |
| `--llm-max-tokens`                | `TROGON_ATLAS_LLM_MAX_TOKENS`          | `4096`                 |
| `--llm-request-timeout-secs`      | `TROGON_ATLAS_LLM_REQUEST_TIMEOUT_SECS`| `60`                   |
| `--llm-concurrency`               | `TROGON_ATLAS_LLM_CONCURRENCY`         | `4`                    |
| `--jev`                           | `TROGON_ATLAS_JEV`                     | `false`                |
| `--jev-api-key`                   | `TROGON_ATLAS_JEV_API_KEY`             | fallback: `AI_GATEWAY_API_KEY` |
| `--jev-timeout-secs`              | `TROGON_ATLAS_JEV_TIMEOUT_SECS`        | `8`                    |
| `--jev-concurrency`               | `TROGON_ATLAS_JEV_CONCURRENCY`         | `4`                    |
| `--jev-cache-capacity`            | `TROGON_ATLAS_JEV_CACHE_CAPACITY`      | `128`                  |
| `--jev-cache-ttl-secs`            | `TROGON_ATLAS_JEV_CACHE_TTL_SECS`      | `300`                  |
| `--jev-min-probability`           | `TROGON_ATLAS_JEV_MIN_PROBABILITY`     | `0.95`                 |
| `--auth-tokens-file`              | `TROGON_ATLAS_AUTH_TOKENS_FILE`        | (unset)                |
| `--auth-token`                    | `TROGON_ATLAS_AUTH_TOKEN`              | (unset, deprecated)    |
| `--insecure-allow-anonymous`      | `TROGON_ATLAS_INSECURE_ALLOW_ANONYMOUS`| `false`                |
| `--protect-baseline`              | `TROGON_ATLAS_PROTECT_BASELINE`        | `false`                |
| `--protect-baseline-namespaces`   | `TROGON_ATLAS_PROTECT_BASELINE_NAMESPACES` | (unset)            |
| `--metrics-listen`                | `TROGON_ATLAS_METRICS_LISTEN`          | (off)                  |
| `--git-mirror`                    | `TROGON_ATLAS_GIT_MIRROR`              | (off)                  |

`--nats-url` accepts credentials as userinfo
(`nats://service:secret@host:4222`) for a server with authentication enabled.
They are lifted out of the URL into the connection options at connect time, so
the URL that reaches logs and error messages no longer carries the secret.

Set `TROGON_ATLAS_GIT_MIRROR_WRITE_TXT=1` to also write a human-readable `.txt`
debug dump alongside each `.binpb` file in the mirror. Off by default because
the debug files double mirror size and clutter git diffs.

When `--git-mirror` is on, schedule the `reconcile` subcommand (see
"Reconciling the mirror" below). A mirror write can fail while the store
commit succeeds, and nothing else detects that divergence.

## Run Jev field inference

Load a Vercel AI Gateway key into `AI_GATEWAY_API_KEY` through your secret
manager, then enable `TROGON_ATLAS_JEV=true` on the server. The key stays on
the server; clients continue using the existing `InferDataFlow` RPC and
MCP `infer_data_flow` tool. Jev uses Gateway's
[evaluation API](https://vercel.com/docs/ai-gateway/modalities/evaluation)
with `typesafe-ai/jev` and requests zero data retention.

From the `atlas` directory, rebuild the local Compose service after
loading the key into the shell environment:

```sh
TROGON_ATLAS_JEV=true mise run compose:up -- --build --no-deps trogon-atlas-server
```

For an existing server launch, add `--jev` to its arguments and keep its
current authentication and NATS configuration. For Kubernetes, use
`server.extraEnv` to set `TROGON_ATLAS_JEV=true` and provide
`AI_GATEWAY_API_KEY` through a Kubernetes Secret reference.

Use `InferDataFlow` with a slice, storyboard, or event-model scope. For
example, replace the identifiers with an existing command slice:

```sh
mise exec -- grpcurl -plaintext \
  -import-path proto -proto service.proto \
  -d '{"scope":{"slice":{"kind":"ENTITY_KIND_COMMAND_SLICE","id":{"namespace":"shop","slug":"place-order","version":"1"}}}}' \
  127.0.0.1:50069 trogonatlas.api.eventmodel.v1alpha1.EventModelService/InferDataFlow
```

Authenticated deployments also need their ordinary bearer metadata.
Branch calls use the existing `x-trogon-atlas-branch` metadata. The analyzer
only receives entities visible to that request.

Explicit derived fields and unique exact-name matches with compatible
shapes use the deterministic path without a provider call. Unresolved
fields are evaluated together. Successful evaluations are cached by their
complete request evidence and policy; simultaneous identical requests
share the result. Cache lifetime and capacity are bounded. The timeout
also includes time spent waiting for evaluation capacity.

A result from Jev has `ANALYSIS_PROVENANCE_LLM`; its mapping rationale
identifies Jev and reports the selected-option probability. This is a
model estimate, not measured correctness or a generated explanation.
The acceptance threshold must be calibrated for your models.
Name equality remains a heuristic: a deterministic mapping does not
establish that identically named fields have the same business meaning.

Uncertain, unknown, over-budget, or failed evaluations use the configured
generative analyzer, or deterministic analysis when that analyzer is
disabled. `fallback_reason` records stable reason codes, including
`jev_uncertain`, `jev_unknown_source`, and `jev_timeout`. If both providers
fail, the codes are comma-separated in attempt order. A fallback is not
evidence that a semantic mapping was established.

This integration covers `InferDataFlow`. `CheckInformationCompleteness`
continues using its existing analyzer. Canonical `trogonatlas.eventmodel.v1alpha1.Schema`
payloads are supported; other schema
payloads cannot produce a successful Jev result. Evaluation limits are
48 unresolved fields, 24 candidate sources per field, 1,024 scoped fields,
and 64 KiB each for the request and response. Exceeding a limit triggers
fallback instead of silently dropping evidence.
Configuration accepts request timeouts up to 60 seconds and concurrency
from 1 through 64; invalid values stop startup.

## Benchmark Jev

From the `atlas` directory, validate the synthetic labeled examples
without contacting a provider:

```sh
mise run jev:benchmark -- --dry-run
```

With `AI_GATEWAY_API_KEY` loaded, run:

```sh
mise run jev:benchmark -- --repeats 3 --concurrency 2
```

The JSON report includes label accuracy, incorrect accepted mappings,
review cases, latency percentiles, and provider-reported usage and cost.
The command exits nonzero on provider errors or incorrect accepted
mappings. These timings measure the provider; they exclude the server's
deterministic shortcuts and cache. Add representative labeled examples
with `--fixtures <path>` before changing the probability threshold.

To exercise the actual handler twice against an isolated test store and
report cold and cached latency, keep the key loaded and Docker running:

```sh
mise exec -- cargo test -p trogon-atlas-server --test jev_handlers \
  live_jev_semantic_mapping -- --ignored --nocapture
```

The ordinary tests mock Gateway and require no paid provider calls.
Gateway pricing and promotion eligibility can change; inspect the
[model page](https://vercel.com/ai-gateway/models/jev) and your team's
Gateway routing before running a large evaluation.

## Authorization model

**Source of truth: `docs/explanation/authorization.md`.** This section is a
summary; that document describes the registry file format, rotation, and what
is deferred.

The server authenticates every RPC with a bearer token and authorizes it
against a per-RPC role table. Concretely:

- **Identity.** `--auth-tokens-file` / `TROGON_ATLAS_AUTH_TOKENS_FILE` points at
  a TOML registry mapping each principal to a role, a list of valid tokens, and
  an optional namespace allow-list (`examples/tokens.toml`). The `BearerAuth`
  interceptor resolves the token to a
  `Principal { name, role, is_anonymous, namespaces }` and inserts it into
  request extensions (`src/auth.rs`). Lookup is constant-time per candidate
  token.
- **Authorization.** `required_role` (`src/auth.rs`) maps each fully qualified
  gRPC path to a minimum role, with `Reader < Writer < Admin`. `AuthzLayer`
  enforces it. Reads require `Reader`, mutations require `Writer`, and
  `DeleteByQuery` requires `Admin`. **Deny by default:** a path absent from the
  table is `PERMISSION_DENIED` for every caller, and a unit test asserts every
  RPC in the service descriptor has an entry, so the table cannot drift from
  the proto.
- **Namespace scope.** A principal may carry `namespaces = ["orders",
  "shop-*"]`, an allow-list applied to every write, on baseline and on
  branches alike, including the merge that lands a branch. Omitting the field
  means unrestricted, so existing registries are unchanged. Reads are never
  scoped.
- **Baseline protection.** With `--protect-baseline`, mutating RPCs that carry
  no `x-trogon-atlas-branch` context are rejected unless the caller is a real
  (non-anonymous) Admin. Merging a branch is then the way baseline changes.
  `--protect-baseline-namespaces=orders,shop-*` narrows that to named
  namespaces, leaving the rest open.

The server refuses to start unless one of `--auth-tokens-file`,
`--auth-token`, or `--insecure-allow-anonymous` is set.

Caveats that are still true:

- **Two RPCs cannot be namespace-scoped.** `RetargetReferences` rewrites
  whatever referenced an entity, and `DeleteByQuery` without a `namespace`
  filter selects its own victims. Neither reach is knowable before the scan,
  so a scoped principal is refused rather than granted an unbounded write.
- **`--insecure-allow-anonymous` grants a synthetic Admin** to every caller
  (`is_anonymous: true`). It exists so a dev stack needs no token file. Code
  that must distinguish "really is Admin" from "RBAC is switched off" checks
  the `is_anonymous` flag, not just the role.
- **`--auth-token` is deprecated.** It maps to a single synthetic
  `legacy-admin` principal with Admin role and logs a warning at startup.
  Prefer the tokens file, which is what makes rotation and per-client roles
  possible.

## Git mirror: content log, principal-attributed

The git mirror records every mutation as a commit, attributed to the
**authenticated principal**. `author_from_metadata` (`src/service.rs`) prefers
the `Principal` in request extensions: the commit author name is the principal
name, and when no advisory `x-trogon-atlas-author-email` header is supplied the
server synthesizes `<principal>@trogon-atlas`. A client cannot spoof another
principal's identity through headers, because `x-trogon-atlas-author-name` is
ignored whenever a principal is present.

The one honest caveat: with no auth stack configured (unit tests that construct
the service directly, or `--insecure-allow-anonymous`, where every caller is
the same synthetic principal), authorship is self-asserted and distinguishes
nobody. Header values are sanitized in both paths (length-capped, control
characters stripped).

The mirror is still a *content* log rather than a complete audit trail, for a
different reason than authorship: it is optional and off by default, so an
operator running without it has no commit history at all.

Every commit to the mirror repository is made with `git commit --no-verify`,
so pre-commit and commit-msg hooks on the mirror repository are always
bypassed. Do not rely on hooks in the mirror repo to enforce policy or
trigger side effects; those hooks will never run for mirror commits.

### Reconciling the mirror

**A mirror write can fail while the store commit succeeds.** `mirror_apply`
logs a warning, increments `git_mirror_failures_total`, and lets the RPC
succeed, which is the right call (the mirror must never fail a write) but
means the two can diverge silently.

`reconcile` is the detector. It compares the configured store against a mirror
working tree, reports every entity that differs, and exits non-zero on drift:

```sh
trogon-atlas-server --store=nats --nats-url=... reconcile --mirror /var/lib/trogon-atlas/mirror
```

Run it on a schedule, not just after an incident. The Helm chart provisions a
CronJob for this automatically when `gitMirror.enabled` and
`gitMirror.reconcile.enabled` are both true (`server.gitMirror.*` under the
`trogon-atlas` umbrella chart); see
`devops/helm/charts/trogon-atlas-server/README.md`.

Alert on these counters. Without them, divergence is discovered on restore day:

| Metric | Meaning | Suggested rule |
|--------|---------|----------------|
| `git_mirror_failures_total` | A store commit succeeded but its mirror write did not. The mirror is now behind. | `increase(...[1h]) > 0` |
| `trogon_atlas_store_change_events_lost_total` | A KV write is durable but its change event was never published. Every change-feed consumer now has a hole. | `increase(...[1h]) > 0` |
| `trogon_atlas_changesets_lost_total` | Writes committed but their changeset never landed. Those changes have no durable attribution: history has a hole. See [`docs/explanation/changesets.md`](../../../docs/explanation/changesets.md). | `increase(...[1h]) > 0` |

The last two are not mirror-specific and matter even with the mirror off: they
are the only signals that a consumer's view of the change feed, or the record
of who changed what, is incomplete.

### Mirror growth, retention, and getting it off-box

The mirror only ever writes to a local working tree. It does not push, has no
remote configuration, and never runs `git gc`. That is deliberate: a
long-running daemon holding push credentials is a materially different security
posture, and a push that can fail is a push that could eventually fail a write.

The consequences are yours to operate:

- **Unbounded growth against a fixed volume.** Every mutation is a commit and
  every entity is a `.binpb` blob. The chart's default PVC is 1Gi
  (`server.gitMirror.persistence.size`); routine local use reaches thousands of
  commits quickly. Size the volume against your write rate, and alert on volume
  utilization the same way you would any other stateful PVC.
- **`git gc` is not automatic.** Run it periodically against the working tree
  (`git -C <mirror> gc --prune=now`) to keep the object store compact. A
  CronJob in the same pod spec as the reconcile job is the natural home.
- **The audit log dies with the volume.** If the mirror is load-bearing for
  you, replicate it off-box.

**Supported pattern for pushing off-box:** a sidecar container or CronJob that
mounts the same PVC and pushes to a remote, holding its own credentials with
its own lifecycle. The server is not involved and cannot be made to fail by a
push failure. If push ever moves in-process it must stay optional and must
never be able to fail a write, exactly as apply does today.

## Batch mutation atomicity

NATS KV has no multi-key transaction, so a baseline batch (`BatchMutate`,
`RevertChangeset`, `DeleteByQuery`, `RetargetReferences`) is recoverable
rather than transactional. Before its first entity write the store records
the prior image of every key the batch touches in the
`trogon-atlas-batches` KV bucket, and it removes that journal entry only
once the batch's revisions, change events and changeset have all been
written. Every entity in a batch therefore ends up in one of two states:
applied, with its revision row, change event and changeset, or absent,
holding its prior image again.

| Where the batch stopped | Caller saw | Recovery |
| --- | --- | --- |
| An op failed and the rollback succeeded | The op's error | Nothing to do. |
| An op failed and the rollback did not finish | `UNAVAILABLE` naming the journal, never a `STATUS_FAILED` batch response | Rolls the batch back. |
| The process died before every write landed | No response | Rolls the batch back. |
| Every write landed, a change event, revision or changeset did not | Success | Rolls the batch forward: republishes every change event, rewrites the revisions, appends the changeset. |

A batch is only ever undone when the caller was never told it succeeded,
and only ever completed when it was. The server runs recovery at
startup, before the first request, and every minute in the background.
Both passes leave journal entries younger than a minute alone, because
during a rolling deploy the old process may still be writing them; a
batch interrupted just before a restart is therefore repaired by the
sweep rather than at startup. To check or repair without a server:

```sh
trogon-atlas-server recover-batches --report-only   # JSON report, exit 1 if anything is pending
trogon-atlas-server recover-batches                 # repair, exit 1 unless every batch converged
```

Pass `--min-age-secs` when running beside a live server so the pass leaves
batches still in flight alone. Recovery assumes the single-writer
deployment the rest of the store assumes: a key that another writer
changes while its batch is interrupted is rolled back over that write, or,
when rolling forward, reported as `superseded` and left without a revision
row because the image the batch wrote no longer exists.

Not journaled: branch batches (a branch is a scratch overlay that nothing
outside it reads until it is merged), single-entity writes (one KV write,
so there is nothing partial to undo; a lost change event or revision is
still counted by the lost-event metrics below), and `MergeBranch`, which
keeps its own compensating rollback.

## Change feed delivery guarantee

The change feed (`ListChanges` / `StreamChanges`) is at-least-once
from the durable change log, with these caveats:

- KV writes and change-log publishes are separate operations. The
  store retries failed publishes with backoff. For a baseline batch a
  publish that still fails is repaired by batch recovery, which
  republishes the batch's events (see "Batch mutation atomicity"). For a
  single-entity write it is not: the write is durable without a change
  event. Both are logged at error level and counted by
  `trogon_atlas_store_change_events_lost_total`. Alert on it, and on
  `trogon_atlas_store_batches_pending_recovery_total` and
  `trogon_atlas_batch_recovery_unresolved_total`.
- Recovery republishes every event of a batch it rolls forward, using the
  same `Nats-Msg-Id` the original publish used. JetStream drops the
  duplicates it sees inside its duplicate window (two minutes by
  default); a repair later than that can deliver an event twice. A
  consumer keyed on `(changeset_id, entity)` sees no difference.
- The stream has finite retention (90 days and 1,000,000 messages by
  default). Treat the change feed as a **notification transport**, not as
  a durable record: anything that must survive retention needs its own
  storage rather than a replay of this stream.
- `StreamChanges` terminates with `DATA_LOSS` when a subscriber falls
  behind the broadcast buffer. Clients must treat that status as a
  signal to re-list from their last token and resubscribe.
- Batch mutations publish change events only after the whole batch
  commits; rolled-back batches publish nothing, whether the rollback ran
  inline or in recovery.

Consumers that need a complete view should periodically reconcile via
`ListChanges` from their last stored token rather than relying solely
on the live stream.

## Changeset log

`ListChangesets` / `GetChangeset` are the durable counterpart to the change
feed: one record per mutation RPC, carrying the authenticated author, the
entities that actually moved, and the branch it happened on. These records
do not expire and are not dropped when a change-feed publish fails.

Each change event carries the `changeset_id` and `author` of the changeset
that produced it, so a feed consumer can attribute a change without a
second call. A non-empty `changeset_id` is not proof the changeset row
exists yet: the feed is a transport with retention, the changeset bucket is
the record. For a baseline batch the row is guaranteed to land eventually,
because batch recovery appends any changeset the RPC did not; such a row's
message ends in `completed by batch recovery`.

## Entity history and revert

`GetEntityHistory` is `git log <entity>`: every changeset that touched one
key, newest first, each row carrying the entity's content before and after
that write. The pre-images live in their own unbounded KV bucket because
the entities bucket is `history=1` (the etag *is* its KV revision), so a
write destroys the content it replaced.

`RevertChangeset` writes the inverse of a changeset as a new changeset.
Nothing is erased and nothing rewinds; the undo goes through the ordinary
batch write path, so it validates, publishes change events, updates the
mirror, and honours baseline protection. It refuses with
`FAILED_PRECONDITION` when a target has been edited since (whole-entity
granularity means any later edit is a conflict) or when the original write
carried no changeset attribution and therefore no pre-image.

See [`docs/explanation/changesets.md`](../../../docs/explanation/changesets.md)
for the full model, including what is deliberately not recorded and the
crash window between a write committing and its changeset landing.

## Library modules

- `service`: the tonic service impl. `EventModelServiceImpl` is the
  type to instantiate.
- `validation`: `validate_model` and `validate_scenarios`. Each rule
  is delimited by a `// ===== PASS: <name> =====` banner; the inline
  pass structure is intentional so rule ordering stays visible during
  review.
- `graph`: reverse-reference index, closures, impact analysis.
- `analysis`: flow analysis used by the `InferDataFlow` and
  `CheckInformationCompleteness` RPCs.
- `llm` / `llm_analysis`: LLM provider abstraction (anthropic-only
  today) and the analyzer that drives the LLM-backed RPCs.
- `git_mirror`: optional write-through to a git working tree, useful
  for auditing changes by commit.
- `search`: in-RAM tantivy index for `Search`.
- `telemetry`: OTLP + Prometheus wiring.

## Tests

Unit tests live inside their module under `#[cfg(test)]`. Integration
tests under `tests/` use an ephemeral NATS JetStream instance via `trogon-atlas-testsupport`.
