# How to assess the blast radius of a change

Answer "what breaks if I land this?" before touching any real entity. The
report combines two RPCs and classifies every affected entity as broken by
the change or bump-only.

## Prerequisites

- The current entity exists (call it `ecommerce/order.placed@1`).
- The proposed shape exists somewhere readable: either a scratch draft entity
  (recommended, see the drafting guide) or an in-memory candidate.

## Steps

1. **Diff the proposed shape against the current one.**
   Call `diff_entities` with `a` = the real entity, `b` = the draft. The
   response is a typed op list (`ADDED`, `REMOVED`, `CHANGED`, `RENAMED`)
   with a structural path per op, e.g. `fields[3].name`. Keep the set of
   paths that remove or rename anything; additions are non-breaking for
   existing referrers.

2. **Collect everything affected.**
   Call MCP `get_impact` with the real entity's flat `kind`, `namespace`,
   `slug`, and `version` arguments. The response explores both incoming
   referrers and outgoing references, ordered by depth. Inspect each path's
   direction before treating a node as a dependent. Use `filter_kinds` to
   narrow returned kinds after understanding the paths, and `max_depth` on
   dense models. Check paging and depth limits as described in the
   [repair playbook](repair-validation-findings.md#discover-the-full-dependency-context).

3. **Classify each affected entity.**
   For each impact node, intersect what it actually uses (the fields its
   slice edges, scenarios, and projections touch) with the breaking op paths
   from step 1:
   - overlap: **broken by change**, needs a real fix before or during the
     ref bump
   - no overlap: **bump-only**, the ref rewrite is mechanical

4. **Report.**
   One table: entity, depth, path from root, classification, and for broken
   entities the specific diff ops that break them. Counts up front, e.g.
   "12 affected, 3 broken, 9 bump-only".

## Notes

- The blast radius belongs to the OLD entity. Nothing new has to exist under
  the real name to run this; a scratch draft is enough for step 1 and the
  real `@1` is enough for step 2.
- `get_incoming_references` is the depth-1 subset of `get_impact`; use it
  when only direct referrers matter.
- After landing, `SLICE_STALE_REF` findings from `validate_project` are the
  live checklist that the bump-only set actually got bumped.
