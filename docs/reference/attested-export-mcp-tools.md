# Attested export MCP tools

The MCP tools that export an [attested bundle](attested-bundle-format.md)
and verify one. For a walkthrough, see
[export and verify an attested model](../how-to/export-and-verify-an-attested-model.md#through-mcp).

| Tool | Calls | Minimum role |
| --- | --- | --- |
| `export_attested_model` | `ExportAttestedModel` | reader |
| `verify_attested_bundle` | nothing; it reads only its input | reader |

Both tools are read-only. `export_attested_model` sees only the namespaces
the caller may read, and honors the `x-trogon-atlas-branch` header the same
way the RPC does.

## `export_attested_model`

### Export parameters

| Parameter | Default | Meaning |
| --- | --- | --- |
| `namespace`, `slug`, `version` | required | The EventModel to export. `version` must be greater than 0. |
| `as_of_changeset` | current state | Describe baseline as it stood right after this changeset. Not allowed with a branch. |
| `allow_incomplete` | `false` | Return a bundle marked incomplete instead of refusing with `EXPORT_INCOMPLETE`. |
| `include_bundle` | `false` | Put the sealed bundle in `bundle_json`. |

`include_bundle` is off by default because a bundle carries every manifest
and finding of the model, which can crowd an agent's context, while most
calls only need its digest and completeness.

### Export result

| Field | Meaning |
| --- | --- |
| `bundle_digest` | The bundle's digest. Record it to compare a later verification against. |
| `complete` | `true` only for a bundle that covers its whole scope. |
| `incomplete_reasons` | The bundle's [incomplete reasons](attested-bundle-format.md#incomplete-reasons). Empty when complete. |
| `unresolved` | One object per entity the export could not attest: `key` (storage key), `entity` (canonical id, or `null` when the bundle names none) and `cause`, one of the [unresolved causes](attested-bundle-format.md#unresolved-causes). |
| `state` | `mode`, `snapshot_id`, `closure_id`, `baseline_changeset_id` and `branch_changeset_id`, as in the bundle. |
| `bundle_bytes` | Size of the bundle's proto3 JSON. |
| `bundle_json` | The bundle's proto3 JSON. Present only when `include_bundle` is set and the bundle fits. |
| `bundle_omitted` | Why `bundle_json` is absent: `not_requested`, or `exceeds_mcp_limit` when the bundle is larger than 4 MiB. |

The tool checks the bundle with the same rules as `verify_attested_bundle`
before it returns, and fails if the server sent one that does not verify.

### Export refusal

Without `allow_incomplete`, an export that cannot cover its scope fails
with `data.code` `EXPORT_INCOMPLETE` and the reasons in
`data.next_action_detail.incomplete_reasons`. See
[failure reasons](failure-reasons.md).

## `verify_attested_bundle`

### Verify parameters

| Parameter | Meaning |
| --- | --- |
| `bundle_json` | The bundle's proto3 JSON, at most 4 MiB. |

### Verify result

An intact bundle returns a result with `verified` set to `true` and the
same `bundle_digest`, `complete`, `incomplete_reasons` and `unresolved`
fields as an export.

A malformed or altered bundle returns a tool error whose body has
`verified` set to `false` and `error` naming the first
[verification](attested-bundle-format.md#verification) check that failed.

An input over 4 MiB is refused as invalid parameters.
