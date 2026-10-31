# Attested bundle format

The `AttestedModelBundle` message, defined in
[`validation_attestation.proto`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation_attestation.proto),
as `ExportAttestedModel` returns it and `trogon-atlas export-attested`
saves it. Why the format looks this way is in
[attested export](../explanation/attested-export.md).

## Format identifiers

| `format` | Signature |
| --- | --- |
| `trogonatlas.attested-model.v1` | Refused. The bundle is unsigned. |
| `trogonatlas.attested-model.v2` | Required. Every other field means what it means in `v1`. |

A verifier refuses any other value. A new value means the digest rules or
the meaning of a field changed. A server without a signing key emits `v1`;
a server with one emits `v2`.

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
| `state.baseline_changeset_id` | Current: newest baseline changeset recorded before the snapshot was read. Historical: the requested changeset. Empty when baseline has none. |
| `state.branch_changeset_id` | Newest changeset on `scope.branch` recorded before the snapshot was read. Empty on baseline or when the branch has none. |
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
| `signature.algorithm` | `ALGORITHM_ED25519`. Present only under `v2`. |
| `signature.key_id` | Lowercase hex SHA-256 of the signing key's raw 32-byte Ed25519 public key. |
| `signature.value` | The 64-byte Ed25519 signature, base64 in proto3 JSON. |

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
| `CAUSE_MISSING` | The board references the entity and the snapshot does not hold it. |
| `CAUSE_HIDDEN` | The board references the entity in a namespace the caller cannot see. |
| `CAUSE_CHANGED` | The entity changed after the snapshot was read. The bundle carries the snapshot's copy. |
| `CAUSE_DELETED` | The entity was deleted after the snapshot was read. The bundle carries the snapshot's copy. |
| `CAUSE_OUTSIDE_SNAPSHOT` | The exported model was not in the snapshot and was read directly from the store. |

## Digest rules

Each digest is SHA-256 over a domain tag followed by canonical JSON. The
tag ends with a NUL byte.

| Digest | Domain tag | Input |
| --- | --- | --- |
| `bundle_digest` under `v1` | `trogonatlas.attested-model.bundle.v1` | The bundle with `bundle_digest` set to empty and no `signature`. |
| `bundle_digest` under `v2` | `trogonatlas.attested-model.bundle.v2` | The bundle with `bundle_digest` set to empty and no `signature`. |
| `validation.rule_catalog.digest` | `trogonatlas.validation.rule-catalog.v1` | A `ListValidationRulesResponse` holding the catalogue. |
| `state.closure_id`, `state.snapshot_id` | as for `GetSnapshotId` | See [snapshot identity](../explanation/snapshot-identity.md). |

Canonical JSON is the message's proto3 JSON, with default-valued fields
omitted, then re-serialized with the
[pinned canonicalization decisions](../explanation/snapshot-identity.md#pinned-canonicalization-decisions)
that entity content hashes use.

## Signed statement

Under `v2`, `signature.value` is an Ed25519 signature, checked with strict
verification, over these bytes:

1. The domain tag `trogonatlas.attested-model.signature.v1` followed by a
   NUL byte.
2. The 32 raw bytes `bundle_digest` encodes.

The tag carries its own version, so a later signature scheme cannot produce
a signature an older verifier accepts by accident. The digest covers
`completeness`, so the signature binds an incomplete bundle's reasons as
firmly as its manifests.

## Server configuration

| Flag | Environment variable | Meaning |
| --- | --- | --- |
| `--attested-signing-key-file` | `TROGON_ATLAS_ATTESTED_SIGNING_KEY_FILE` | Path to a PKCS#8 PEM Ed25519 private key. |
| `--attested-signing-key-env` | `TROGON_ATLAS_ATTESTED_SIGNING_KEY_ENV` | Name of an environment variable holding a PKCS#8 PEM Ed25519 private key. |

The flags are mutually exclusive. With neither set, bundles are unsigned.
The key is read on every export. When it cannot be read or parsed, the
export is refused with `SIGNING_KEY_UNAVAILABLE`, whose `source` metadata is
`file` or `env`; see the [failure reasons](failure-reasons.md). The server
logs the key id at startup when the key loads, and an error when it does
not.

## Verification

`trogon-atlas verify` accepts a bundle when all of these hold, and reports
the first that does not:

- The saved JSON parses with no unknown field.
- `format` is one of the identifiers above, and `signature` is absent under
  `v1` and present under `v2`.
- A signature names `ALGORITHM_ED25519`, a well-formed key id and a 64-byte
  value.
- `bundle_digest` recomputes.
- With `--trusted-key`, the bundle is signed, its key id is one of the
  trusted keys, and the signature verifies against that key.
- `scope.event_model_id`, `state.mode`, `validation.coverage` and
  `validation.rule_catalog` are present.
- `entries` are in strictly increasing key order with well-formed hashes,
  and `state.closure_id` recomputes from them.
- `unresolved` rows are in strictly increasing key order, each with a
  known cause.
- A complete bundle lists no reasons, no skipped entities and no
  unresolved entities, and its coverage shows every suite completed over
  an untruncated snapshot.
- An incomplete bundle lists at least one known reason, sorted and unique,
  and lists `REASON_UNRESOLVED_ENTITIES` exactly when `unresolved` is not
  empty.

Verification does not contact a server, so it cannot check that the state
named in the bundle exists.

## Authenticity outcomes

| `authenticity` in `--format json` | Text output | Meaning |
| --- | --- | --- |
| `unsigned` | `integrity only, issuer unauthenticated` | A `v1` bundle, verified without `--trusted-key`. |
| `signature_unchecked` | `signature present, not checked` | A `v2` bundle verified without `--trusted-key`. Not authenticated. |
| `authenticated` | `signed by trusted key <key id>` | A `v2` bundle whose signature verified against a trusted key. |

`signing_key_id` carries the key id for the signed outcomes and is null for
`unsigned`. What each outcome proves is in
[integrity, not authenticity](../explanation/attested-export.md#integrity-not-authenticity).
