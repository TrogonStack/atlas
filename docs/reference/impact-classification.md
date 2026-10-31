# Impact classification

`ClassifyImpact` answers "if dependents of this exact version moved to that
exact version, which of them break and which only need their pinned
reference bumped?" It is a read RPC (Reader role) gated by
`GetServerInfo.features.impact_classification`. The
[how-to guide](../how-to/assess-change-impact.md) shows it in use.

## Surfaces

| Surface | Call |
| --- | --- |
| gRPC | `EventModelService.ClassifyImpact(ClassifyImpactRequest)` |
| Rust client | `trogon_atlas_client::impact::classify(client, current, candidate, max_depth)` follows every page and returns an `ImpactClassification`. |
| CLI | `trogon-atlas classify-impact <current> <candidate> [--max-depth N]`, both addresses exact (`event/shop/order-placed@1`). |
| MCP | `classify_impact` with `current` and `candidate` as `{kind, namespace, slug, version}` and optional `max_depth`. Returns proto3 JSON. |

All of them honor the branch selected by `x-trogon-atlas-branch` (CLI
`--branch`), for both entity reads and the dependent traversal.

## Request

| Field | Meaning |
| --- | --- |
| `current` | Exact `EntityRef` dependents pin today. Required. |
| `candidate` | Exact `EntityRef` that would replace it. Required, same kind, different `Id`; otherwise `INVALID_ARGUMENT`. Either one outside the caller's view is `NOT_FOUND`. |
| `max_depth` | Referrer hops from `current`. `0` and anything above the server maximum (16) mean the server maximum. |
| `page_size`, `page_token` | Page the `dependents` list. Every other response field is repeated whole on each page. |

## Dependents

A dependent is an entity that reaches `current` through incoming references
only: something that points at `current`, something that points at that, and
so on. An outgoing reference (the swimlane an event sits in, the source
events a read model names) never makes its target a dependent. `candidate` is
never a dependent, even when its `supersedes` points at `current`. Only
references pinned to `current` exactly count; a referrer already pinned to
another version is not a dependent.

Dependents are ordered by depth, then kind, namespace, slug and version.

## Snapshot

Each call reads the model once and computes everything from that one read:
the inputs, the dependents, the hidden-dependent check and `snapshot_id`. It
takes no lock, so writes proceed while it runs, and a concurrent write shows
up as a different `snapshot_id` on the next call rather than as an error.

Each page is its own call. Pages whose `snapshot_id` differ describe different
states and must not be combined. The Rust client, CLI and MCP tool report
that case as `clientSnapshotConsistent: false` and never read it as
`bump_only`.

Default chosen for review: a missing `current` or `candidate` is `NOT_FOUND`,
because there is nothing to compare. A dependent that is missing, hidden or
cannot be resolved is never an error; it makes the answer `UNKNOWN` with the
reason in `incompleteness`.

## Response

| Field | Meaning |
| --- | --- |
| `snapshot_id` | Content identity of the caller's view the answer was computed from: the `GetSnapshotId` value with no namespace filter, on the same branch. |
| `ops` | `DiffEntities(current, candidate)`, the vocabulary every causal op is drawn from. |
| `verdict` | The worst dependent classification, raised to `UNKNOWN` when `incompleteness` is non-empty. `BUMP_ONLY` when complete and nothing breaks, including when nothing depends on `current`. |
| `incompleteness` | Why the dependent set may be partial. See below. |
| `uncovered_changes` | Parts of the entity that differ but that the typed diff does not compare, e.g. `swimlane`, `key`, `schema.metadata`. |
| `counts` | `breaking`, `bump_only` and `unknown` over every dependent, not just the page. |
| `dependents` | One `DependentImpact` per dependent. |

Each `DependentImpact` carries:

| Field | Meaning |
| --- | --- |
| `dependent`, `depth` | The entity and its shortest referrer-only distance. |
| `classification` | The worst of its occurrences and of what it inherits through `through`. |
| `occurrences` | One `ConsumedOccurrence` per reference straight to `current`, in reference order. A repeated field that names `current` twice yields two occurrences, each with its own `from_field` and pinned `to`. |
| `through` | References to other dependents, each with that dependent's classification. |
| `unknown_reasons` | Every reason, only when the classification is `UNKNOWN`. |

Each `ConsumedOccurrence` carries the `reference`, its own `classification`,
`consumed_paths` (root paths in diff vocabulary, such as
`schema.fields[amount]`) and the `causal_ops` that make it `BREAKING` or
`UNKNOWN`.

## Classification rules

Classification fails closed: only what the server can prove compatible is
`BUMP_ONLY`.

| Change to a consumed field | Classification |
| --- | --- |
| Removed, type changed, `repeated` or `optional` changed | `BREAKING` |
| `doc` changed | No effect |
| `metadata` changed | `UNKNOWN` (`UNCOVERED_CHANGE`) |
| Field added | Depends on the reference site, below |
| A change in `uncovered_changes`, or any op the rules do not recognize | `UNKNOWN` (`UNCOVERED_CHANGE`) |

`title`, `doc` and `supersedes` changes never affect a dependent.

