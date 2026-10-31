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

A snapshot id is a SHA-256 over the declared scope and the set of
`(storage key, entity content hash)` pairs in it, and an entity content
hash is a SHA-256 over that entity's canonical JSON. Both live in
`rsworkspace/crates/trogon-atlas-core/src/content_hash.rs`.

Equal ids mean equal content under the same scope. Unequal ids mean the
content or the scope differs.

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

An id answers a question: "what does this reader see in these
namespaces?" The question is hashed with the answer, so two ids are only
equal when both match. An earlier format hashed the content alone, which
left "compare only over the same scope" as a caller obligation: two empty
namespaces, or a tenant's empty view of a namespace it cannot read, all
produced the same id.

What goes into the digest:

- **The namespace selection.** `GetSnapshotIdRequest.namespaces`, parsed as
  namespace names, sorted and with repeats dropped, so `["b", "a", "b"]`
  and `["a", "b"]` are one question. An empty list means every namespace
  the reader can see, which is its own selection, distinct from any
  explicit list. A name that is not a valid namespace name, including the
  empty string, is `INVALID_ARGUMENT` rather than a filter that matches
  nothing.
- **The reader's owner boundary.** A reader bound to an owner sees only
  that owner's namespaces, so the same request can mean different content
  for different readers. Binding the owner keeps one tenant's id from ever
  matching another's, even when both views are empty. A reader with no
  owner boundary hashes as such.

What stays out of the digest and travels in the response envelope instead:

- **The branch.** Branch scope travels the usual way, on the
  `x-trogon-atlas-branch` metadata header, and the response reports it in
  `scope.branch`. It is not hashed, because an untouched branch is the same
  state as its baseline and should say so. Where the branch diverges, the
  content differs and so does the id.
- **The principal.** Two readers under the same owner see the same
  namespaces, so the principal name would only split identical answers.

The response carries the normalized scope in `scope` (owner, namespaces
and branch) and the format in `identity_version`, so a caller can see what
an id covers without reconstructing the request.

### Format version

`identity_version` is 2. Version 1 hashed content without the scope; a
version 1 id never equals a version 2 id over the same content, because
the snapshot domain tag changed with the format. Ids are not durable
across format versions: re-pin after an upgrade rather than comparing an
old id to a new one.

Default chosen: the owner boundary is part of the id even when a caller
names explicit namespaces, so an id computed by an unbounded operator
never matches the same namespace read by its owner. That gives up
cross-reader comparison in exchange for never letting two different views
share an id.

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

```text
# Pin the state a report describes.
GetSnapshotId(namespaces: ["billing"])
  -> snapshot_id: "9f2c...", entity_count: 412, truncated: false,
     scope: { owner: "", namespaces: ["billing"], branch: "" },
     identity_version: 2

# Later: did anything change?
GetSnapshotId(namespaces: ["billing"])
  -> compare the two strings.

# Which entities changed?
GetSnapshotId(namespaces: ["billing"], include_entries: true)
  -> per-entity content hashes, sorted by key; diff the two entry lists.
```

The same call is available as the `get_snapshot_id` MCP tool.

Requires the `Reader` role, like every other read RPC.
