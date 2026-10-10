# Changesets: what landed together, and who did it

Status: **SHIPPED.** Every mutation RPC that writes something records one
changeset. `ListChangesets` pages the log newest first; `GetChangeset`
fetches one by id. Every entity a changeset touched also gets a revision
row holding its content before and after, which is what
`GetEntityHistory` pages and what `RevertChangeset` inverts.

## The problem it solves

The change feed answers "what changed". It never answered three questions
a reviewer asks first:

- **Who did this?** A change event named an entity and a time. The author
  existed only in the git mirror's commit metadata, which is a
  best-effort export, not the system of record.
- **What landed together?** A `BatchMutate` of forty entities produced
  forty independent change events. Nothing said they were one act. Undoing
  or reviewing "that migration" meant reconstructing the grouping from
  timestamps, which is a guess.
- **Is this still here tomorrow?** The change feed is a JetStream stream
  with retention (`changes_max_age`, `changes_max_msgs`). Old events are
  discarded by design. Anything treating the feed as history was building
  on a surface that erases itself.

## The split: the feed is a transport, the log is the record

This is the load-bearing distinction, and the two live in different places
on purpose.

| | Change feed | Changeset log |
|---|---|---|
| Storage | JetStream stream `atlas.changes.>` | KV bucket `trogon-atlas-changesets` |
| Retention | Bounded (age and count) | Unbounded, never pruned |
| Granularity | One event per entity | One record per mutation RPC |
| On publish failure | `StoreError::ChangeEventLost`, KV write stays committed | n/a |
| Purpose | Wake up consumers, drive live views | Answer "who/what/when", durably |

A change event carries `changeset_id` and `author` so a feed consumer can
attribute a change without a second call. **Never treat a non-empty
`changeset_id` on an event as proof the changeset row exists**, and never
treat a missing event as proof nothing happened. The feed drops; the log
does not.

## What a changeset is

```proto
message Changeset {
  string id = 1;                 // UUIDv7, server-minted
  string author = 2;             // authenticated principal, never caller-supplied
  string at = 3;                 // RFC 3339
  string message = 4;            // server-generated, matches the git mirror commit
  string branch = 5;             // empty = baseline
  repeated ChangesetOp ops = 6;  // every entity this RPC actually wrote
  string rpc = 7;                // "PutEntity", "BatchMutate", "MergeBranch", ...
}
```

One mutation RPC, one changeset. `PutEntity` records one op; `BatchMutate`,
`DeleteByQuery`, `RetargetReferences` and `MergeBranch` record one op per
entity that actually moved.

## Why UUIDv7

A UUIDv7's lexicographic order is its chronological order. That single
property is what makes the log pageable on a KV bucket that has no ordered
range scan: `page_token` is just the id of the oldest row you have seen,
and the next page is everything sorted strictly below it. No secondary
index, no separate timestamp cursor, no clock skew to reconcile.

The id is validated as a lowercase hyphenated UUID before it reaches NATS,
so a caller-supplied id can never become a KV key carrying a subject
wildcard.

## Why the author cannot be spoofed

The author is `Principal.name` from the auth stack, resolved from the
bearer token. The `x-trogon-atlas-author-name` header is accepted only when
no principal is present, which is the un-authenticated development path. A
write that reaches the store outside a mutation RPC (`trogon-atlas-server
import`, a direct `Store` user, a test) mints no changeset at all, and its
change events carry an empty `changeset_id` and `author`. That is the
documented unattributed case, not a bug: inventing an author would be
worse than admitting there isn't one.

## What is deliberately not recorded

- **No-op writes.** Re-putting byte-identical content changes nothing, so
  there is nothing to attribute. An empty changeset would be a false audit
  trail.
- **`validate_only` calls.** Nothing was written.
- **Nothing about reads.** The log is a write log.

## Ordering: the writes commit first

`append_changeset` runs *after* the entity writes land. When the append
fails outright the RPC still succeeds (the data is already durable and
reporting failure would misrepresent that), the failure is logged at ERROR
with the changeset id and op count, and `trogon_atlas_changesets_lost_total`
increments.

What happens next depends on how many entities the mutation wrote.

