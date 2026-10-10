# How to check code against the model (drift report)

The schema deliberately models decisions and data, never implementation.
Keeping code and diagram in sync is therefore an agent
procedure over the MCP tools, not a server feature. This guide is the
read-only phase: produce a drift report, mutate nothing.

## What the model gives the agent

- `LinkAnnotation` with `rel: "implementation"` on an entity points at the
  code (file, package, or PR) that implements it.
- `Component.members` lists what a deployable unit hosts (processors, UIs,
  read models); `Component.tech` and `doc` name the transports involved.
- `Schema` entities and `FieldSpec` carry the typed field contract.
- Scenarios (Given/When/Then) carry the behavioral contract per slice.

## Procedure

1. **Pick the scope.** A Component is the natural unit: fetch it, then
   `batch_get` its members. Entities without any implementation link are
   reported as **unlinked** and skipped.

2. **Resolve each implementation link** to actual code in the repository.
   A link that resolves to nothing (moved or deleted code) is **stale-link**.

3. **Compare contracts, per entity:**
   - Events / commands / read models: model `Schema`/`FieldSpec` field names,
     kinds, and optionality versus the code-level type. Missing field in
     code, extra field in code, or type mismatch is **drifted**, reported
     field by field.
   - Slices: the scenario set versus the handler's observable behavior
     (emitted event types, rejection reasons). Judgement call; report with
     the evidence, not just a verdict.

4. **Emit the report.** Per entity: `in-sync`, `drifted` (with field-level
   detail), `stale-link`, or `unlinked`. Counts up front. No writes: findings
   about the model itself (e.g. the code is right and the model is behind)
   become proposals for a human, drafted per the drafting guide, never
   applied directly.

## Watch mode (incremental)

Instead of re-checking everything, poll `list_changes` with the stored
cursor (scoped, when a scope matching your component's model exists) and
re-run steps 2 and 3 only for entities whose change events arrived. Publish
results as a Tracker overlay so status lives where project state belongs,
one item per drifted entity.

## Boundaries

- The git mirror (when enabled) is the model's edit history; correlate a
  drift with the model commit that introduced it, but never read the mirror
  as the source of truth. The store is.
- Generating code from the model and proposing model edits from code are
  later phases; both must go through drafts and human review.
