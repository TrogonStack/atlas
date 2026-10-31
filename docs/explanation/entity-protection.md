# Entity protection

Some entities stop being drafts. A published event contract or a command
that other teams already code against should not change because a writer,
a branch merge or a revert happened to touch it. Review conventions catch
most of that, but nothing in the write path enforced it. Entity protection
is the enforced version: an admin marks an identity as protected, and the
server refuses every change to it until an admin lifts the mark.

## What is protected

The unit is the identity, `(kind, namespace, slug)`, not one version. A
protected `event/orders/placed` refuses an edit to version 1, a new version 2
and a delete of any version, including versions nobody has written yet.
Protecting a single version would leave the obvious bypass open: write the
next version and point everything at it.

The marks live in their own registry next to the store, not inside the
entity. An entity's content is whatever its writer sends, so a mark stored
there could be removed by sending the entity without it. Keeping the registry
separate means no content write can remove protection, whatever the
proposed entity says.

## Who sets and lifts it

Only an authenticated `admin` principal. `ProtectEntity` and
`UnprotectEntity` require the `admin` role in the method map, and the
handlers refuse again when there is no authenticated principal at all, so a
server started without a token registry cannot protect anything. The
principal that set a mark is taken from the request's credentials and stored
with it; a caller cannot name someone else as the setter.

A `ProtectionAnnotation` in an entity's metadata is a request to protect,
not a mark. The server always removes it from the stored entity. It honors
it only when an authenticated admin writes on baseline, and then protects
the identity in the same changeset as the write. A write whose body is
unchanged but which newly protects the identity is not a no-op: it records a
changeset and its receipt names it. An `operation_id` settles only once the
protection has landed, so when the registry fails after the content write, a
retry with the same id runs again and finishes protecting instead of
replaying a success that never protected anything. Anyone else's annotation,
and every annotation that arrives through `import-git`, is dropped without
effect.

Protection does not exempt admins. An admin's write to a protected identity
is refused like anyone else's; lifting the mark first is the deliberate,
audited step that makes the change possible.

## Where it is enforced

Every route that can change an entity looks up the protection of each
identity it would touch, one key per identity, inside the section where it
already holds the mutation lock. Nothing scans the registry and no lock is
held longer than the write already held it. `ProtectEntity` takes the same
lock for its own read and write, so a protect and a write cannot interleave:
whichever goes first wins, and the other sees its result.

| Route | Refused when |
| --- | --- |
| `PutEntity` | the identity being written is protected |
| `DeleteEntity` | the identity being deleted is protected |
| `DeleteByQuery` | any matched entity is protected |
| `BatchMutate` | any op targets a protected identity; the whole batch is refused |
| `RetargetReferences` | any referrer it would rewrite is protected |
| `MergeBranch` | the merge would write or delete a protected identity |
| `ResolveBranchEntry` | the resolution keeps the branch's side of a protected identity |
| `RevertChangeset` | the inverse would touch a protected identity |
| `import-git` | skips protected identities, and identities whose protection cannot be read, with a warning |

Dry runs answer the same way as the real call. The check runs before the
dry-run return on every route, so a plan that previews cleanly does not then
fail on protection when applied, and a refused preview means the apply will
be refused too.

The refusal is `FAILED_PRECONDITION` with reason `ENTITY_PROTECTED`, naming
each protected identity, its reason and its setter. See
[failure reasons](../reference/failure-reasons.md).

## When protection cannot be read

A lookup has three answers, not two: unprotected, protected, or unknown.
Unknown covers a row that does not decode, a row that names a different
identity than its key, and a registry that did not answer. A write treats
unknown as protected and is refused with `PROTECTION_STATE_UNKNOWN`, so a
caller can tell "this is protected" from "the server could not check".

Readers see the same distinction. `GetEntityProtection` returns a `state`
and, for an unknown identity, an `unknown` entry with the cause and the
revision of the unreadable row. `ListEntityProtections` returns unreadable
rows in `unknown` and sets `complete` to false when it could not enumerate
the whole registry, so a missing entry in an incomplete listing proves
nothing.

## Lifting by revision

Each protection row carries a `revision`. `UnprotectEntity` removes the row
only at the revision it read, and a caller may pass `if_match` to require the
revision it saw; a mismatch is refused with `STALE_PRECONDITION` and the
current revision. An unreadable row can be lifted the same way, which is how
an admin repairs it. A row whose state is unknown because the registry did
not answer has no revision and cannot be lifted until the registry answers.

## Audit

Protecting and lifting each record a changeset authored by the admin who did
it, with op kinds `KIND_PROTECTED` and `KIND_UNPROTECTED`. The protection row also
records the changeset that created it, so "who froze this, when and why" is
answered from the same log as every content change. See
[changesets](changesets.md).

## Clients

`trogon-atlas protection` and the `protect_entity`, `unprotect_entity`,
`get_entity_protection` and `list_entity_protections` MCP tools call these
RPCs. A protection read the server could not finish exits `11` from the CLI
and reports `verdict: "partial"` or `state: "unknown"`, so a script or agent
does not read a gap as "unprotected". See the
[command reference](../reference/entity-protection-commands.md) and
[how to protect a published entity](../how-to/protect-a-published-entity.md).

## Defaults chosen for review

- Default chosen for review: a server without an auth stack refuses
  `ProtectEntity`. Protection that anyone could lift would be a label, not a
  control.
- Default chosen for review: admins are refused like any other writer until
  they lift the mark.
- Default chosen for review: protect and lift are baseline-only. A branch
  header on either is refused; branch merges into baseline are what the mark
  guards against.
- Default chosen for review: protecting requires the identity to exist.
- Default chosen for review: a source run that carries a
  `ProtectionAnnotation` is refused outright, since source runs are synced
  rather than authored.
- Default chosen for review: `import-git` skips protected identities and
  drops every annotation, whoever runs it.
- Default chosen for review: a changeset that contains `KIND_PROTECTED` or
  `KIND_UNPROTECTED` ops cannot be reverted. `UnprotectEntity` and
  `ProtectEntity` are the explicit inverses.
- Default chosen for review: `ResolveBranchEntry` keeping the baseline side
  of a protected identity is allowed, because it leaves the identity as it
  is.
- Default chosen for review: replaying an `operation_id` that targets an
  identity protected since the first attempt is refused rather than
  answered from the stored result. `DeleteEntity` is the exception: it
  answers a replay from its receipt, because protection cannot apply to a
  delete that already happened.

- Default chosen for review: an identity whose protection cannot be read is
  refused like a protected one, with its own reason, rather than allowed.
- Default chosen for review: `import-git` skips an identity whose protection
  cannot be read, as it skips a protected one.
- Default chosen for review: an unreadable row is lifted with
  `UnprotectEntity`, which records a `KIND_UNPROTECTED` changeset like any
  other lift.
- Default chosen for review: `trogon-atlas protection list` exits `11` when
  the listing is partial and `protection get` exits `11` when the state is
  unknown, so a gate does not pass on an answer the server could not finish.
- Default chosen for review: a `GetEntityProtection` answer with no
  recognizable state, or `protected` without a row, is reported as unknown
  by the CLI and MCP tools, never as unprotected.
- Default chosen for review: the CLI and MCP tools refuse protect and
  unprotect while a branch is set, before sending anything.

## Not covered yet

- `export` does not write the annotation back out; a re-import would not
  restore protection, which is also what the import rule requires.
