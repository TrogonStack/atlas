# Invariant gates

Wire contract for
[`InvariantAnnotation`](../../proto/trogonatlas/eventmodel/v1alpha1/annotations.proto)
and
[`InvariantCheck`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto).
For the reasoning behind it, see the
[invariant gates explanation](../explanation/invariant-gates.md).

## Declaring an invariant

Add an `InvariantAnnotation` to an `EventModel`'s `metadata` as a
`google.protobuf.Any` with type URL
`type.googleapis.com/trogonatlas.eventmodel.v1alpha1.InvariantAnnotation`.

| Field | Meaning |
| --- | --- |
| `name` | Required. Unique among the model's invariants |
| `doc` | Optional. Why the invariant exists |
| `forbidden_codes` | Model rule codes the model must never gain, for example `MEMBER_NOT_FOUND` |
| `forbidden_severity` | Any model finding at or above this severity is forbidden. Unspecified forbids nothing by severity |
| `on_unevaluable` | `UNEVALUABLE_REFUSE` (the default when unspecified) or `UNEVALUABLE_ADVISE` |

A write of the model is refused with `INVALID_ARGUMENT` when a declaration
has no name, forbids nothing, repeats a name, or cannot be decoded.

## Gated routes

| Route | Response carrying `invariant_check` |
| --- | --- |
| `PutEntity` | `PutEntityResponse` |
| `DeleteEntity` | `DeleteEntityResponse` |
| `BatchMutate` | `BatchMutateResponse` |
| `DeleteByQuery` | `DeleteByQueryResponse` |
| `RetargetReferences` | `RetargetReferencesResponse` |
| `RevertChangeset` | `RevertChangesetResponse` |
| `MergeBranch` | `MergeBranchResponse` |

`invariant_check` is unset when the scope declares no invariant. The dry
run of each route returns the same check, or the same refusal, as its real
write.

## `InvariantCheck`

| Field | Meaning |
| --- | --- |
| `outcomes` | One per declared invariant, in carrier then name order |
| `snapshot_id` | Snapshot the verdict was computed against, as `GetSnapshotId` reports it. Empty when the snapshot has no content identity |
| `inputs` | Carriers and touched entities, each with the etag it had in that snapshot. An empty etag means the entity was absent |
| `complete` | True when no outcome is unevaluable and none is redacted |

## `InvariantOutcome`

| Field | Meaning |
| --- | --- |
| `carrier` | The event model declaring the invariant. Unset when `redacted` |
| `name` | The invariant's name. Unset when `redacted` |
| `status` | See the statuses table |
| `violations` | Forbidden findings the write introduces. `STATUS_VIOLATED` only |
| `reason` | Why the invariant could not be evaluated. `STATUS_UNEVALUABLE` only |
| `missing` | Members the world lacks. `REASON_MISSING_INPUTS` only |
| `advisory` | The declaration lets an unevaluable outcome through |
| `preexisting` | Forbidden findings present both before and after the write |
| `redacted` | The carrier lives in a namespace the caller cannot read |

## Statuses

| Status | Refuses the write |
| --- | --- |
| `STATUS_HOLDS` | no |
| `STATUS_VIOLATED` | yes |
| `STATUS_UNEVALUABLE` | yes, unless `advisory` |
| `STATUS_ALREADY_VIOLATED` | no |

## Unevaluable reasons

| Reason | Meaning |
| --- | --- |
| `REASON_MISSING_INPUTS` | A member of the carrier is missing from the world the write produces |
| `REASON_SNAPSHOT_TRUNCATED` | The snapshot hit the entity cap. When the scope holds too many models to enumerate its carriers, one outcome with no carrier or name stands for all of them |
| `REASON_BUDGET_EXHAUSTED` | The scope declares more invariants than one evaluation checks (64) |
| `REASON_MALFORMED_DECLARATION` | A stored declaration cannot be decoded or forbids nothing |

## Refusals

A refused write returns one of the `INVARIANT_*` reasons listed in
[failure reasons](failure-reasons.md). Violated and unevaluable refusals
carry this `ErrorInfo` metadata:

| Key | Value |
| --- | --- |
| `invariant` | The invariant's name, empty when redacted |
| `carrier` | `namespace/slug@version` of the carrier, empty when redacted |
| `status` | `VIOLATED` or `UNEVALUABLE` |
| `reason` | The unevaluable reason without its `REASON_` prefix, empty otherwise |
| `snapshot_id` | Snapshot the verdict was computed against |
| `redacted` | `true` when the carrier is hidden from the caller |

A violated refusal also carries a `PreconditionFailure` with one violation
of type `INVARIANT` per introduced finding, described as `CODE: message`.

`INVARIANT_STALE` means a write landed in the judged scope between the
verdict and the apply. Nothing was applied; read again and retry.
