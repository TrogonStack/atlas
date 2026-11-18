# Example specifications

`Specification` cases that walk the [example manifests](../manifests/) with
concrete data: initial values, ordered command steps, and the read-model
rows each step should leave. Walk them offline:

```sh
trogon-atlas walk-scenarios \
  -f examples/manifests/acme-reviews examples/manifests/acme-storefront \
  --cases examples/specifications
```

The `trogon-atlas-client` tests walk these cases. The `acme-reviews` case
currently reports `WALK_EVENT_FIELD_UNSOURCED` for
`review.submitted.submitted_at`: the event records a submission time that
neither `submit-review`, the swimlane state, nor a `DerivedFieldAnnotation`
supplies. That is a gap in the example model; repairing it is tracked
separately from these cases.

See [walk scenarios](../../docs/how-to/walk-scenarios.md) and the
[walk_scenarios reference](../../docs/reference/walk-scenarios.md).
