# Supplied source bundle profile

Status: implemented in the Rust core `source_bundle` module. This intake boundary
validates supplied bytes and computes digests. Registration, retained snapshots,
source overlays and reconciliation remain proposed in the
[source integration contract](source-integration-contracts.md).

## Inventory JSON

The profile name is `supplied-native-v1` and `profile_revision` is the integer
`1`. The inventory is strict UTF-8 JSON. Field names are case-sensitive; every
field below is required except `coverage`, which defaults to `PARTIAL`.
Unknown fields, duplicate object keys at any depth, wrong types and trailing
JSON values reject. JSON object-key order and whitespace remain original bytes.

```json
{
  "profile": "supplied-native-v1",
  "profile_revision": 1,
  "registration": "registry-a",
  "source_version": "revision-A",
  "partition": {
    "categories": ["model"],
    "logical_roots": ["native"],
    "selected_keys": []
  },
  "coverage": "PARTIAL",
  "resources": [
    {
      "key": {"category": "model", "key": "orders"},
      "file": "native/model.yaml",
      "pointer": {"kind": "yaml", "raw": "#/orders"}
    }
  ],
  "relations": [
    {
      "owner": {"category": "model", "key": "orders"},
      "occurrence": "observes-orders",
      "role": "observes",
      "position": 0,
      "target": {"kind": "opaque", "raw": "unresolved-target"},
      "file": "native/model.yaml"
    }
  ]
}
```

Registration, source version, category, resource key, occurrence key, pointer
kind and role are opaque case-sensitive strings. They must be nonblank, contain
no Unicode control characters and occupy at most 1,024 UTF-8 bytes. They are
retained without trimming, normalization or native version conversion.

A pointer's `raw` value is any JSON string occupying at most 16,384 decoded UTF-8
bytes. Empty strings and escaped controls are permitted and retained verbatim
as decoded strings; the original JSON spelling remains in `inventory_bytes`.
Pointers are never resolved, fetched or executed. A relation's `position` is an
unsigned 64-bit integer, independent of its array index. Authored array order is
preserved. Resource keys must be unique. Occurrence keys must be unique within
their owner; repeated equal targets with different occurrence keys remain
separate declarations. Each relation owner must occur in `resources`.

## Partition and file locators

`categories` and `logical_roots` must be nonempty unique lists. `selected_keys`
is a unique list of resource-key objects whose categories must be declared.
An empty `selected_keys` list selects all keys within the declared categories
and roots. A nonempty list restricts enumerated resources to those exact keys.
Selected keys may be absent from `resources`, including under `COMPLETE` coverage.
Their absence supplies no deletion authority at this intake boundary.

Every resource and relation `file` must name a supplied file and match a logical
root exactly or begin with that root followed by `/`. Supplied files outside the
roots are permitted as retained inert bytes. No filesystem lookup occurs.

Logical names and roots are case-sensitive relative slash-separated strings of
at most 1,024 UTF-8 bytes. Empty segments, `.`/`..`, leading or trailing slash,
backslash, colon anywhere and Unicode control characters reject. No path or
Unicode normalization occurs. Supplied file names must be unique. Media types
are required nonblank ASCII strings without controls, at most 1,024 bytes, and
are retained without trimming or inference. File payloads are unrestricted bytes;
archive content may be retained but is never expanded or treated as packaging.

`COMPLETE` and `PARTIAL` are the only coverage values. The validator retains the
supplier's declaration; it does not verify external completeness, authorization,
native validity or snapshot acceptance.

## Intake bounds

| Input | Inclusive ceiling |
| --- | --- |
| Original inventory bytes | 4 MiB |
| Supplied files | 256 |
| Each file payload | 4 MiB |
| Inventory plus file payload bytes | 32 MiB |
| Resources | 1,000 |
| Relation occurrences | 10,000 |
| Each partition list | 1,000 |
| JSON container depth, root container counted as 1 | 32 |

A MiB is 1,048,576 bytes. JSON depth, duplicate keys and collection ceilings
are checked before typed collection deserialization. Transport metadata is
bounded separately by the logical-name and media-type limits. Source run,
retention and publication budgets belong to the proposed control plane.

## Rust boundary and digests

`source_bundle::validate_bundle(&[u8], &[SuppliedFile])` returns an immutable,
owned `ValidatedBundle` or a sanitized `BundleError`. A `SuppliedFile` borrows
`logical_name`, `media_type` and `bytes`; validation retains independent copies.
No disk, network, compilation or native model operations occur.

The result exposes `inventory_bytes()`, typed `inventory()`, `files()`,
`inventory_digest()` and `bundle_digest()`. Files are returned sorted by exact
logical-name UTF-8 bytes; resource and relation arrays retain authored order.
Each retained file exposes its original bytes, typed name/media type and digest.
Digest values expose `purpose()`, `bytes()` and lowercase `hex()`; inventory,
bundle and raw file-content purposes are distinct. Their encoding is defined
by the [digest contract](source-integration-contracts.md#supplied-byte-digest-encoding).

Errors retain fixed categories without raw pointers, unknown keys, input content
or serializer diagnostics. `JsonDepth` identifies excessive nesting; `Inventory`
covers malformed JSON and field shape. Other categories distinguish limits,
text, locators, media types, profile, duplicates, partition and unavailable
supplied owners/files. These categories do not establish native resolution.

Public [boundary tests](../../rsworkspace/crates/trogon-atlas-core/tests/source_bundle.rs)
and [fixed digest vectors](../../rsworkspace/crates/trogon-atlas-core/tests/source_bundle_vectors.rs)
cover the implemented intake slice. They do not prove source persistence or
reconciliation acceptance.
