# Backing report

`GetBackingReport` (MCP `get_backing_report`, CLI `trogon-atlas
backing-report`) counts which visible entities carry recorded evidence and
which do not. It reports evidence presence, never whether the evidence is
correct. For why the report is shaped this way, see
[backing evidence](../explanation/backing-evidence.md).

The request and response messages live in
[`backing.proto`](../../proto/trogonatlas/api/eventmodel/v1alpha1/backing.proto).
The RPC needs the Reader role.

## Request

| Field | Meaning |
| --- | --- |
| `namespaces` | Namespace ids whose entities count as subjects. Empty means every namespace the caller can see. Each one must be visible to the caller, otherwise the call fails with `NOT_FOUND`. |
| `include_unbacked_findings` | When true, the response carries one `ENTITY_UNBACKED` finding per unbacked subject. Off by default. |

The branch comes from the `x-trogon-atlas-branch` header, the same as for
every other read. A branch the caller may not read fails with `NOT_FOUND`.

## Subjects

A subject is one exact entity version in the requested namespaces whose
kind is neither Decision nor Document. Each version is counted on its own:
evidence naming version 1 does not back version 2. A subject is counted
once in `total`, however many pieces of evidence name it.

## Evidence categories

| Category | Present when | Unknown when |
| --- | --- | --- |
| `decision` | An `ACCEPTED` Decision lists the subject's exact `EntityRef` among its subjects. `PROPOSED`, `REJECTED`, `WITHDRAWN` and `SUPERSEDED` Decisions never count. | No accepted Decision names it, and a Decision with a status this server does not recognize does; or the view is restricted or truncated. |
| `document` | A Document lists the subject's exact `EntityRef` among its subjects. | No Document names it, and the view is restricted or truncated. |
| `code_ref` | The subject's own metadata holds a `CodeRefAnnotation` with both `repo` and `path` set. Whether that file still exists is not checked. | A `CodeRefAnnotation` is present but cannot be decoded or lacks `repo` or `path`. |

Evidence is read from the whole visible snapshot, so a Decision in another
namespace still backs a subject in the requested ones. A selection of a
connection, a contract member or a data port does not back the entity that
owns it: only an exact entity selection counts.

## Subject state

| State | Rule |
| --- | --- |
| backed | At least one category is present. |
| unknown | No category is present, and at least one is unknown. |
| unbacked | Every category is absent. |

## Response

| Field | Meaning |
| --- | --- |
| `snapshot_id` | The id `GetSnapshotId` returns for the same branch and caller with an empty namespace list. Cite it to pin the state the report describes. |
| `snapshot_truncated` | The snapshot hit the server's entity cap. |
| `branch` | The branch the report was computed on. Empty for the default view. |
| `visibility.restricted` | The caller sees only some namespaces. |
| `visibility.authorized_namespaces` | The namespace ids a restricted caller sees. Empty when unrestricted. |
| `namespaces` | The requested subject scope, sorted. |
| `totals` | `total`, `backed`, `unbacked` and `unknown` subjects, the present, absent and unknown count per category, and `unbacked_rate`. |
| `kinds` | The same totals for each subject kind that has at least one subject. |
| `unresolved_declarations` | Subject selections on accepted Decisions and on Documents whose exact target is not in the visible snapshot but whose namespace is in scope. |
| `incomplete_reasons` | Why some subjects could not be settled. |
| `findings` | `ENTITY_UNBACKED` findings, sorted by subject, when requested. |

Category counts overlap: a subject with both a Decision and a Document adds
one to each category's `present` but only one to `backed`.

`unbacked_rate` is `unbacked / total`. It is set only when `total` is above
zero, `unknown` is zero and the snapshot is not truncated. An empty scope
has no rate, not a rate of zero.

### Incomplete reasons

| Reason | When |
| --- | --- |
| `SNAPSHOT_TRUNCATED` | Always reported when the snapshot is truncated. |
| `VISIBILITY_RESTRICTED` | A restricted caller has a subject whose Decision or Document evidence may sit in a namespace it cannot see. |
| `UNRESOLVED_CODE_REF` | A subject has an incomplete or unreadable `CodeRefAnnotation`. |
| `UNRECOGNIZED_DECISION_STATUS` | A Decision with an unrecognized status names a subject. |

## `ENTITY_UNBACKED`

An Info rule in the `backing` category of `ListValidationRules`. It is
emitted only by this report, only when `include_unbacked_findings` is true,
and only for subjects settled as unbacked. An unknown subject never gets
the finding. No validation entry point emits it.

Default chosen for review: the rule is off unless the caller asks for it,
because a model with no backing yet is not wrong, and an always-on Info
finding per entity would drown the findings that are.

## CLI

```sh
trogon-atlas backing-report [--namespace <id>]... [--unbacked-findings]
```

Text output prints the totals, one line per kind, each incomplete reason
and each finding. `--format json` puts the proto3 JSON of the response in
`data`. The command exits `0` when every subject was settled over an
untruncated snapshot and `11` otherwise; see
[exit codes](trogon-atlas-exit-codes.md).

## MCP

`get_backing_report` takes `namespaces`, `include_unbacked_findings` and
`json`. It returns the response as `binpb_base64`, or as proto3 JSON under
`json` when `json` is true.
