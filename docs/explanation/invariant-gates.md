# Invariant gates

Validation reports what is wrong with a model. It never stops a write from
making things worse. An invariant gate does: the owner of an event model
declares the findings the model must never gain, and any write that would
introduce one is refused before anything applies. Field-level detail lives
in the [invariant gates reference](../reference/invariant-gates.md).

## Who declares an invariant

An invariant is an `InvariantAnnotation` in an `EventModel`'s metadata. Only
an event model may carry one, so the namespace writers who may change the
model are the only callers who can add, weaken or remove its invariants.
A writer elsewhere cannot opt out of an invariant by writing in a different
namespace: removing an entity another namespace's model depends on is
judged by that model's invariants.

A declaration names the invariant and forbids model rule codes, a severity,
or both. A declaration that forbids nothing, has no name, or repeats a name
already used on the same model is rejected when the model is written, so a
broken declaration cannot sit silently in the store.

## What a gate judges

The gate compares the findings of the model's own validation pass before
and after the write:

- A forbidden finding the write introduces violates the invariant and
  refuses the write.
- A forbidden finding present both before and after is reported as already
  violated and does not block. Declaring an invariant on a model that
  already breaks it must not freeze every unrelated write; the existing
  findings are listed so they can be fixed.
- Anything else holds.

Every write route that can change what a model sees is gated: `PutEntity`,
`DeleteEntity`, `BatchMutate` (and so manifest imports), `DeleteByQuery`,
`RetargetReferences`, `RevertChangeset` and `MergeBranch`. Each route's
dry run returns the same verdict its real write would, so a preview that
passes describes a write that will pass, and a refused write applies
nothing.

## Unknown is not holding

A verdict is only as good as the world it was computed over. When the
evaluation cannot see enough to judge, the outcome is unevaluable with a
typed reason rather than a guess:

- A member of the model is missing from the world the write produces.
- The snapshot hit the entity cap, so absence proves nothing.
- The scope declares more invariants than one evaluation may check.

By default an unevaluable invariant refuses the write, because an invariant
that cannot be checked is not held. The declaring model may opt into
advisory behaviour per invariant, in which case the write goes through and
the outcome is reported as unevaluable and advisory.

## Optimistic evaluation

Evaluating every invariant against a candidate world is expensive, and the
global mutation lock serializes every writer. The gate therefore never
evaluates under that lock. It reads a snapshot, builds the world the write
would produce, and judges it before taking the lock. The verdict is bound
to the snapshot id and to the etags of the carriers and touched entities
it read.

Under the lock the gate only checks that no write has landed in the scope
since the verdict was taken. If one has, the write is refused as stale
rather than re-evaluated, and the caller replans. That keeps the lock as
short as it was before invariants existed, at the cost of an occasional
refusal under contention.

Routes whose change set only the route itself can compute (merges,
retargets, query deletes and reverts) run their own dry run to judge the
write, then apply it only if nothing moved in between.

## Hidden carriers

Invariants are evaluated over the whole world, including namespaces the
caller cannot read, because an invariant that only counted what the caller
can see could be bypassed by not looking. When an outcome comes from a
model the caller cannot read, its status is kept and every identifying
field is cleared, and the check reports itself incomplete. A refusal from
such a model still refuses, without naming it.

## Defaults chosen for review

- Unevaluable invariants refuse unless the declaration says otherwise.
- Findings that predate the write do not block it.
- A stale verdict is refused, never retried by the server.
- One evaluation checks at most 64 declarations; the rest are unevaluable.
- Write clocks live in the writer process, matching the
  [single-writer](single-writer.md) posture.
- `UpdateBranch` and `ResolveBranchEntry` are not gated; their result is
  judged when the branch is merged.
