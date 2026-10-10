# trogon-atlas-cli (`trogon-atlas`)

kubectl-style CLI for the trogon-atlas server: declarative YAML manifests in,
one atomic `BatchMutate` out. Replaces the per-project `.mjs` push scripts
(node → shell → `docker compose run` → MCP stdio → gRPC) with a direct gRPC
client.

The schema stays owned by the proto: a manifest's `spec` IS the proto3-JSON
body of the entity message, validated against the embedded descriptor pool.
`trogon-atlas` never invents fields; unknown spec keys are rejected at parse time.

The manifest/apply/export/client logic lives in the shared
`trogon-atlas-client` crate (`../trogon-atlas-client`); this crate is a thin
clap CLI on top of it, so the MCP adapter can build on the same semantics.

## Usage

```sh
trogon-atlas apply  -f manifests/                 # create + configure + skip unchanged
trogon-atlas apply  -f manifests/ --dry-run       # server-side validation, nothing persisted
trogon-atlas diff   -f manifests/                 # unified diff vs live state, exit 1 on drift
trogon-atlas export --namespace shop -o manifests # live store -> manifest YAML (one file per kind)
trogon-atlas export --namespace shop -o manifests --layout slices # one self-contained file per slice
trogon-atlas openslo export --namespace shop -o shop.openslo.yaml --bindings openslo-bindings.yaml # live store -> OpenSLO v1 YAML
trogon-atlas fmt manifests/                       # canonical YAML to stdout (single file) or report via --check
trogon-atlas fmt manifests/ --write               # rewrite files in place
trogon-atlas fmt manifests/ --check               # CI gate: exit 1 and list files that would change
```

Connection flags (or environment):

| Flag | Env | Default |
|---|---|---|
| `--endpoint` | `TROGON_ATLAS_ENDPOINT` | `http://127.0.0.1:50069` |
| `--auth-token` | `TROGON_ATLAS_AUTH_TOKEN` | none (anonymous) |
| `--rpc-timeout-secs` | `TROGON_ATLAS_RPC_TIMEOUT_SECS` | `30` |

## Structured output

`--format json` (or `TROGON_ATLAS_FORMAT=json`) makes stdout carry exactly one
`CommandOutcome` document (schema `trogon-atlas.result.v1`) instead of
human-readable text, so an agent driving `trogon-atlas` gets one value to parse
per command instead of scraping stdout. Not `-o`: `export` already uses
that short flag for its output directory.

```sh
trogon-atlas --format json branch list
# {"schema":"trogon-atlas.result.v1","ok":true,"data":{"branches":[...]}}

trogon-atlas --format json apply -f manifests/nonexistent.yaml
# {"schema":"trogon-atlas.result.v1","ok":false,"failure":{"category":"internal", ...}}
```

`text` (the default) is unchanged: progress and results print the same as
always, and nothing moves to stderr that was not already there.

The exit code is always a direct function of the failure category,
regardless of format; see `docs/reference/trogon-atlas-exit-codes.md`.

## Manifest format

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice            # any entity kind; short forms accepted (read-model, readModel, read_model)
metadata:
  namespace: shop
  name: place-order-slice     # the slug
  version: 1                  # optional; breaking-change counter, defaults to 1
spec:                         # proto3-JSON body of the entity message (minus id)
  title: Place order
  command: place-order        # ref/edge shorthand
  emittedEvents: [order.placed]
