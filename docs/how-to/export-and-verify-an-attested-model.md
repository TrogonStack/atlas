# Export and verify an attested model

Save an event model's manifests and validation findings as one bundle, then
check later that nothing in it changed. For what a bundle proves, see
[attested export](../explanation/attested-export.md).

## Export the current state

Point the CLI at the writer. A reader or standby replica refuses with
`NOT_WRITER`.

```sh
trogon-atlas export-attested shop/checkout -o checkout.bundle.json
```

The command prints the bundle digest. Record it somewhere the bundle's
holder cannot rewrite, such as the release notes or the CI log, so a later
check proves the bundle is the one you recorded.

Add `--branch <name>` to export a branch's view of the model.

## Export a past state

Name the baseline changeset whose state the bundle should describe, for
example one listed by `ListChangesets`:

```sh
trogon-atlas export-attested shop/checkout \
  --as-of-changeset "$CHANGESET_ID" \
  -o checkout-then.bundle.json
```

Historical exports read baseline only, so they cannot be combined with
`--branch`. If a type library changed after that changeset, the export is
refused, because the manifests would render through the newer types.

## When the export is refused as incomplete

`EXPORT_INCOMPLETE` (exit 4) means the server could not cover the whole
scope, for example because the store holds more entities than one read
loads. The error lists the reasons. If you still want a record, ask for one
marked incomplete:

```sh
trogon-atlas export-attested shop/checkout --allow-incomplete -o partial.bundle.json
```

The command saves the bundle and exits 11. Do not treat that bundle as a
clean attestation.

## Verify a bundle

```sh
trogon-atlas verify checkout.bundle.json
```

| Exit | Meaning |
| --- | --- |
| `0` | The bundle is intact and complete. Compare the printed digest with the one you recorded. |
| `11` | The bundle is intact but marked incomplete. Its reasons are printed. |
| `5` | The bundle is malformed or was changed after it was sealed. |

With `--format json`, the outcome carries `bundle_digest`, `complete` and
`incomplete_reasons`.
