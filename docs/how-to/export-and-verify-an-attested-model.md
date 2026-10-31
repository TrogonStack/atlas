# Export and verify an attested model

Save an event model's manifests and validation findings as one bundle, then
check later that nothing in it changed. For what a bundle proves, see
[attested export](../explanation/attested-export.md).

## Export the current state

Point the CLI at the writer. A reader or standby replica still exports,
but marks the bundle incomplete with `REASON_BOUNDARY_UNVERIFIED`.

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
`--branch`. If a type library changed after that changeset, the bundle is
marked incomplete with `REASON_TYPE_LIBRARIES_CHANGED`, because the
manifests render through the newer types.

## When the export is refused as incomplete

`EXPORT_INCOMPLETE` (exit 4) means the server could not cover the whole
scope, for example because the store holds more entities than one read
loads, or a write changed the model while it was being exported. The error
lists the reasons. When the only reasons are `REASON_CONCURRENT_WRITE` and
`REASON_UNRESOLVED_ENTITIES`, exporting again once writes settle usually
gives a complete bundle. If you still want a record, ask for one
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
| `11` | The bundle is intact but marked incomplete. Its reasons and unresolved entities are printed. |
| `5` | The bundle is malformed or was changed after it was sealed. |

With `--format json`, the outcome carries `bundle_digest`, `complete`,
`incomplete_reasons` and `unresolved`, a list of `key`, `entity` and
`cause`.

## Through MCP

An agent connected to the MCP endpoint uses `export_attested_model` and
`verify_attested_bundle`. Their fields are listed in the
[tool reference](../reference/attested-export-mcp-tools.md).

1. Call `export_attested_model` with the model's `namespace`, `slug` and
   `version`. The result reports `bundle_digest`, `complete`,
   `incomplete_reasons` and `unresolved` without the bundle itself.
2. If it fails with `EXPORT_INCOMPLETE`, read
   `data.next_action_detail.incomplete_reasons` and act as described in
   [when the export is refused as incomplete](#when-the-export-is-refused-as-incomplete).
   Set `allow_incomplete` only when a record marked incomplete is what you
   want.
3. To keep the bundle, call again with `include_bundle` set and save
   `bundle_json`. If `bundle_omitted` is `exceeds_mcp_limit`, export with
   the CLI instead.
4. To check a saved bundle, pass its JSON as `bundle_json` to
   `verify_attested_bundle`. A tool error with `verified` false means the
   bundle is malformed or was changed. A result with `complete` false is
   intact but partial: `unresolved` names each entity it could not attest
   and why.
