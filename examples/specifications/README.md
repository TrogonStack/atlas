# Example specifications

`Specification` cases that walk the [example manifests](../manifests/) with
concrete data: initial values, ordered command steps, and the read-model
rows each step should leave. Walk them offline:

```sh
trogon-atlas walk-scenarios \
  -f examples/manifests/acme-reviews examples/manifests/acme-fulfillment \
  --cases examples/specifications
```

The `trogon-atlas-client` tests walk these cases and expect no error or
warning findings.

See [walk scenarios](../../docs/how-to/walk-scenarios.md) and the
[walk_scenarios reference](../../docs/reference/walk-scenarios.md).
