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

## External read model reported as unsourced

### Rule finding and evidence

`READ_MODEL_FIELD_UNSOURCED` reports every field of a read model whose
`externalSource` names a system. The read model has no source events,
because the data arrives from outside the model: for example
`readModel:acme-payments/payment-webhooks@1`, fed by
`externalSystem:acme-payments/payment-gateway@1`. An automation slice reads
the view and emits a command that records the webhook.

The rule checked only source events and swimlane state. An external feed
has neither, so no repair inside the model could satisfy it.

### Validator change

The model was correct, so the validator changed. A read model with an
`externalSource` system is exempt from the rule: the external payload
supplies its fields. The rule's description says so.

These alternatives would have changed behavior or hidden the finding:

- Inventing a local event that "carries" the webhook fields describes a
  fact the system does not record.
- Dropping the fields hides the payload the automation reads.
- Removing the external source deletes the seam to the outside system.

### Verification after the fix

Scoped validation of the board reported no `READ_MODEL_FIELD_UNSOURCED`
finding after the server was rebuilt with the fix. A server test pins
both cases: an external read model with a system reports no finding, and
one without a system still reports each field.

### Remaining limits

- An external source with no system still reports one finding per field.
  A single finding about the missing system would be clearer.
- A read model that has both source events and an external source skips
  the check for every field, including fields its events should carry.

## Add a pattern after a repair

Give each new pattern a descriptive heading. Record its symptom,
applicability evidence, behavior-preserving decision, alternatives that
would change behavior, verification, and limits. Link to existing schema
or fixture definitions when useful, and distinguish historical results
from current validation. Amend an existing pattern when the new case
refines its applicability rather than copying the same procedure.

## Example manifests held to the validator

### Findings on unmodified main

The manifests under `examples/manifests/` were only checked against the
manifest schema, so they drifted from the validator and taught patterns it
rejects. `example_manifests_validate` in `trogon-atlas-server` now applies all
three namespaces to an ephemeral store and requires every model to validate
with no warning or error.

On unmodified main the three models reported these findings.

| Model | Errors | Warnings |
| --- | --- | --- |
| `acme-storefront` | 12 | 2 |
| `acme-reviews` | 7 | 2 |
| `acme-customer-data-deletion` | 0 | 1 |

### Decisions

- **System-recorded timestamp.** `EVENT_FIELD_UNSOURCED` on
  `order.placed.placed_at` and `review.submitted.submitted_at` is a real gap.
  The command now declares the field with a `DerivedFieldAnnotation`, the
  way `capture-payment.paid_at` already did.
- **Fields a downstream view needs travel with the events.**
  `READ_MODEL_FIELD_UNSOURCED` on `fulfillment-queue.buyer_id` and
  `seller_id` was repaired by carrying both from `place-order` through
  `order.placed`, `pending-payments`, `capture-payment` and `order.paid`.
  Dropping the fields would have removed what the dispatch automation hands
  to `mark-fulfilled`.
- **A view must carry every field of each source event.**
  `RM_SOURCE_EVENT_FIELD_NOT_PROJECTED` is repaired by adding the missing
  fields, including swimlane state such as `status`.
- **One projection per emitter.** A read model fed by several events
  (`order-status`) cannot sit in one slice: `PROJECTION_BEFORE_EMITTER` and
  `PROJECTION_NOT_ADJACENT` both fire. It is now one read-model slice per
  source event, each placed right after the command slice that emits it.
- **A queue filled in an earlier storyboard.** `pending-payments-rms` moved
  into `checkout-flow` after `place-order-slice`. `fulfillment-flow` still
  enters on that queue. This is the same-context sibling of the
  [upstream subscription pattern](#upstream-subscription-at-a-local-storyboard-entry):
  there the emitter lives in another bounded context, here in another
  storyboard of the same model, so the projection can follow its emitter
  instead of leaving the model.
- **Seed catches up with the recorded acme-reviews repair.** The upstream
  fulfillment projection left the storyboard and the model membership, and
  the storyboard doc says why. The projection slice itself is deleted from
  the seed because no storyboard, model, scenario, or test references it.
- **Each example declares its Project.** `DOMAIN_MISSING_PROJECT` only shows
  in project-scoped validation. Each namespace now has a Project of the same
  name that its Domain points to, and the test validates every Project.
- **Contexts touch through subscriptions only.**
  `CROSS_MODEL_BOUNDARY_VIOLATION` on the grant command slice scenario was
  repaired by dropping the `given` that named an `acme-storefront` event. The
  automation slice scenario already starts from the subscribed queue.
- **Entry read model.** `STORYBOARD_MISSING_ENTRY` is repaired by naming the
  read model the entry observer sees: `cart-summary`, `eligible-reviews` and
  `pending-deletions`.
- **Declared intent.** `ORPHAN_NOT_FLAGGED` on `support-agent` and
  `UI_USED_AS_INPUT_AND_OUTPUT` on `review-form` are kept and annotated
  (`OrphanAnnotation`, `MixedSurfaceAnnotation`) because the persona is
  reserved for a flow not modeled yet and the form is prefilled from the row
  it submits against.

### Rule observations, not changed here

- `RM_SOURCE_EVENT_FIELD_NOT_PROJECTED` cannot express a view that keeps a
  count and ignores the event payload: `cart-summary` now also holds the last
  `product_id` and `quantity`.
- `STORYBOARD_MISSING_ENTRY` asks for an observed read model even when the
  flow starts at a UI that has nothing to display yet, as in `checkout-flow`.
- `trogon-atlas fmt --check` already reports
  `acme-customer-data-deletion/read-model.yaml` as unformatted on main. That
  file is left alone.
