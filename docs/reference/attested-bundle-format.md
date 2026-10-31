# Attested bundle format

The `AttestedModelBundle` message, defined in
[`validation_attestation.proto`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation_attestation.proto),
as `ExportAttestedModel` returns it and `trogon-atlas export-attested`
saves it. Why the format looks this way is in
[attested export](../explanation/attested-export.md).

## Format identifier

| Field | Value |
| --- | --- |
| `format` | `trogonatlas.attested-model.v1` |

A verifier refuses any other value. A new value means the digest rules or
the meaning of a field changed.

## Fields

| Field | Meaning |
| --- | --- |
| `scope.event_model_id` | The exported model, with its concrete version. |
| `scope.branch` | Branch header the export ran under. Empty means baseline. |
| `scope.lens.unrestricted` | True when the caller could see every namespace. |
| `scope.lens.namespaces` | Namespace ids the caller could see, sorted. Empty when unrestricted. |
| `state.mode` | `MODE_CURRENT` or `MODE_HISTORICAL`. |
| `state.snapshot_id` | `GetSnapshotId` over the whole view validation read. |
| `state.snapshot_entity_count` | Entities that id covers. |
| `state.closure_id` | `GetSnapshotId` over `entries` only. |
| `state.baseline_changeset_id` | Current: newest baseline changeset recorded before the snapshot was read. Historical: the requested changeset. Empty when baseline has none, and on a current export by a caller bound to an owner, since the id dates every owner's writes. |
| `state.branch_changeset_id` | Newest changeset on `scope.branch` recorded before the snapshot was read. Empty on baseline or when the branch has none. |
| `state.closure_truncated` | True when the board walk hit its entity bound, so `entries` stop short of everything the model reaches. |
| `manifests` | Manifest YAML for the board in the model's namespace, as `ExportEventModelManifests` renders it. |
| `external_manifests` | Manifest YAML for entities in other namespaces the board references. |
| `entries` | Content hash of every entity in both manifest sets, sorted by storage key. |
| `skipped` | Entities the renderer could not render. |
| `unresolved` | Entities the export names but could not attest from its snapshot, one row per storage key, sorted by key. Each row carries `key`, `entity` and `cause`. Empty unless `REASON_UNRESOLVED_ENTITIES` is listed. |
| `validation.issues` | Findings of the `model` and `strategic` suites. |
| `validation.coverage` | Coverage for those suites, as `ValidateEventModel` reports it. |
| `validation.rule_catalog.digest` | Digest of the rule catalogue. |
| `validation.rule_catalog.server_version` | Version of the server that sealed the bundle. |
| `completeness.state` | `STATE_COMPLETE` or `STATE_INCOMPLETE`. |
| `completeness.reasons` | Why the bundle is incomplete, sorted and unique. Empty when complete. |
| `bundle_digest` | Lowercase hex SHA-256, computed as below. |

## Incomplete reasons

| Reason | Cause |
| --- | --- |
| `REASON_SNAPSHOT_TRUNCATED` | The entity load hit the server's cap. |
| `REASON_CLOSURE_TRUNCATED` | The board walk hit its entity bound. |
| `REASON_HISTORY_INCOMPLETE` | A historical rewind could not fetch every entity it needed. |
| `REASON_ENTITIES_SKIPPED` | At least one entity could not be rendered and is listed in `skipped`. |
| `REASON_VALIDATION_INCOMPLETE` | A validation suite did not complete. |
| `REASON_UNRESOLVED_ENTITIES` | At least one entity is listed in `unresolved`. |
| `REASON_CONCURRENT_WRITE` | A write landed after the snapshot was read and touched what the export read, or the branch or namespace visibility changed. |
| `REASON_TYPE_LIBRARIES_CHANGED` | Historical only: a visible type library changed after the requested changeset, so manifests render through newer types. |
| `REASON_BOUNDARY_UNVERIFIED` | The exporting process does not hold the writer lease, so its snapshot may predate the changeset boundary it names. |

## Unresolved causes

| Cause | Meaning |
| --- | --- |
| `CAUSE_MISSING` | The board references the entity and the snapshot does not hold it. An untyped slice reference that names no slice in the snapshot is listed once per concrete slice kind. |
| `CAUSE_HIDDEN` | The board references the entity in a namespace the caller cannot see. |
| `CAUSE_CHANGED` | The entity changed after the snapshot was read. The bundle carries the snapshot's copy. |
| `CAUSE_DELETED` | The entity was deleted after the snapshot was read. The bundle carries the snapshot's copy. |
| `CAUSE_OUTSIDE_SNAPSHOT` | The exported model was not in the snapshot and was read directly from the store. |

## Digest rules

Each digest is SHA-256 over a domain tag followed by canonical JSON. The
tag ends with a NUL byte.

| Digest | Domain tag | Input |
| --- | --- | --- |
| `bundle_digest` | `trogonatlas.attested-model.bundle.v1` | The bundle with `bundle_digest` set to empty. |
| `validation.rule_catalog.digest` | `trogonatlas.validation.rule-catalog.v1` | A `ListValidationRulesResponse` holding the catalogue. |
| `state.closure_id`, `state.snapshot_id` | as for `GetSnapshotId` | See [snapshot identity](../explanation/snapshot-identity.md). |

Canonical JSON is the message's proto3 JSON, with default-valued fields
omitted, then re-serialized with the
[pinned canonicalization decisions](../explanation/snapshot-identity.md#pinned-canonicalization-decisions)
that entity content hashes use.

## Verification

`trogon-atlas verify` and the `verify_attested_bundle` MCP tool accept a
bundle when all of these hold, and report the first that does not:

- The saved JSON parses with no unknown field.
- `format` is the identifier above.
- `bundle_digest` recomputes.
- `scope.event_model_id`, `state.mode`, `validation.coverage` and
  `validation.rule_catalog` are present.
- `entries` are in strictly increasing key order with well-formed hashes,
  and `state.closure_id` recomputes from them.
- `unresolved` rows are in strictly increasing key order, each with a
  known cause.
- A complete bundle lists no reasons, no skipped entities and no
  unresolved entities, does not set `state.closure_truncated`, and its
  coverage shows every suite completed over an untruncated snapshot.
- An incomplete bundle lists at least one known reason, sorted and unique,
  lists `REASON_UNRESOLVED_ENTITIES` exactly when `unresolved` is not
  empty, and lists `REASON_CLOSURE_TRUNCATED` exactly when
  `state.closure_truncated` is set.

Verification does not contact a server, so it cannot check that the state
named in the bundle exists. It checks integrity only; see
[integrity, not authenticity](../explanation/attested-export.md#integrity-not-authenticity).
