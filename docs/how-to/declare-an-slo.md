# How to declare an SLO

Attach a measurable objective to a real connection in the model, alert on
it, validate it, and export it to a vendor-neutral format. See
[reliability overlays](../explanation/reliability-overlays.md) for why
these entities point at connections instead of carrying free-text targets.

This guide assumes an existing `ecommerce` namespace with
`command:ecommerce/place-order@1`, `event:ecommerce/order.placed@1`,
`event:ecommerce/order.confirmed@1`, and the command slice that handles
`place-order` already modeled, plus the `ecommerce/checkout@1`
`EventModel` referenced elsewhere in these docs.

## Declare the indicators

A `ServiceLevelIndicator` names a connection and a signal. A latency
indicator on the command slice that handles `place-order`:

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ServiceLevelIndicator
metadata:
  name: place-order-latency
  namespace: ecommerce
spec:
  title: Place order latency
  doc: How long it takes to accept a place-order command.
  connection:
    commandHandling:
      commandSlice:
        id: { namespace: ecommerce, slug: place-order-slice, version: 1 }
  latency: {}
  signal:
    eventTimestamps: {}
```

A completion indicator over the journey from placing an order to
confirming it, correlated on `order_id`:

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ServiceLevelIndicator
metadata:
  name: order-confirmation-journey
  namespace: ecommerce
spec:
  title: Order confirmation journey
  doc: Fraction of placed orders confirmed within 10 minutes.
  connection:
    journey:
      storyboard:
        id: { namespace: ecommerce, slug: checkout-flow, version: 1 }
      start:
        id: { namespace: ecommerce, slug: order.placed, version: 1 }
      end:
        id: { namespace: ecommerce, slug: order.confirmed, version: 1 }
      correlateOn: [order_id]
  completion:
    within: 600s
  signal:
    eventTimestamps: {}
```

`completion.within` and every other `google.protobuf.Duration` field in a
manifest is proto3 JSON's duration string: a plain number of seconds with
a literal `s` suffix (`600s`, `0.300s`), never a shorthand like `10m`.

## Declare the objective and its alert

A notification target names where an alert pages:

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: AlertNotificationTarget
metadata:
  name: checkout-oncall
  namespace: ecommerce
spec:
  title: Checkout on-call
  doc: Pages the checkout on-call rotation.
  target: pagerduty:checkout-oncall
```

An alert policy fires on a burn rate against that target:

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: AlertPolicy
metadata:
  name: place-order-burn-rate
  namespace: ecommerce
spec:
  title: Place order burn rate
  doc: Pages when the place-order latency budget burns down fast.
  conditions:
  - displayName: Fast burn
    severity: SEVERITY_PAGE
    burnRate:
      op: COMPARISON_GTE
      threshold: 14.4
      lookbackWindow: 3600s
      alertAfter: 300s
  notificationTargets:
  - id: { namespace: ecommerce, slug: checkout-oncall, version: 1 }
  runbook: docs/runbooks/place-order.md
```

The objective ties an indicator to a service, a threshold, a target ratio,
and the alert policy that watches it:

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ServiceLevelObjective
metadata:
  name: place-order-slo
  namespace: ecommerce
spec:
  title: Place order latency SLO
  doc: Place order stays under 300ms for 99% of attempts over a rolling 30 days.
  indicator:
    id: { namespace: ecommerce, slug: place-order-latency, version: 1 }
  service:
    id: { namespace: ecommerce, slug: checkout-service, version: 1 }
  objectives:
  - displayName: p95 under 300ms
    threshold: { op: COMPARISON_LTE, value: 0.300s }
    target: { ratio: 0.99 }
  timeWindow:
    rolling: { duration: 2592000s }
  budgetingMethod: BUDGETING_METHOD_OCCURRENCES
  alertPolicies:
  - id: { namespace: ecommerce, slug: place-order-burn-rate, version: 1 }
  commitment:
    internal: {}
```

A latency indicator's objective must carry `threshold`; a ratio or
completion indicator's must not (`SLO_THRESHOLD_MISMATCH`). `target` must
be a ratio strictly between 0 and 1 (`SLO_TARGET_RANGE`).

## Add the new entities to the EventModel

The validator only checks an indicator or objective that is listed as a
member of an `EventModel`; one that exists in the store but is not a
member of any model is never checked. Add each new entity to
`ecommerce/checkout@1`'s `members`:

```yaml
spec:
  members:
  # ...existing members...
  - id: { namespace: ecommerce, slug: place-order-latency, version: 1 }
    kind: ENTITY_KIND_SERVICE_LEVEL_INDICATOR
  - id: { namespace: ecommerce, slug: order-confirmation-journey, version: 1 }
    kind: ENTITY_KIND_SERVICE_LEVEL_INDICATOR
  - id: { namespace: ecommerce, slug: checkout-oncall, version: 1 }
    kind: ENTITY_KIND_ALERT_NOTIFICATION_TARGET
  - id: { namespace: ecommerce, slug: place-order-burn-rate, version: 1 }
    kind: ENTITY_KIND_ALERT_POLICY
  - id: { namespace: ecommerce, slug: place-order-slo, version: 1 }
    kind: ENTITY_KIND_SERVICE_LEVEL_OBJECTIVE
```

## Apply and validate

Apply every manifest, including the updated `EventModel`:

```sh
trogon-atlas apply -f place-order-latency.yaml -f order-confirmation-journey.yaml \
  -f checkout-oncall.yaml -f place-order-burn-rate.yaml -f place-order-slo.yaml \
  -f event-model.yaml --endpoint http://127.0.0.1:50069
```

`trogon-atlas apply`, even with `--dry-run`, only runs request-time validation on
the batch; it does not run `validate_model`, so it never surfaces
reliability findings. Run full validation through the `validate_event_model`
MCP tool instead:

```json
{ "namespace": "ecommerce", "slug": "checkout", "version": 1 }
```

Check the response for `SLI_CONNECTION_UNRESOLVED`, `SLI_JOURNEY_UNREACHABLE`,
`SLI_CORRELATION_UNSOURCED`, `SLI_OUTCOME_UNREACHABLE`,
`SLI_SIGNAL_MISMATCH`, `SLO_THRESHOLD_MISMATCH`, `SLO_TARGET_RANGE`,
`SLO_EXTERNAL_UNALERTED`, and `SLO_UNALERTED` (see
[reliability validation rules](../reference/reliability-rules.md)). An
objective with an `Internal` commitment and an alert policy, like the one
above, clears all of them.

## Export to OpenSLO

The stored model has no vendor queries in it; a bindings file supplies
them at export time. Bindings are keyed by `Signal`, and each entry
supplies the metric query for every role its measure needs: `threshold`
for a latency measure, `good`/`total` (or `bad`/`total`) for an outcome
ratio or completion measure.

```yaml
bindings:
- signal:
    source: eventTimestamps
  dataSource:
    name: event-store
    type: EventStore
    connectionDetails:
      endpoint: https://events.internal
  metricSource:
    threshold:
      query: "p95:trace.place-order.duration{service:checkout}"
    good:
      query: "events in order.confirmed within {{within}} of order.placed"
    total:
      query: "events in order.placed"
```

Export the namespace:

```sh
trogon-atlas openslo export --namespace ecommerce -o ecommerce.openslo.yaml \
  --bindings openslo-bindings.yaml
```

`trogon-atlas openslo export` exits nonzero when an indicator or objective was
skipped (reported as a "NOT exported" list on stderr), even though the
output file for everything that did export is still written. Pass
`--allow-placeholder-metrics` to export anyway with a placeholder query
for a role no binding supplies, rather than skipping that entity.