**Baseline batches are journaled.** Entity persistence, change events,
revision rows and the changeset are four separate writes with no
transaction across them, so a batch records its intent first: a journal
entry, keyed by the changeset id, holding the prior image of every key it
touches. The entry moves through these phases and is deleted when the
changeset lands:

| Phase | Written when | Recovery does |
| --- | --- | --- |
| `planned` | Before the first entity write | Restores every key to its prior image |
| `aborted` | An op failed and the rollback started | Finishes the rollback |
| `committed` | Every entity write landed; the RPC will report success | Republishes the change events, rewrites the revision rows, appends the changeset |
| `published` | Change events and revision rows are written | Appends the changeset |

The rule that decides direction is what the caller was told. Before
`committed` nobody heard the batch succeed, so undoing it keeps the error
the caller got true. From `committed` on the caller may already be acting
on the success, so the only honest repair is to finish it. Either way each
entity converges to applied with its revision, change event and changeset,
or absent, and a gap in attribution is temporary instead of permanent. A
changeset that recovery appended has the message
`<rpc>: <n> change(s), completed by batch recovery`.

This costs one journal write before the batch and two after it, per batch
rather than per entity, which is why it is reserved for batches.

**Single-entity writes are not journaled.** One entity write has nothing
partial to undo, and the residual window stays: a crash between the write
and the append loses **the attribution for one mutation, never the write
itself**. The same holds for `MergeBranch` and for branch batches.

Watch `trogon_atlas_changesets_lost_total` alongside
`trogon_atlas_store_change_events_lost_total` and
`trogon_atlas_batch_recovery_unresolved_total`. A nonzero value means
history has a hole, or for a batch, that recovery has not closed it yet;
it never means a write was lost. `trogon-atlas-server recover-batches
--report-only` lists what is outstanding.

Appends are idempotent on id (`kv.create`, treating `AlreadyExists` as
success), so a retried mutation cannot duplicate a row.

## Branches

A branch write gets a changeset. It never reaches the change feed, the
search index, or the git mirror, so the log is the *only* place "who
changed what on this branch" is answerable. `ListChangesetsRequest.branch`
filters to one branch; an empty value returns everything, baseline and
branches alike.

A `MergeBranch` records a **baseline** changeset (empty `branch`) whose ops
are the entities that actually landed, not the branch's whole overlay. The
branch's own edit history stays under the branch name.

Branches also use the log as a coordinate system. `BranchInfo`
`fork_changeset_id` is the newest baseline changeset at the moment the branch
was cut, and `DiffBranch` answers "behind baseline, by how much" by walking
the log back from newest to that point. Since ids order chronologically, that
is a comparison of two positions rather than a scan of the branch's deltas.
See [branching](branching.md).

## Entity history: the other half of the log

A changeset says what landed together. A revision says what one entity
looked like on either side of that landing. Together they are
`git log <path>`: the log is the index, the revisions are the content.

```proto
message EntityRevision {
  string changeset_id = 1;
  string author = 2;
  string at = 3;
  string rpc = 4;
  string branch = 5;
  EntityRef entity = 6;
  ChangeEvent.Kind kind = 7;
  Entity before = 8;   // absent when the write created the key
  Entity after = 9;    // absent when the write deleted the key
}
```

Rows live in their own unbounded KV bucket, `trogon-atlas-revisions`, keyed
`<base|branch.<name>>.<kind>.<ns>.<slug>.<version>.<changeset_id>`. The
changeset id is the last component, so within one entity's prefix
lexicographic order is chronological and `GetEntityHistory` pages exactly
the way `ListChangesets` does.

**Why both images, rather than a chain.** Storing only `after` and
reconstructing `before` from the previous row assumes the chain is
complete. It is not: revisions exist only for attributed writes, so an
`trogon-atlas-server import-git` in the middle of an entity's life breaks the
chain silently and every `before` after it would be a lie. Each row
carries its own pre-image and is true on its own.

**Why the entities bucket cannot supply the pre-image.** It is `history=1`
by requirement, not by accident: the etag *is* the KV revision, so CAS
depends on the depth being one. Once a delete lands the prior content is
gone from that bucket. The revision row is written from the value the
write path already had in hand, before it overwrote it.

