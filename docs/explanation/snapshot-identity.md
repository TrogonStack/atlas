# Snapshot identity: naming a model state

Status: **SHIPPED.** `GetSnapshotId` returns a content-addressable id for
the current state, per namespace scope and per branch.

## The problem it solves

Before this, nothing in the system could name a state.

Every entity carried an etag, but the etag is deliberately non-composing:
`proto/trogonatlas/type/v1alpha1/id.proto` says outright that a stable etag
on a slice implies nothing about its transitive references. So a caller
holding a thousand etags still had no way to say "the model I looked at".
The only whole-store artifact was `snapshot_cache`, an in-process read
cache, invalidated only by local mutations: not addressable, not durable,
not something a client could hold.

Three things follow from having nothing to name:

- **Nothing to pin.** A drift report, a CI validation run, or an agent's
  analysis was a statement about an unnamed moment. It could not be
  reproduced, and two runs could not be compared.
- **No cheap change detection.** "Did anything change between these two
  moments" meant walking the change feed or rescanning the store.
- **No way to say "validate exactly this state"**, which is what a reviewer
  wants when a merge landed a week ago.

## What a snapshot id is

A snapshot id is a SHA-256 over the set of `(storage key, entity content
hash)` pairs in scope, and an entity content hash is a SHA-256 over that
entity's canonical JSON. Both live in
`rsworkspace/crates/trogon-atlas-core/src/content_hash.rs`.

Equal ids mean equal content. Unequal ids mean something changed.

## Why the hash is over JSON, not over the stored bytes

Protobuf serialization is not canonical. Map-typed fields have no defined
wire order, and a `google.protobuf.Any` payload carries whatever byte order
its producer happened to emit. Hashing stored bytes would make two
semantically identical models hash differently.

That is the same failure the branching model already refuses: byte-level
noise must never present as a conflict. So the hash goes over proto3 JSON
rendered through the embedded descriptor pool, re-serialized with object
keys sorted and no insignificant whitespace.

A second benefit falls out of that choice: a canonical-JSON hash is
reproducible by any reimplementation working from the `.proto` files. A
hash over prost's output would only be reproducible by prost.

## What identity deliberately ignores

**The server-owned `system` block.** `uid` and `created_at` are provenance,
not content. Excluding them is what makes the id agree with
`semantically_equal`, and it is what lets a model restored from manifests
land on the id it was pinned at, even though every entity got a fresh uid
on the way in.

**The difference between an unset field and one explicitly set to its
default.** proto3 JSON omits default-valued fields, and proto3 does not
preserve that distinction on the wire either, so there is nothing here to
preserve.

## Pinned canonicalization decisions

These three would otherwise drift, and changing any of them changes every
id, which is a format break:

1. **64-bit integers render as JSON strings** (`"version": "1"`), which is
   proto3 JSON's own rule, kept verbatim.
2. **Object keys sort by UTF-8 byte order**, done explicitly rather than
   inherited from `serde_json::Map` happening to be a `BTreeMap`. That
   backing type is a build-time feature (`preserve_order`) of a shared
   dependency; content identity must not be decided by feature unification
   somewhere else in the dependency graph.
3. **Floats format as shortest round-trip with a mandatory decimal point**
   (`1.0`, never `1`), so a `double` never collides textually with an
   integer.

Entity hashes and snapshot hashes carry distinct domain tags, so a digest
computed for one purpose cannot be mistaken for the other.

## Scope is part of the identity

`GetSnapshotIdRequest.namespaces` bounds what the id covers. An empty list
means every namespace.

**Two ids are comparable only when taken over the same scope.** An id over
one namespace is not an id for the store. The scope is not recorded inside
the id, so this is a caller obligation, and it is the one sharp edge in the
API.

Branch scope travels the usual way, on the `x-trogon-atlas-branch` metadata
header, so a branch's merged view has its own identity. An untouched branch
has the same id as its baseline, which is the honest answer: it is the same
state.

## Truncation

The server loads at most `MAX_PROJECTION_ENTITIES` entities. When that cap
is hit, `truncated` is true and the id names a *partial* state. Do not
compare a truncated id against anything. This is reported rather than
silently tolerated because the whole value of an id is that it stands in
for a complete state.

## Failure mode: an entity with no content identity

An entity that cannot render as JSON (an `Any` packed under a type url that
is no longer in the descriptor pool) has no content hash, so the snapshot
has no id either, and the RPC fails with `FAILED_PRECONDITION` naming the
key.

Skipping the entity would be worse: it would produce a confident id for a
model that is missing something.

## What this is not

Not a history. The id names a state, not a path to it. Delete an entity and
recreate it identically and you are back at the id you started from, which
is correct: it is the same model.

Not a merkle tree. The snapshot is a flat sorted list hashed down. If
subtree-level diffing ever justifies the structure, `include_entries`
already answers "which entities differ" without it.

## Using it

```
# Pin the state a report describes.
GetSnapshotId(namespaces: ["billing"])
  -> snapshot_id: "9f2c...", entity_count: 412, truncated: false

# Later: did anything change?
GetSnapshotId(namespaces: ["billing"])
  -> compare the two strings.

# Which entities changed?
GetSnapshotId(namespaces: ["billing"], include_entries: true)
  -> per-entity content hashes, sorted by key; diff the two entry lists.
```

The same call is available as the `get_snapshot_id` MCP tool.

Requires the `Reader` role, like every other read RPC.
