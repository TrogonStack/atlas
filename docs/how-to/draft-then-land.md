# How to draft and land a new entity version

Safely author a new version of an entity, validate it, measure its blast
radius, and land it without polluting the main EventModel or triggering
model-wide SLICE_STALE_REF noise until you are ready.

## Prerequisites

- The current entity exists, e.g. `ecommerce/order.placed@1`.
- You have write access to at least one scratch namespace or can use a
  dedicated scratch slug suffix (e.g. `order.placed.draft`) that you control.
  A written (slug, version) pair can never be reused after deletion, even in a
  scratch namespace, so choose the slug deliberately.
- The main EventModel that will eventually include the new version exists,
  e.g. `ecommerce/checkout@1`.

## Steps

1. **Write the draft entity under a scratch identity.**
   Choose a scratch namespace (e.g. `scratch`, `wip`) or a scratch slug
   suffix (e.g. `order.placed.draft`) that is distinct from the real name.
   Call `put_entity` with `create_only: true`. Never write the draft under
   the real slug; the (slug, version) pair is permanent and cannot be reused.
   Attach a `LifecycleAnnotation` with `status: "draft"` in the entity's
   `metadata` so the validator and listing tools can treat it accordingly.

2. **Keep the draft off the main EventModel's member list.**
   Do not add the draft entity to `ecommerce/checkout@1` members.
   Instead, create a scratch EventModel (e.g. `scratch/order.placed.wip@1`)
   that lists your draft entity. Use this scratch model as the scope for
   validation and projection calls while authoring.

3. **Validate the draft in isolation.**
   Call `validate_event_model` with the scratch model's identity. Address any
   findings before measuring blast radius. Drafts with `status: "draft"` or
   `status: "proposed"` are excluded from the SLICE_STALE_REF latest-version
   map, so existing referrers of `@1` will not receive stale-ref warnings
   while you work.

4. **Measure blast radius against the real entity.**
   Call `diff_entities` with `a` = the real `@1`, `b` = the draft scratch
   entity, to see what changes. Then call `get_impact` with `root` = the
   real `@1` to find every entity that depends on it. Classify each as
   broken-by-change or bump-only using the diff ops, as described in the
   assess-change-impact guide.

5. **Use `lifecycle_status_not_in` to hide drafts in listings.**
   When listing entities for production views, pass
   `lifecycle_status_not_in: ["draft", "proposed"]` to `list_entities` so
   scratch drafts do not appear alongside landed entities.

6. **Land the new version when ready.**
   Write the real `@2` under the real namespace and slug using `put_entity`
   with `create_only: true`. Include a `supersedes` reference back to `@1`
   in the entity's `supersedes` field so the supersession chain is intact.
   Remove the `LifecycleAnnotation` (or set its status to `"accepted"`) so
   the version participates in SLICE_STALE_REF checks from this point forward.

7. **Rewire referrers with `retarget_references`.**
   Call `retarget_references` with `dry_run: true` first to preview every
   referrer that would be updated. Review the result's `fields` list to confirm
   the scope. Then call again without `dry_run` to write all rewrites atomically
   under the server's mutation lock. Referrers that intentionally stay on `@1`
   should carry a `PinnedRefAnnotation` before this call so they are silenced
   in SLICE_STALE_REF findings and do not appear as outstanding work.

   For partial migrations (only a subset of referrers should move, or you need
   per-referrer control), fall back to the manual path: `get_incoming_references`
   to enumerate referrers, then `batch_mutate` to rewrite each one individually.

   To review the rewrite before anyone sees it, run steps 6 to 9 under a branch
   header (`trogon-atlas --branch`, or `x-trogon-atlas-branch` from any surface).
   `retarget_references` honors it, so the new version and every rewritten
   referrer land as one change set you can diff and merge atomically. See
   [branching](../explanation/branching.md).

8. **Add `@2` to the main EventModel with optimistic locking.**
   Call `put_entity` on `ecommerce/checkout@1` with `if_match` set to the
   current etag (from a prior `get_entity` call) and the new version added to
   its `members` list. This prevents a silent overwrite if another writer
   modified the model concurrently.

9. **Delete the scratch entities.**
   Call `delete_entity` on the scratch draft and the scratch EventModel once
   the real version is landed and all referrers are updated.

## Notes

- After landing, `SLICE_STALE_REF` findings from `validate_project` are the
  live checklist that bump-only referrers actually got bumped.
- Use `diff_entities` with the scratch draft as `b` during authoring
  (step 4) rather than after landing, so you can iterate on the shape before
  any real version is written.
- `get_incoming_references` is the depth-1 subset of `get_impact`; use it
  when only direct referrers matter for step 7.
- `PinnedRefAnnotation` (attach to the slice, not the entity) silences
  SLICE_STALE_REF for referrers that have a genuine reason to stay on an
  older version after the new one lands.