**One row per (entity, changeset).** A batch that writes the same key
twice folds into one row keeping the first write's `before` and the last
write's `kind`/`after`, so a changeset's effect on an entity is always one
pair and its inverse is always one op.

Recording is best-effort with the same posture as the changeset append:
the entity write is already durable, so a failure logs at ERROR and
increments `trogon_atlas_store_revisions_lost_total` rather than failing the
RPC. For a baseline batch the journal keeps the pre-image, so recovery
rewrites the row from it and the entity's current value. If another write
replaced that value first, the after-image is gone; recovery reports the
key as `superseded` and writes no row rather than inventing one. A branch batch that rolls back purges the rows it already wrote,
because a row pointing at a changeset the failed RPC never appended is a
phantom entry in that entity's history.

## Revert: undo as a forward write

`RevertChangeset` writes the inverse of a changeset as a new changeset.
Nothing is erased and nothing rewinds: the original stays in the log, the
undo joins it, and the undo can itself be reverted. Because the inverse
goes through the ordinary batch write path it validates, records change
events, updates the mirror and search index, and respects baseline
protection and namespace scope like any other write.

The inverse of each op comes from its revision row: `before` present means
put it back, `before` absent means the changeset created the key, so
delete it. Ops unwind newest first.

It refuses in exactly two cases, and refusing is the point:

- **No revision row.** The write carried no changeset attribution, so no
  pre-image was ever recorded and the entities bucket no longer holds one.
  `FAILED_PRECONDITION`. Guessing would write fabricated content.
- **The entity moved on.** Restoring a pre-image discards everything
  written after it. This works at whole-entity granularity, so *any*
  later edit to the same entity overlaps, which is the same condition git
  reports as a revert conflict. `FAILED_PRECONDITION`, naming the entity.
  Revert the newer changesets first.

`dry_run: true` reports the ops it would write and touches nothing.

The branch header chooses where the undo lands, which need not be where
the original landed. Reverting a baseline changeset under
`x-trogon-atlas-branch` stages the undo for review and merge, exactly like
any other branch write. Pre-images are always read from the original
changeset's own log, baseline or branch.

```
# What has happened to this entity, newest first?
GetEntityHistory(ref: {kind: EVENT, id: {namespace: "shop", slug: "order.placed", version: 1}})
  -> revisions (each with before/after), next_page_token

# What would undoing that changeset do?
RevertChangeset(id: <changeset id>, dry_run: true)  -> ops

# Do it.
RevertChangeset(id: <changeset id>)  -> ops, changeset_id
```

`GetEntityHistory` requires `Reader`; `RevertChangeset` requires `Writer`,
since its targets are a fixed list read off a recorded changeset rather
than a query the caller composes. Both are available as MCP tools
(`get_entity_history`, `revert_changeset`).

## Cost, and when to move off this

`list_changesets` scans the bucket's keyspace per page, keeping at most
`limit` keys in memory (a bounded top-N selection, since JetStream KV
offers no filtered or ordered scan). Memory is bounded; time is O(total
changesets) per page.

`get_entity_history` carries the same caveat and worse: with no ordered
range scan, answering for one entity enumerates the whole revision
keyspace. Memory stays bounded by the page size; time is linear in the
store's total recorded history, not in that entity's own.

That is fine for a design-time model store and will not stay fine forever.
When history outgrows it, project the log into a read-optimised store
rather than adding an index to the KV bucket: the bucket is the record,
and the record should stay simple.

## Using it

```
# What happened most recently?
ListChangesets(page_size: 50)
  -> changesets (newest first), next_page_token

# Walk further back.
ListChangesets(page_token: <id of the oldest row you have>, page_size: 50)

# Only this branch.
ListChangesets(branch: "checkout-redesign")

# Attribute a change event you saw on the feed.
GetChangeset(id: <event.changeset_id>)
```

Both are available as the `list_changesets` and `get_changeset` MCP tools,
and both require the `Reader` role: the log exposes nothing a Reader could
not already see by listing entities, plus the author of each write.

## Related

- [Branching](branching.md): where branch changesets come from.
- [Snapshot identity](snapshot-identity.md): names a *state*; a changeset
  names a *transition*. They compose, and neither replaces the other.
- [Authorization](authorization.md): where the author comes from.
