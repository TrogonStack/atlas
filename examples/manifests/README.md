# Example manifests

Event models for a fictional retailer, Acme, written as declarative
manifests. Each directory is one namespace, with one multi-document YAML file
per entity kind.

| Namespace                     | Contents                                                          |
|-------------------------------|-------------------------------------------------------------------|
| `acme`                        | The problem space: the project, the domain, and its subdomains    |
| `acme-storefront`             | Cart, checkout, and payment capture                               |
| `acme-fulfillment`            | Carrier dispatch, delivery confirmation, and order tracking       |
| `acme-reviews`                | Review eligibility and review submission for delivered orders     |
| `acme-customer-data-deletion` | A customer data deletion request, across every downstream vendor  |
| `acme-wishlist`               | The smallest valid model: a shopper saves a product to a wishlist |

`acme` holds no bounded context. Every other namespace is one bounded
context that realizes one `acme` subdomain across namespaces.

Apply `acme` first, because every context realizes one of its subdomains.
`acme-fulfillment` references entities in `acme-storefront`, and
`acme-reviews` references entities in `acme-fulfillment`, so apply them
together or in that order: `acme`, `acme-storefront`, `acme-fulfillment`,
then `acme-reviews`. `-f` takes several paths, so one command applies
them together:

```sh
trogon-atlas apply -f examples/manifests/acme/ examples/manifests/acme-storefront/ \
  examples/manifests/acme-fulfillment/ examples/manifests/acme-reviews/
```

Once its dependencies are in the store, apply a namespace in one atomic
batch, check it without writing, or detect drift between the files and a
running server:

```sh
trogon-atlas apply -f examples/manifests/acme-storefront/            # idempotent
trogon-atlas apply -f examples/manifests/acme-storefront/ --dry-run  # validate only
trogon-atlas diff  -f examples/manifests/acme-storefront/            # exit 1 on drift
```

`acme-customer-data-deletion` and `acme-wishlist` only need `acme`. To
build `acme-wishlist` step by step, follow
[start an event model from scratch](../../docs/how-to/start-a-model-from-scratch.md).

Every file here is checked against the generated manifest schema by the
`trogon-atlas-client` tests, and the server's account deletion fixture test
applies `acme` and `acme-customer-data-deletion` end to end. The server's
`example_manifests_validate` test checks that each model validates with no
errors or warnings, and that validating the `acme` project reports a model
for every context with no errors or warnings either.

## Legacy `fields` lists

These manifests declare every Event, Command and ReadModel field list inside
`schema`, as an Any-packed `trogonatlas.eventmodel.v1alpha1.Schema`. The
inline `fields` list used before contract revision 8 is retired. A store
written before then is migrated with
[`trogon-atlas-server migrate-legacy-fields`](../../docs/how-to/migrate-legacy-entity-fields.md).
