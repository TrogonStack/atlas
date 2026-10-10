# Example manifests

Complete event models for a fictional retailer, Acme, written as declarative
manifests. Each directory is one namespace, with one multi-document YAML file
per entity kind.

| Namespace                     | Models                                                           |
|-------------------------------|------------------------------------------------------------------|
| `acme-storefront`             | Cart, checkout, payment capture, and carrier fulfillment         |
| `acme-reviews`                | Review eligibility and review submission for delivered orders    |
| `acme-customer-data-deletion` | A customer data deletion request, across every downstream vendor |

Apply a namespace in one atomic batch, check it without writing, or detect
drift between the files and a running server:

```sh
trogon-atlas apply -f examples/manifests/acme-storefront/            # idempotent
trogon-atlas apply -f examples/manifests/acme-storefront/ --dry-run  # validate only
trogon-atlas diff  -f examples/manifests/acme-storefront/            # exit 1 on drift
```

`acme-reviews` references entities in `acme-storefront`, so apply them
together or apply the storefront first.

Every file here is checked against the generated manifest schema by the
`trogon-atlas-client` tests, and the server's account deletion fixture test
applies `acme-customer-data-deletion` end to end.

## Legacy `fields` lists

These manifests declare every Event, Command and ReadModel field list inside
`schema`, as an Any-packed `trogonatlas.eventmodel.v1alpha1.Schema`. The
inline `fields` list used before contract revision 8 is retired. A store
written before then is migrated with
[`trogon-atlas-server migrate-legacy-fields`](../../docs/how-to/migrate-legacy-entity-fields.md).
