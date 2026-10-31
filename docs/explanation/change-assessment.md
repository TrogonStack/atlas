# Change assessment

A merge dry run already reports the raw evidence about landing a branch:
whether it conflicts, whether the landed world passes validation, the
[consequence delta](consequence-delta.md) and the
[invariant check](invariant-gates.md). A reviewer, human or automated, still
has to turn that evidence into a decision. The change assessment does that
on the server, so every client reads the same verdict from the same rules
instead of each one re-deriving its own.

Field-level detail lives in the
[change assessment reference](../reference/change-assessment.md).

## Advisory, not enforced

The assessment never refuses a merge. A `MergeBranch` call without
`dry_run` lands or refuses exactly as it did before the assessment
existed, and its response carries no assessment. Whether a `reject` or
`needs_review` verdict should one day block a merge, and who may override
it, is a policy decision left for later. Shipping the verdict first lets
its rules be observed against real branches before anything depends on
them.

## The verdicts

- `reject`: the merge as it stands cannot or should not land. The branch
  conflicts with baseline, or the landed world fails a blocking check.
- `needs_review`: the merge would be admitted, but something about it calls
  for a person. The reasons say what.
- `land`: the merge would be admitted, the delta proves it introduces no
  finding and worsens none, and every comparison it relied on completed.

Every verdict except `land` lists stable reason codes. Reasons accumulate,
so a rejected merge still names the findings it would introduce.

## Unknown never lands

`land` is a claim that nothing got worse, and that claim needs proof. Each
of these yields `needs_review`, never `land`:

- the delta is missing or reports itself incomplete,
- the delta could not classify a finding and calls it unknown,
- an invariant could not be evaluated or was hidden from the caller,
- either world the delta compared has no snapshot id, so the verdict cannot
  be tied to the state it judged.

A gap in coverage is a reason to look, not a reason to assume the best.

## Binding

A verdict is only meaningful for the exact state it judged. Each
assessment carries a binding:

- the branch name,
- the baseline snapshot id the delta started from and the snapshot id of
  the world the merge would produce,
- every branch entry that would land, with the etag its baseline
  counterpart had,
- the scope the caller could read, since a hidden subject contributes
  nothing to the verdict,
- the catalogue revision: a digest of the assessment rules and the
  validation rule catalogue.

A reader that holds an assessment can compare the baseline snapshot id
with [`GetSnapshotId`](snapshot-identity.md) and the pinned etags with the
live entities. If either moved, the verdict describes a world that no
longer exists and the dry run should be repeated. A different catalogue
revision means the rules changed and the verdict may differ today.

The assessment is a pure judgement of evidence the dry run already
gathered. It reads nothing from the store, so it adds no store work to the
dry run and none under the mutation lock.

## Where it appears

The assessment rides `MergeBranchResponse.change_assessment` on dry runs,
including dry runs that report conflicts. The CLI's `branch merge` JSON
output and the MCP `merge_branch` tool include it under
`change_assessment`. There is no separate MCP tool: the verdict belongs to
the merge preview a caller can already run, so it inherits that tool's
permissions instead of needing a policy of its own.

## Default chosen for review

- Any introduced finding, at any severity, needs review. A warning a
  branch adds is still a change in the model's health that a reviewer
  should see.
- A finding counts as worsened only when its severity rises. A lowered
  severity and a finding carried to a newer version of its subject at the
  same severity are not reasons.
- Coverage is judged through the delta's completeness. A delta is only
  complete when every compared suite completed on both sides, so a skipped
  or partial suite always surfaces as `delta_incomplete`.
- An invariant refusal stays the typed `INVARIANT_*` error the dry run
  returns, not a `reject` verdict. A refused merge has nothing to assess.
- The scope is the caller's ownership boundary, not their identity: two
  callers who can read the same namespaces get bindings with the same
  scope.