```

Ergonomics, all schema-derived (see `src/manifest.rs`):

- Wherever the proto expects an `Id`, a `*Ref`, an `*Edge`, or a scenario
  `*Example`, a bare string works: `slug`, `ns/slug`, or `ns/slug@version`.
  Namespace defaults to the manifest's namespace, version to 1.
- `EntityKind` enum values accept short forms (`storyboard`) in addition to
  proto names (`ENTITY_KIND_STORYBOARD`).
- Scenario `id` fields are plain strings and are left untouched; the
  normalizer walks the proto schema, not key names.

Apply semantics:

- `created`: no entity at `(kind, namespace, slug, version)`.
- `configured`: entity exists but differs (byte-level compare after
  stripping server-owned `system` metadata); overwritten in place. Remember
  `version` is a breaking-change counter: bump it in `metadata.version` for
  breaking changes instead of editing in place.
- `unchanged`: skipped entirely, so re-applying a manifest set is a no-op
  (no spurious git-mirror commits). This is enforced twice: trogon-atlas skips
  unchanged entities client-side, and the SERVER also skips semantically
  identical puts (`PutEntityResponse.no_op`), so no client, including raw
  gRPC or MCP agents, can churn revisions with equal content. A deliberate
  rewrite needs `PutEntityRequest.force`.
- The whole change set is one `BatchMutate`: server-side validation rejects
  the batch atomically, and `--dry-run` maps to `validate_only`.
- Servers may require `--branch` for writes (`--protect-baseline`): pass it
  to apply on a branch instead of baseline.

Deletion is intentionally out of scope for `apply` (no prune yet): use the
server's `DeleteEntity`/`DeleteByQuery` RPCs. Prune needs an ownership
marker (an apply-set annotation) before it can be safe.

See `examples/manifests/shop.yaml` for a complete small model.

## Export layouts

`trogon-atlas export --namespace <ns> -o <dir>` defaults to `--layout single` (one
file per entity kind). `--layout slices` instead writes one file per slice
(`CommandSlice`, `ReadModelSlice`, `AutomationSlice`, `UiSlice`), named
`<namespace>/<slug>.yaml` (`@<version>` suffix when `version != 1`), each
bundling the slice with every entity it references (commands, events,
read models, personas, swimlanes, UIs, schemas, and so on) so a coding
agent can read one file and have the whole slice self-contained. Entities
shared by more than one slice are duplicated byte-for-byte across their
files; `trogon-atlas apply` accepts identical duplicate documents across files as
a no-op, so applying the whole directory back is safe. Anything not
referenced by any slice (storyboards, event models, orphaned entities)
lands in `unsliced.yaml`.

With `--layout slices`, `-o` refuses a non-empty directory unless `--force`; `--force` only
overwrites the files this export writes and never deletes files left over
from a previous export.

## OpenSLO export

`trogon-atlas openslo export --namespace <ns> -o <file> --bindings <openslo-bindings.yaml>`
renders a namespace's `ServiceLevelIndicator`, `ServiceLevelObjective`,
`AlertPolicy`, and `AlertNotificationTarget` entities, plus the `Component`s
they reference, as a vendor-neutral [OpenSLO v1](https://github.com/OpenSLO/OpenSLO)
multi-document YAML stream (see `../trogon-atlas-client/src/openslo.rs`).
`Connection` and `Commitment` have no OpenSLO field, so they travel whole,
as JSON, under `metadata.annotations["trogonatlas.eventmodel.v1alpha1/connection"]` and
`["trogonatlas.eventmodel.v1alpha1/commitment"]` (the proto package this crate owns, not a
domain the project does not own).

A `LatencyThreshold.value` renders in fractional seconds (OpenSLO's unit
for this field), so a 300ms threshold renders as `value: 0.3`.

OpenSLO's `metricSource` needs a vendor query the model deliberately does
not carry (a `Signal` is a logical pointer, not a vendor query). The
`--bindings` file supplies it, keyed by `Signal`, with one independent
template per query role the signal's measure needs:

```yaml
bindings:
  - signal:
      source: telemetry        # or: source: eventTimestamps
      kind: span                # span | metric; telemetry signals only
      name: checkout.duration
    dataSource:
      name: tracing-backend
      type: Datadog
      connectionDetails:
        site: datadoghq.com
    metricSource:
      threshold:
        query: "p95:trace.checkout.duration{service:checkout}"
  - signal:
      source: eventTimestamps
    dataSource:
      name: event-store
      type: EventStore
      connectionDetails:
        endpoint: https://events.internal
    metricSource:
      good:
        query: "events in {{events}}"
      total:
        query: "events in {{events}},order-rejected"
```

Each role (`threshold`, `good`, `bad`, `total`) is its own template, not
shared text substituted differently, because a ratio's numerator and
denominator are usually different queries, not the same query with a
different window. OpenSLO allows a `ratioMetric` to carry `good`+`total`
or `bad`+`total`, never all three, so `good` and `bad` are separate roles
rather than one "numerator" role. The converter selects the roles each
measure needs:

| Measure | Roles needed | Tokens available |
|---|---|---|
| `Latency` | `threshold` | none |
| `OutcomeRatio` | `good`+`total` (or `bad`+`total` if `good` is empty) | `{{events}}` (comma-joined event slugs) |
| `Completion` | `good`+`total` | `good`: `{{within}}` (duration-shorthand); `total`: none |

A role the bound signal's measure needs but the binding does not supply is
always an error naming the indicator and the missing role, even when
`--allow-placeholder-metrics` is set: the flag only covers a `Signal`
with no binding entry at all, so a placeholder never silently papers over
an incomplete binding. After substitution, any template that still
contains an unresolved `{{token}}` is also an error, naming the indicator
and the token; an unresolved token is never emitted into the rendered
YAML. Pass `--allow-placeholder-metrics` to emit a documented placeholder
(`metricSource.type: Unbound`) for a fully-unbound signal instead of
failing the export, so a draft export can still be produced before every
signal is wired up.

## Formatting

`trogon-atlas fmt` canonicalizes manifest YAML offline: no server connection, same
manifest parser and the same canonical emission `export` uses (see
`../trogon-atlas-client/src/fmt.rs`). It is semantics-preserving
(`parse(fmt(x)) == parse(x)`) and idempotent (`fmt(fmt(x)) == fmt(x)`), and
keeps multi-document files multi-document, in their original order.

YAML comments cannot survive the proto round trip, so a file carrying one
is refused with a clear error unless `--force`; `shop.yaml` above is a
hand-commented example and is deliberately left out of any canonicalization
pass for that reason.

```sh
trogon-atlas fmt manifests/foo.yaml          # print canonical YAML to stdout (single file only)
trogon-atlas fmt manifests/ --write          # rewrite every changed file in place
trogon-atlas fmt manifests/ --check          # CI gate: list files that would change, exit 1 if any would
trogon-atlas fmt manifests/foo.yaml --force  # canonicalize anyway, dropping comments
```

## Editor completion and validation

`trogon-atlas schema` prints a JSON Schema (draft 2020-12) for the manifest
document, generated straight from the proto descriptors (see
`../trogon-atlas-client/src/manifest_schema.rs`); it accepts the same
envelope, per-kind spec shape, and ref/id shorthand as the parser in
`src/manifest.rs`, and is regenerated into `../trogon-atlas-client/manifest.schema.json`
whenever the proto changes.

Point yaml-language-server at it, either per-file:

```yaml
# yaml-language-server: $schema=../trogon-atlas-client/manifest.schema.json
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
```

or repo-wide, with a `yaml.schemas` setting mapping the committed schema
path to your manifest file glob (e.g. `manifests/**/*.yaml`).
