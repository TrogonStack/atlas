# How to track a version migration with a Tracker

`SLICE_STALE_REF` findings tell you what is not migrated yet, but only as a
binary fixed-or-not signal. When a migration needs human sequencing (owners,
blockers, review states), overlay a Tracker. Trackers are the
project-management layer by design: flipping an item status
never mutates a design entity, and several trackers can overlay the same
model without interfering.

## Seed the Tracker from the live reference set

1. Call `get_incoming_references` on the OLD version (e.g.
   `ecommerce/order.placed@1`). Every hit is one unit of migration work.

   Alternatively, call `retarget_references` with `dry_run: true` on the OLD
   version to get the same referrer list in one call; the `results` field maps
   directly to Tracker items.

2. Create one Tracker with one item per hit via `put_entity_json`:

   ```json
   {
     "tracker": {
       "id": {"namespace": "ecommerce", "slug": "migrate-order-placed-v2", "version": 1},
       "title": "Migrate order.placed v1 to v2",
       "items": [
         {
           "subject": {"kind": "ENTITY_KIND_COMMAND_SLICE",
                       "id": {"namespace": "ecommerce", "slug": "place-order", "version": 1}},
           "status": "TRACK_STATUS_TODO",
           "doc": "bump emitted_events ref to @2"
         }
       ]
     }
   }
   ```

3. Work the items: `TODO`, `IN_PROGRESS`, `REVIEW`, `BLOCKED` (with the
   reason in `doc`), `DONE`.

## Reconcile against the source of truth

The Tracker is a plan; validation is the truth. Reconcile on demand:

1. Run `validate_project` and filter findings for code `SLICE_STALE_REF`
   where the flagged ref targets the old version.
2. An item marked `DONE` whose slice still appears in the findings is not
   done; flip it back and note why.
3. Zero findings for the pair AND `get_incoming_references` on the old
   version returning empty means the migration is complete, regardless of
   what the Tracker says. Close remaining items and mark the Tracker itself
   done in its `doc`.

## Notes

- Intentionally pinned referrers (those carrying a justification via
  `PinnedRefAnnotation`) are excluded from the findings; track them as a
  terminal `DONE` item whose `doc` records the justification, or leave them
  out of the Tracker entirely.
- Do not track migration state in the design entities' own doc text. It goes
  stale; the reference set and the findings are computed fresh every time.
