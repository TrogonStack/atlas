# Failure reasons

Every write RPC can fail. This page is the registry of the stable,
machine-readable identifiers a caller gets back when one does, independent
of the human-readable `message()`/`message` prose, which is free to change
without notice.

Two separate mechanisms carry these identifiers, because they sit at two
different layers of the wire protocol:

- A **gRPC `ErrorInfo` reason**, attached to the `tonic::Status` thrown by
  `put_entity`, `delete_entity`, and any other RPC that fails outright.
  Domain `trogonatlas.api.eventmodel.v1alpha1`.
- A **`BatchMutateResponse.failure[].code`**, a plain string field on a
  `batch_mutate` response that came back with `status: FAILED`. A failed
  batch op is not a transport error: the RPC still returns `Ok`, carrying
  the failure as data, so it needs its own code rather than reusing
  `ErrorInfo`.

`trogon-atlas-client`'s `Failure::classify` (see `rsworkspace/crates/trogon-atlas-client/src/failure.rs`)
is the single place that reads either mechanism and turns it into a
`FailureCategory`. Every surface (trogon-atlas, the MCP adapter) builds on that
instead of re-deriving category from code or reason on its own.

## `ErrorInfo` reasons (domain `trogonatlas.api.eventmodel.v1alpha1`)

| Reason | gRPC code | Category | Metadata |
| --- | --- | --- | --- |
| `NOT_FOUND` | `NOT_FOUND` | `not_found` | none |
| `ALREADY_EXISTS` | `ALREADY_EXISTS` | `precondition` | none |
| `STALE_PRECONDITION` | `ABORTED` | `stale_state` | none |
| `VALIDATION_FAILED` | `INVALID_ARGUMENT` | `validation` | none |
| `UNAVAILABLE` | `UNAVAILABLE` | `unavailable` | none |
| `PARTIAL_APPLY` | `UNAVAILABLE` | `partial_apply` | `journal`, the batch journal id, when the apply was journaled and so is recoverable; absent when it was not (branch batches, branch merges) and the affected keys need manual repair |
| `INTERNAL` | `INTERNAL` | `internal` | none |
| `OPERATION_IN_PROGRESS` | `UNAVAILABLE` | `unavailable` | none |
| `OPERATION_ID_REUSED` | `ALREADY_EXISTS` | `operation_conflict` | none |
| `NOT_WRITER` | `UNAVAILABLE` | `unavailable` | `role`, this process's configured role (`writer`, `standby`, or `reader`); `epoch`, the lease epoch it last observed |
| `CURSOR_REJECTED` | `INVALID_ARGUMENT` | `validation` | none |
| `BREAKING_CHANGE` | `FAILED_PRECONDITION` | `precondition` | none |

`PARTIAL_APPLY` and `UNAVAILABLE` share the `UNAVAILABLE` gRPC code on
purpose: both mean "retry later", but only `PARTIAL_APPLY` means a batch
left entities in an intermediate state that a recovery pass (or the
journal named in `metadata.journal`) needs to resolve. A client that only
looked at the gRPC code could not tell them apart; the reason is what
disambiguates.

`OPERATION_IN_PROGRESS` and `OPERATION_ID_REUSED` are the two reasons an
`operation_id` can fail with, and each carries a `next_action` other than
its category's default: `OPERATION_IN_PROGRESS` means another call already
owns the id and has not settled it yet, so the next action is
`check_operation` (poll `GetOperation` rather than retry the mutation
itself) instead of `unavailable`'s usual `retry`; `OPERATION_ID_REUSED`
means the id is already bound to a different request, so the next action
is `use_new_operation_id` instead of `operation_conflict`'s usual
`replan`.

`NOT_WRITER` means this server process does not currently hold the single
writer lease (see `docs/explanation/single-writer.md`), so the mutation
never ran. Retrying against the same process will keep failing until it
becomes the writer; a caller load-balancing across replicas should retry
against a different one.

`CURSOR_REJECTED` is `ListChanges`' `since_token` carrying the sealed
cursor prefix but failing to open: tampered, minted for a different
principal, or sealed under a key this process does not hold. Its next
action is `restart_listing` instead of `validation`'s usual `fix_input`,
since there is no input to fix -- the caller restarts the poll with an
empty `since_token` rather than retry the same one. A `since_token` that
is not even sealed (garbage, or a foreign format) still gets the plain
`invalid_argument("invalid since_token")` status with no `ErrorInfo`, the
same as before this reason existed.

`BREAKING_CHANGE` is a TypeLibrary write that would make payloads stored
against the previous version decode differently. Each change is listed as a
`PreconditionFailure` violation of type `TYPE_LIBRARY_COMPATIBILITY`. Retrying
cannot succeed: publish the new shape under a new package and supersede the
library, as `docs/how-to/register-a-type-library.md` describes. A delete that
would strand a library's dependents or the entity schemas using its types is
refused with `FAILED_PRECONDITION` and violations of type
`TYPE_LIBRARY_DEPENDENCY` or `TYPE_LIBRARY_SCHEMA`, one per referrer.

A status without an `ErrorInfo` at all (most `INVALID_ARGUMENT` statuses
raised directly by request validation, for instance) still classifies
correctly from its gRPC code alone -- `ErrorInfo` exists to disambiguate
within a code, not to replace the code.

## `BatchMutateResponse.failure[].code`

A batch op failure is reported as data on an otherwise-successful RPC, so
it carries its own code rather than an `ErrorInfo` reason:

| Code | Meaning |
| --- | --- |
| `STALE_PRECONDITION` | The op was planned against a specific revision (an `if_match` etag, or `create_only` against an entity that already exists) and that revision has moved. `BatchMutateResponse.precondition_failure` names the entity and both revisions. |
| `ENTITY_NOT_FOUND` | The op targeted an entity that does not exist, with no revision having been planned against it (an unconditional put's target, or a delete with no `if_match`). |
| `ENTITY_REFERENCED` | A delete op (the default `mode=FAIL_IF_REFERENCED`, or `mode` left unset) targeted an entity something else still references. Same refusal `delete_entity` gives a standalone caller; pass `mode=FORCE` or repair the referrers first. |
| `INVALID_OP` | The op itself was structurally invalid in a way request-time validation did not already reject (for example, violating a store-level invariant). |
| `BATCH_OP_FAILED` | Fallback for any other store failure (a backend error, a lost change event, corrupt stored data). Not actionable by the caller beyond retrying or reporting it; the detail stays in the server log, not the response. |

## CLI exit codes

`trogon-atlas`'s exit code is a direct function of the failure category (see
`docs/reference/trogon-atlas-exit-codes.md`); this page is what it reads the
category *from*.
