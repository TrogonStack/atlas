# How to assess the blast radius of a change

Answer "what breaks if I land this?" before touching any real entity:
classify every dependent of the current version as broken by the change or
bump-only.

## Prerequisites

- The current entity exists (call it `event/ecommerce/order.placed@1`).
- The proposed shape exists as an exact version readable on the same branch:
  a scratch draft (see [draft then land](draft-then-land.md)) or the next
  version on a branch.

## Steps

1. **Classify the dependents.**

   ```sh
   trogon-atlas classify-impact event/ecommerce/order.placed@1 event/ecommerce/order.placed@2
   ```

   From MCP, call `classify_impact` with `current` and `candidate` as
   `{kind, namespace, slug, version}`. Add `--branch` (CLI) when the
   candidate lives on a branch.

2. **Read the verdict.**
   The CLI exits `0` when every dependent is bump-only, `1` when at least one
   breaks, and `12` when nothing provably breaks but something could not be
   proven bump-only. JSON output carries the same answer as `clientVerdict`.

3. **Fix the breaking dependents.**
   Each breaking dependent lists its occurrences: the reference site
   (`from_field`), the root fields it consumes (`consumed_paths`) and the diff
   ops that break it (`causal_ops`). Fix those before or during the reference
   bump.

4. **Resolve every unknown.**
   Read `incompleteness` first. `DEPTH_LIMITED` means raise `--max-depth`;
   `HIDDEN_DEPENDENTS` means someone with a wider view must run the
   classification; `SNAPSHOT_TRUNCATED` means the server's snapshot cap is too
   small for the model; `UNRESOLVED_REFERRERS` means a referrer could not be
   read, so inspect `get_incoming_references` by hand. Then read each unknown dependent's `unknown_reasons`
   and inspect that site by hand.

5. **Bump the rest.**
   Bump-only dependents need only their pinned reference rewritten, for
   example with `retarget_references`. The answer holds for the
   `snapshot_id` it reports; if `get_snapshot_id` returns a different value
   before you land, classify again.

The [impact classification reference](../reference/impact-classification.md)
lists every rule and reason.

## Check the classification by hand

The manual procedure the RPC replaces still works for a single dependent, and
it is how to audit an answer:

1. Call `diff_entities` with `a` = the current entity and `b` = the
   candidate. Paths look like `schema.fields[amount]`, with
   `.kind`, `.repeated`, `.optional`, `.doc` and `.metadata` suffixes for
   changes within a field.
2. Call `get_impact` on the current entity. It walks outgoing references as
   well as incoming ones, so keep only nodes whose every path hop is
   incoming: each hop's `to` is the previous node. The rest are not
   dependents.
3. For each dependent, intersect the fields its reference sites consume with
   the removals and type or cardinality changes from step 1.
4. Do not treat additions as harmless. A projection must project every field
   its source event carries, and an event must source every field from its
   command, so adding a field breaks those consumers.

`ClassifyImpact` names the same dependents as step 2 and uses the same ops as
step 1.

## Notes

- The blast radius belongs to the old entity. Nothing new has to exist under
  the real name; a scratch draft is enough for the candidate.
- `get_incoming_references` is the depth-1 subset of the dependents; use it
  when only direct referrers matter.
- After landing, `SLICE_STALE_REF` findings from `validate_project` are the
  live checklist that the bump-only set actually got bumped.
