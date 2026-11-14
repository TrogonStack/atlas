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
   Read `incompleteness` first. `DEPTH_LIMITED` means raise `--max-depth` when
   it is below 16. The server caps the walk at 16; at that cap, continue past
   it by hand, as the continuation below describes.
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
2. Collect every incoming referrer of the current entity, then of each newly
   discovered exact referrer. Keep a visited set of exact entity identities,
   and stop when no new referrer appears. Stop at 16 hops to audit a
   `ClassifyImpact` answer: that capped set is the one the RPC names. When
   the answer is `DEPTH_LIMITED` at that cap, keep the same walk going past
   16 hops. Referrers found only past the cap are the ones the RPC omitted.
   At each hop, follow `next_page_token` on `GetIncomingReferences` until it
   is empty before expanding the next hop. MCP `get_incoming_references`
   returns only the first page and accepts no page token, so a complete hop
   goes through gRPC. Skip the candidate wherever it appears: it is not a
   dependent, and its own referrers are not part of this walk, even when its
   `supersedes` points at the current version.
3. Classify a direct dependent by intersecting the fields its reference
   sites consume with the removals and type or cardinality changes from the
   diff. A transitive dependent does not reference the current entity, so
   its sites are not intersected with that diff. Read each `through` link.
   A link to a bump-only dependent inherits bump-only. A link to a breaking
   or unknown dependent is unknown (`TRANSITIVE_THROUGH_AFFECTED`); inspect
   that intermediate. A UI slice that only points at a breaking read model
   is unknown for this reason, and the break to fix is on the read model.
4. Do not treat additions as harmless. A projection must project every field
   its source event carries, and an event must source every field from its
   command, so adding a field breaks those consumers.

`ClassifyImpact` names the same dependents as the walk that stops at 16 hops
and uses the same ops as `diff_entities`. Referrers found only by continuing
past that cap are outside its answer. A transitive unknown is the
`through` link above, not a second diff of the root.

## Read-path impact inventory

Call MCP `get_impact` with the real entity's flat `kind`, `namespace`, `slug`,
and `version` arguments. The response explores both incoming referrers and
outgoing references, ordered by depth. Inspect each path's direction before
treating a node as a dependent. Use `filter_kinds` to narrow returned kinds
after understanding the paths, and `max_depth` on dense models. When
`depth_limited` or `snapshot_truncated` is set, the node list is a lower bound,
not the whole blast radius. Check paging and depth limits as described in the
[repair playbook](repair-validation-findings.md#discover-the-full-dependency-context).

For each impact node, intersect what it actually uses with the breaking op
paths. An overlap is broken by the change; no overlap is bump-only. Record the
entity, depth, path from the root, classification, and the specific diff ops.

## Notes

- The blast radius belongs to the old entity. Nothing new has to exist under
  the real name; a scratch draft is enough for the candidate.
- `get_incoming_references` is the depth-1 subset of the dependents; use it
  when only direct referrers matter. MCP returns the first page only.
- After landing, `SLICE_STALE_REF` findings from `validate_project` are the
  live checklist that the bump-only set actually got bumped.
