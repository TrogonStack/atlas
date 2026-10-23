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

Those names are historical. The fulfillment half of the storefront has
since moved into the `acme-fulfillment` namespace, so the command slice is
now `commandSlice:acme-fulfillment/mark-fulfilled-slice@1`, the event is
`event:acme-fulfillment/order.fulfilled@1`, and the storyboard is
`storyboard:acme-fulfillment/fulfillment-flow@1`. The storefront's former
`fulfillment-flow` storyboard is now `payment-flow`. See
[one problem space, contexts realize across namespaces](#one-problem-space-contexts-realize-across-namespaces).

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

## One problem space, contexts realize across namespaces

### Strategic findings and evidence

The strategic entities of an example model reported findings that nothing
surfaced:

- `DOMAIN_IN_CONTEXT_NAMESPACE` and `DOMAIN_SLUG_MISMATCH`: each context
  namespace declared its own Project, Domain and Subdomain. A Domain whose
  slug names a business area (`retail-commerce`, `customer-trust`,
  `customer-privacy`, `shopping`) differs from its namespace, and a Domain
  in a context namespace makes the business look like one context's
  property.
- `REALIZES_MIXED_CLASSIFICATIONS`: `acme-storefront` realized both a CORE
  subdomain (ordering) and a SUPPORTING one (fulfillment). Studio's domain
  chart drew that one context under both Core and Supporting.
- `SEAM_WITHOUT_RELATIONSHIP`: contexts subscribed to each other's events
  without declaring a relationship intent.

This pattern applies when current definitions establish that:

- A Domain shares a namespace with a BoundedContext, or its slug differs
  from its namespace.
- A BoundedContext realizes Subdomains with different classifications.
- A read model in one context sources events from another context that
  declares no `relationships` entry toward it.

### Root cause

Model and project validation never validated Domain, Subdomain or
BoundedContext entities, so these findings were never reported on the
examples. The how-to guide then taught a Project, Domain and Subdomain per
context namespace, which silenced `DOMAIN_MISSING_PROJECT` while hiding the
shape of the business. Both reports now include the strategic chain of the
model's namespace: its BoundedContext, the Subdomains it realizes, their
Domains, and any Domain declared in that namespace.

### Decision

- One problem-space namespace, `acme`, holds the Project, the Domain and
  every Subdomain. Project and Domain slugs equal the namespace.
- Each BoundedContext lives in its own namespace and realizes one `acme`
  Subdomain across namespaces. The context namespaces declare no Project,
  Domain or Subdomain.
- A context that mixes classifications is split along the classification
  line. The storefront became `acme-storefront` (ordering, CORE) and
  `acme-fulfillment` (fulfillment, SUPPORTING). The fulfillment entities
  moved with their behavior: events, commands, slices, read models, UI,
  processor, external system, swimlane, storyboard and tracker.
- Every cross-context seam declares a relationship intent on the
  downstream context: `acme-fulfillment` is a customer of `acme-storefront`
  (`INTENT_CUSTOMER_SUPPLIER`) and `acme-reviews` conforms to
  `acme-fulfillment` (`INTENT_CONFORMIST`).

Never silence `DOMAIN_MISSING_PROJECT` by creating a Project and a Domain
per context namespace. Point the single Domain at the single Project.

These alternatives would have changed behavior or hidden the finding:

- Reclassifying fulfillment as CORE, or ordering as SUPPORTING, only to
  agree with the other half misstates where Acme invests.
- Suppressing `REALIZES_MIXED_CLASSIFICATIONS` while one context keeps both
  halves keeps one boundary under two investment levels.
- Dropping the cross-context subscriptions would remove the seams the
  fulfillment and review flows depend on.

### Verification

`example_manifests_validate` and `account_deletion_fixture` pass. The first
validates every model and the single `acme` project with no warning or
error. The second loads `acme` and `acme-customer-data-deletion`.
`mise run manifests:verify -- bootstrap` reports zero drift. Apply order for
the examples is `acme`, `acme-storefront`, `acme-fulfillment`, then
`acme-reviews`; the other contexts only need `acme`.

### Limits of the split

- The split moved entities between namespaces. Applying the new manifests
  creates and overwrites entities but removes none, so a store seeded from
  the old manifests keeps the old fulfillment entities in `acme-storefront`
  and the per-context Projects, Domains and Subdomains until they are
  deleted.
- `REALIZES_MULTIPLE` stays informational when the Subdomains share a
  classification, so a context that realizes several of them is not
  flagged as a mistake.

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

On unmodified main the three models reported these findings. The test has
since grown to the problem-space namespace and a fourth model, and the
`fulfillment-queue`, `order-status` and `fulfillment-flow` entities named
below now live in `acme-fulfillment`. The decisions are kept as they were
recorded.

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
  in project-scoped validation. Each namespace was given a Project of the
  same name that its Domain pointed to, and the test validated every
  Project. This decision is superseded: a Project and a Domain per context
  namespace was the wrong fix, and it was replaced by one problem-space
  namespace. See
  [one problem space, contexts realize across namespaces](#one-problem-space-contexts-realize-across-namespaces).
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
