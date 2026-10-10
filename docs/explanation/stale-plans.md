# Stale plans: acting only on the state you read

Agents and people work against the same store at the same time. Every write
they make is a decision taken from something they read earlier: `trogon-atlas apply`
compares manifests with what the server holds, and a conflict resolution
picks a side after looking at `{base, ours, theirs}`. Between the read and
the write, someone else can change that state. If the write goes through
anyway, it silently overwrites work its author never saw.

The server refuses such writes. A decision carries the revisions it was
made from, the server checks them while holding the mutation lock, and a
mismatch comes back as a typed refusal that names what moved. The caller
then reads again and decides again. Nothing is merged or retried on the
caller's behalf, because the decision itself may no longer be right.

## Apply: every write carries its precondition

`apply` plans from one `BatchGetEntities` call. That response reports, next
to each entity, the etag the server held when it answered (`etags`, parallel
to `entities`; empty for an entity that does not exist). The plan keeps that
revision per manifest, and the apply sends it with the write:

- An entity that was absent when planned is written with `create_only`. If
  another writer created it in the meantime, the write is refused instead of
  replacing their entity.
- An entity that was present is written with `if_match` set to the planned
  etag. If another writer updated or deleted it, the write is refused
  instead of overwriting their change or resurrecting what they deleted.

The batch is atomic, so one stale write refuses the whole apply and nothing
lands. Dry runs (`--dry-run`, `validate_only`) check the same preconditions,
so a dry run reports the same staleness the real apply would hit. Applies
scoped to a branch carry the same guarantee; there the etag is the revision
of the branch's own delta row.

The refusal is a `BatchMutateResponse` with status `FAILED` and a
`precondition_failure` naming the entity, the etag the plan expected, and
the etag the server holds now. An empty etag on either side means the
entity is absent. Other batch failures (validation errors, a missing entity
a delete names without a precondition) never set `precondition_failure`, so
a caller can tell "your input is wrong" apart from "the world moved".

Unchanged entities are not written and therefore not checked. If another
writer changes one of them between plan and apply, the apply still succeeds:
it never claimed anything about entities it left alone.

## Resolution: a decision is bound to the conflict it decided

A conflict resolution is a judgement about specific content. `keep_ours`
after reading one baseline revision says nothing about a newer baseline
revision that arrived afterwards; `take_theirs` after reading one branch
edit says nothing about a later branch edit it would now discard.

Every entry `DiffBranch`, `UpdateBranch` and `MergeBranch` report carries a
`state`: the `base_etag` the branch delta was taken against, the `ours_etag`
of the delta row itself, and the `theirs_etag` of the current baseline, each
empty when that side is absent. `ResolveBranchEntry` takes that state back
as `expected_state`. The server recomputes it under the mutation lock and
refuses the resolution with `ABORTED` if any side differs, or if the branch
no longer holds an entry for the key. The conflict stays exactly as it was.

`trogon-atlas` and the MCP tools always send the state; the wire field stays
optional only so older callers keep working.

## How a refusal reaches the caller

Each surface turns the refusal into an instruction to replan rather than a
generic error:

- **Rust client** (`trogon-atlas-client`): `apply` returns a `StalePlan`, and
  `branch::resolve_branch_entry` returns a `StaleResolution`, inside the
  `anyhow::Error`. A caller downcasts to read the entity and both sides.
  `StaleResolution` carries the entry's current state, read by diffing the
  branch again right after the refusal.
- **trogon-atlas**: prints `stale, replan: ...` with both revisions and exits 3,
  a code no other failure uses.
- **MCP**: `apply_manifests` and `resolve_branch_entry` fail with
  `invalid_params` whose `data` has `category: "stale_state"` and
  `next_action: "replan"`, plus `entity` and the expected and actual
  revisions (`expected_etag`/`actual_etag` for apply, `expected_state`/
  `actual_state` for resolve). A `null` revision means absent.

## Older servers

A server that enforces these checks advertises
`GetServerInfo.features.state_preconditions` (contract revision 2). trogon-atlas
and the MCP tools refuse to apply or resolve against a server that does not,
through the same [compatibility check](wire-compatibility.md) that gates
every mutation, because such a server would accept the write without
checking anything. Reading still works: `trogon-atlas diff` and the MCP
`diff_manifests` tool show the same diff they always did. A plan made
against such a server through the library carries no revision for the
entities it found, and applying it is refused rather than written
unguarded.

## What this does not cover

- Direct `PutEntity`/`BatchMutate` callers get the guarantee only if they
  send `create_only` or `if_match` themselves.
- The refusal tells the caller what moved, not why. Who wrote the newer
  revision is in the [changeset log](changesets.md).

See [how to replan after a stale refusal](../how-to/replan-after-a-stale-refusal.md)
for the recovery steps, and [branching](branching.md) for how conflicts are
classified in the first place.