| Root | Reference site | Consumed fields | Added field breaks when |
| --- | --- | --- | --- |
| Event | CommandSlice `emitted_events[i]` | Every event field | The slice's command does not carry it and it is not marked derived (`EVENT_FIELD_UNSOURCED`). |
| Event | ReadModelSlice `source_events[i]` | Event fields the target read model has, plus its key | Always: the projection must project it (`RM_SOURCE_EVENT_FIELD_NOT_PROJECTED`). |
| Event | ReadModel `source_events[i]` | Event fields the read model has, plus its key | Always, for the same reason. |
| Command | CommandSlice `command` | Command fields the slice's emitted events carry | Never. |
| Command | AutomationSlice `emitted_command` | Every command field | No source read model carries it and it is not marked derived (`AUTOMATION_COMMAND_FIELD_UNSOURCED`). |
| ReadModel | ReadModelSlice `read_model` | Every read model field | No source event carries it and it is not marked derived (`READ_MODEL_FIELD_UNSOURCED`). |
| ReadModel | AutomationSlice `source_read_models[i]` | Read model fields the emitted command has | Never. |
| Any field-bearing root | A typed field reference (`...fields[i]...ref`) | Every field | It is required. |
| Any | A scenario example with a `google.protobuf.Struct` payload | The payload's keys | It is required. |
| Any | A scenario example without a payload, `supersedes`, or a membership-only referrer (EventModel, Storyboard, Component, Tracker, Term, Ambiguity, Swimlane, strategic and responsibility kinds, Decision, Document) | Identity only | Never. |

Any other site has no consumption model. It is `BUMP_ONLY` when the versions
differ in documentation alone and `UNKNOWN` otherwise.

| `unknown_reasons` value | Cause |
| --- | --- |
| `NO_CONSUMPTION_MODEL` | No rule above covers the site. |
| `OPAQUE_PAYLOAD` | A scenario example payload is not a `Struct`. |
| `OPAQUE_ROOT_SCHEMA` | The root's schema is a type library message, not an inline `Schema`. |
| `CONSUMER_CONTEXT_MISSING` | The entity that defines the site's consumption (a slice's command, events or read model) is outside the caller's view or has no inline schema. |
| `UNCOVERED_CHANGE` | See the first table. |
| `UNSUPPORTED_ROOT_KIND` | A field-consuming site on a root other than Event, Command, ReadModel or Schema. |
| `TRANSITIVE_THROUGH_AFFECTED` | The dependent references a dependent that is `BREAKING` or `UNKNOWN`. A dependent that references only `BUMP_ONLY` dependents stays `BUMP_ONLY`. |
| `TRAVERSAL_INCOMPLETE` | The response is incomplete, so no dependent is reported `BUMP_ONLY`. |

## Incompleteness

| Value | Cause |
| --- | --- |
| `SNAPSHOT_TRUNCATED` | The server's entity snapshot hit its cap. |
| `DEPTH_LIMITED` | A dependent lies beyond the effective `max_depth`. |
| `HIDDEN_DEPENDENTS` | A dependent lies outside the caller's view. The response names none of them; the flag discloses only that they exist. |
| `UNRESOLVED_REFERRERS` | A reference into the walk names a referrer the server cannot resolve to an entity, so what lies behind it is unknown. |

Any incompleteness turns every `BUMP_ONLY` dependent into `UNKNOWN` with
`TRAVERSAL_INCOMPLETE` and makes the verdict at least `UNKNOWN`. A `BREAKING`
dependent stays `BREAKING`.

Default chosen for review: hidden dependents make the result `UNKNOWN` rather
than silently omitting them, because a dependent the caller cannot see still
breaks when the change lands.

## CLI exit codes and JSON

| Verdict | Exit |
| --- | --- |
| `bump_only` | `0` |
| `breaking` | `1` |
| `unknown` | `12` |

Under `--format json`, `data` is the aggregated response in proto3 JSON plus
`clientVerdict` (`bump_only`, `breaking` or `unknown`), which is the value
the exit code follows, and `clientSnapshotConsistent`, false when pages came
from different snapshots. The client reads an unrecognized verdict, or a
`BUMP_ONLY` verdict beside any incompleteness or mixed snapshots, as
`unknown`. The MCP tool
returns the same JSON. Against a server that does not advertise
`impact_classification`, every surface fails with `incompatible` (exit `8`)
before calling the RPC.

## Not covered yet

- Field consumption by UiSlice, AutomationSlice processors, Specification,
  DataProduct and architecture resources. Those sites report
  `NO_CONSUMPTION_MODEL`.
- Renames. The typed diff reports a renamed field as a removal and an
  addition, so a consumer of the old name is `BREAKING`.
- Roots whose schema is a type library message, and Schema entities reached
  through a `SchemaRef`.
- The exact truncation and depth signals planned for `GetImpact`. The
  classifier keeps its completeness inputs separate so it can adopt them.
- A precondition that binds a later write to `snapshot_id`. Compare it with
  `GetSnapshotId` before landing; a write still uses its own `if_match`.
- An end-to-end test that produces `UNRESOLVED_REFERRERS`: every referrer the
  store accepts today resolves, so only the resolution rule is unit tested.
- A numeric risk score. Classification is categorical on purpose.
