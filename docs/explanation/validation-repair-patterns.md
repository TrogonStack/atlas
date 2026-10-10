# Validation repair patterns

These patterns record why a repair was valid and when it can be reused.
They complement the [repair playbook](../how-to/repair-validation-findings.md).
They are precedents for investigation, not commands to replay without
checking current definitions.

## Upstream subscription at a local storyboard entry

### Symptom and evidence

`PROJECTION_BEFORE_EMITTER` reports a read-model slice that consumes an
event before an emitting command slice in the same storyboard. The source
event might have a valid emitter in an upstream bounded context; the
finding does not establish that the event is absent from the entire model.

This pattern applies when current definitions establish that:

- An upstream command slice emits the exact pinned source event in its own
  storyboard.
- The downstream read model subscribes to that event.
- The downstream storyboard declares that already-populated view as its
  entry, with the appropriate observer.
- Its automation scenarios treat the populated view as given state.
- The flagged projection represents preparation of that entry state,
  rather than a projection that executes during the local narrative.

If that last distinction is not supported by the model, this pattern does
not establish a repair. A projection within the local flow still needs an
earlier causal emitter. Fetch all ordered slices and scenarios before
deciding whether to reorder, correct a reference, or clarify the model.

### Recorded acme-reviews repair

The baseline repair on 2026-09-22 concerned
`readModelSlice:acme-reviews/orders-delivered-unreviewed-upsert-rms@1` in
`storyboard:acme-reviews/review-eligibility-grant@1`.

The fetched `commandSlice:acme-storefront/mark-fulfilled-slice@1` emitted
`event:acme-storefront/order.fulfilled@1` in
`storyboard:acme-storefront/fulfillment-flow@1`.
`readModel:acme-reviews/orders.delivered-unreviewed@1` subscribed to that event.
The review storyboard's entry and automation scenario already described
the granter observing that populated queue.

The repair changed the local narrative boundary:

- `storyboard:acme-reviews/review-eligibility-grant@1` starts with
  `automationSlice:acme-reviews/grant-eligibility-auto@1`. Its documentation
  makes the prepopulated entry explicit; the entry, observer, outcome, and
  remaining ordered slices stay intact.
- `eventModel:acme-reviews/acme-reviews-model@1` no longer explicitly curates
  the upstream fulfillment event or its entry-populating read-model slice.

The source event and projection slice remain stored. The subscription in
`orders.delivered-unreviewed.sourceEvents`, grant command, emitted
eligibility event, local projections, scenarios, and contracts remain
unchanged. The upstream fulfillment narrative remains intact.

Removing the projection from the storyboard alone would leave an explicit
member slice without a storyboard (`SLICE_NOT_IN_STORYBOARD`). Retaining
the explicit upstream event membership without its local member projection
would produce `RM_EVENT_NOT_PROJECTED`. Those memberships were changed
because they described upstream entry preparation as local execution, not
because membership removal is a general way to pass validation.

Importing the upstream command slice would incorrectly join separate
context timelines and violate the storyboard slice namespace boundary.
Inventing a local emitter or removing the subscription would alter domain
behavior. The read model's source-event subscription remains the seam
between the contexts.

### Recorded verification

The repair used `get_entity_json`, `validate_event_model`, and
`batch_mutate_json`. The batch preview returned `STATUS_VALIDATED`, then
the guarded application returned `STATUS_APPLIED`. Fresh scoped validation
cleared the requested finding and introduced no new findings. All fetched
definitions outside the proposed edits retained their etags.

At that execution, acme-reviews changed from 31 errors, 1 warning, and
47 informational findings to 30 errors, 1 warning, and 46 informational
findings. The removed informational finding was `SCHEMA_MISSING` on the
upstream event leaving the downstream membership scope. Its schema was
not repaired. Upstream acme-storefront validation stayed at 14 errors and
12 informational findings. These are historical results, not a statement
about the current store.

Remaining errors included a separate ordering finding on
`acme-reviews/review-window-expiry-slice@1`, missing UI persona, and
read-model field coverage findings. Studio confirmed that the requested finding was
absent. Narrow seed corrections landed on their own; unrelated
seed/live drift was preserved.

### Tooling lessons retained

The original run used namespace exports and manual tracing to discover the
upstream emitter. Future runs should start with the existing incoming,
outgoing, and impact tools, checking their pagination and depth limits.

The temporary driver also decoded mutation results from protobuf. A JSON
input tool does not guarantee a JSON output. That decoding requirement and
the distinction between batch preview and full scoped validation are now
part of the playbook.

No reusable repair runner was shipped with this documentation. The next
automation should preserve these checks while extracting repeated context
collection and verification, rather than generalizing the domain-specific
membership edits into an unconditional fix.

## Add a pattern after a repair

Give each new pattern a descriptive heading. Record its symptom,
applicability evidence, behavior-preserving decision, alternatives that
would change behavior, verification, and limits. Link to existing schema
or fixture definitions when useful, and distinguish historical results
from current validation. Amend an existing pattern when the new case
refines its applicability rather than copying the same procedure.
