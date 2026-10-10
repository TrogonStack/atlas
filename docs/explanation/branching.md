# Branching: developing model changes in isolation

Status: **ALL PHASES SHIPPED.** Branches are a full capability: isolation, review, merge, and studio preview. Isolation
is not a response to the first collision; it is what makes concurrent
writers (humans and agents) safe at all. The requirements below are the
contract; the working decisions section records the defaults we build
with, each reversible if reality disagrees.

## What we are trying to do

We want to develop a set of related changes to an event model in isolation
(a "branch"), see that work rendered live in the studio while it is in
progress, review it as a diff against the live model, and then integrate it
atomically, with conflicts detected when the live model moved underneath
us. And we want all of that **without relying on git as the mechanism**.

The store is the source of truth. Humans push into it through `trogon-atlas`
manifests; agents push into it through the MCP tools, often mid
conversation; the studio will eventually be a write surface too. A
branching model that only works for the file-based surface (git branches
of manifests) leaves the other surfaces with nothing, and we have already
committed to the rule that every surface goes through the same code except
for what makes it a different surface. Branching must therefore be a
capability of the server, expressed once, worn by every surface.

Git stays available as *workflow*: a team that wants GitOps can keep
manifests in a repo and apply them from CI. Git is never the *mechanism*:
nothing about creating, previewing, reviewing, or merging a branch may
require a git repository.

## Why now

Everything shipped recently sharpened the gap:

- **Manifests + `trogon-atlas apply` + the MCP declarative tools** made writing
  declarative and idempotent, but writes go straight to the live store.
  There is no place to put work-in-progress that other people, agents, and
  the studio should not yet see.
- **Semantic idempotency** (server-side no-op puts) and **merged-view batch
  validation** were built for atomic whole-model applies. They are exactly
  the primitives an overlay-style branch needs; the machinery exists, the
  concept does not.
- **Agents work in parallel.** Two agents modeling different concerns today
  either collide in the live store or serialize. Isolation is what lets
  them run concurrently and lets a human review before anything lands.

## What exists today, and why it is not enough

- **[Draft-then-land](../how-to/draft-then-land.md)** is the current
  documented workflow: author under a scratch identity (scratch namespace
  or `.draft` slug) with a `LifecycleAnnotation`, validate in a scratch
  EventModel, then land the real version and retarget referrers. It works,
  per entity. Its limits are structural: scratch identities pollute the
  permanent (slug, version) space; a multi-entity change has no shared
  container; there is no "render the whole model as it would look with my
  changes"; landing is a manual choreography per entity, not an atomic
  merge; and nothing detects that the baseline moved while you worked.
- **Versioning + `supersedes` + stale-ref detection** handle the
  *aftermath* of change well (mid-flight migrations are visible and
  trackable), but not the *development* of change. They begin working only
  after the new version is already public.
- **Git branches of manifests** give isolation for the file surface only,
  make the studio blind to work in progress, and are unusable by agents
  editing through MCP mid-conversation. This is the option we explicitly
  rejected as a long-term answer.

## Requirements

The design, whatever it is, must satisfy these:

1. **Isolation.** A branch holds a set of related changes spanning many
   entities (and possibly several namespaces) without any effect on what
   the live model's readers see.
2. **One implementation.** Branch operations are server capabilities,
   consumed identically by `trogon-atlas`, the MCP tools, and later the studio.
   No surface-specific branching logic.
3. **Live preview.** The studio can render a branch: the live model with
   the branch's changes layered on, indistinguishable in fidelity from
   rendering the live model itself.
4. **Reviewable.** A branch can be diffed against the live model at any
   time, semantically (canonical JSON, not bytes), including "what would
   conflict if merged now".
5. **Atomic merge with conflict detection.** Merging applies every change
   or none. A conflict exists when the baseline entity changed since the
   branch last saw it AND the two results differ semantically. Byte noise
   must never manufacture a conflict.
6. **Valid at the seam.** A merge may not leave the live model in a state
   the server's own write-time validation would have rejected. Whether a
   branch may be internally invalid *while being worked on* is an open
   question below.
7. **Identity is preserved.** Merging must not re-mint incarnation
   identity: an entity created on a branch keeps its uid across the merge;
   an entity edited on a branch keeps the baseline's uid. Version counters
   and `supersedes` chains behave exactly as they do on the live model.
8. **Parallel work.** Many branches exist at once, owned by different
   humans and agents, with no interference between them.
9. **Auditable.** Branch merges appear in the change feed and the git
   mirror the way any write does. (Whether branch work-in-progress is
   mirrored is an open question below.)
10. **Cheap.** Creating a branch costs nothing proportional to the size of
    the store; only divergence occupies space.

## Non-goals

- Reimplementing git. No rebase, cherry-pick, or branch-off-branch in the
  first design. One level: baseline and branches off it.
- Locking. Two branches may touch the same entity; the second merge sees a
  conflict. No reservations, no pessimistic locks.
- Replacing draft-then-land's annotations. `LifecycleAnnotation` remains
  meaningful on the live model (a landed entity can still be marked
  proposed); branches are about *where* work happens, not entity lifecycle.

## Merge conflicts

Detection alone is half a design; this section is the other half.

**Conflict classes.** Four, and the fourth has no git analog:

1. *Edit/edit*: baseline and branch both changed the same entity, and the
   results differ semantically. (If they converged on the same content,
   that is not a conflict; semantic equality already knows.)
2. *Edit/delete*: the branch deleted what the baseline meanwhile changed.
3. *Delete/edit*: the branch changed what the baseline meanwhile deleted.
4. *Compositional invalidity*: no entity conflicts at all, but the merged
   world fails the server's own validation. Baseline added a slice
   referencing an entity the branch deletes; both sides gave the same
   command an issuer. Three-way comparison cannot see this class. Only
   validating the whole post-merge world catches it, which is why the
   merge gate always runs the dry-run overlay validation, even for
   "clean" merges.

**Resolution happens on the branch, never inside the merge.** Merge is
all-or-nothing: it either lands everything or reports conflicts and
changes nothing. Resolving is ordinary branch work, not a special mode:
take the baseline's version by dropping the branch delta, keep the
branch's version, take the structural merge of the two, or write a
hand-merged entity onto the branch, and in every case the key's recorded
base refreshes to the current baseline.
Merge again, now clean. There are no half-merged states, no resolution
write path distinct from the normal one, and every surface renders the
same conflict payload: per entity, `{base, ours, theirs}` as canonical
JSON.

**A resolution is bound to the conflict it decided.** Every reported entry
carries the revisions of its three sides, and `ResolveBranchEntry` takes
them back: if the baseline or the branch moved after the entry was
inspected, the resolution is refused and the conflict stays as it was. See
[stale plans](stale-plans.md).

**Structured data lets resolution improve in stages.**

- *Stage 1, entity-level*: pick ours, pick theirs, or write the merged
  entity by hand. Coarse but complete.
- *Stage 2, field-level* (SHIPPED): entities are typed, so a true
  three-way structural merge is possible. Baseline edited a doc while the
  branch added a scenario: non-overlapping field paths, deterministic
  auto-merge. Only same-field-path changes remain true conflicts. Even in
  stage 1, conflicts should be *reported* at field-path granularity so
  the human sees "you both touched `scenarios[2].then`" rather than two
  walls of JSON.

  The merge runs over the same canonical proto3 JSON that content hashing
  and semantic equality already use, rather than over 23 hand-written
  per-kind mergers: one rule, agreeing with the rest of the system by
  construction, and covering kinds added later for free. Lists are merged
  whole rather than by index. Position `i` on two sides is not the same
  member once either side inserts, and a merger that assumes otherwise
  interleaves two authors' lists into something neither wrote. Merging
  lists by member identity is stage 3's job.

  Every conflicting entry carries its structural merge as `auto_merged`
  and stays classified a conflict. Nothing consumes it implicitly:
  `MergeBranch` lands it under `auto_merge`, and `ResolveBranchEntry`
  writes it onto the branch under `take_merged`. Combining two authors'
  edits without either of them seeing the result is a judgement call, so
  the caller makes it rather than the server. When it lands, it lands
  guarded by the baseline revision it was merged against, so a write that
  slips in first fails the whole merge instead of overwriting a revision
  nobody merged against.
- *Stage 3, schema-aware*: per-field merge strategies. Member lists merge
  as sets; scenario lists merge by scenario id; annotation lists merge by
  type. Declared in one place, worn by every surface.

One design consequence must be locked in from the start regardless of
stage: field-level anything requires the *base value*, not merely a base
etag. A branch therefore copies the baseline entity into itself at first
touch (copy-on-write). This also makes branch diffs self-contained and
costs only what actually diverged.

**Long-lived branches reconcile incrementally.** An `update` operation
(the pull/rebase analog, and the one loosening of the "no git
reimplementation" non-goal) refreshes the branch's recorded base against
the current baseline: non-conflicting keys advance silently, conflicting
keys surface for resolution on the branch immediately. Conflicts are then
paid for as they appear instead of as a wall at merge time.

**A branch also knows where it forked from.** The copy-on-write base is
per key and captured at first touch, so the bases on a branch's deltas were
taken at different moments and get rebased individually. They answer "did
THIS key move under me" and nothing larger. Adding them up does not produce
a statement about the branch.

So a branch additionally records a **fork point**: the id of the newest
baseline changeset at the moment it was created (`BranchInfo`
`fork_changeset_id`). Changeset ids are UUIDv7, so a fork point is a
*position* in baseline history, and "what landed after this branch forked"
is a comparison of two positions instead of a walk over every delta.

`DiffBranch` reports that as `BranchAncestry`: the fork point plus the
baseline changesets recorded after it, newest first, capped (with
`behind_truncated` when the branch is further behind than the cap). The walk
costs what the branch is behind, not what the log holds.

`UpdateBranch` advances the pointer to the newest baseline changeset, **but
only when it leaves no conflicts**. A branch with an open conflict has not
reconciled every baseline write; moving its fork point past them would erase
exactly the evidence a reviewer needs. The response echoes the resulting
`fork_changeset_id` so a caller can tell whether it moved.

`MergeBranch` with `keep_branch` moves the pointer to the merge it just
produced. The kept branch's deltas were purged, so it has nothing left to
reconcile, and leaving the pointer where it was would report the branch as
behind by its own merge.

Two honest limits. A branch created before this field existed, or cut from a
store with no attributed write history, has no fork point and reports empty
ancestry rather than claiming the whole log landed after it. And writes that
mint no changeset (`trogon-atlas-server import-git`, direct `Store` use) are
invisible to the pointer, the same unattributed-write caveat the
[changeset log](changesets.md) carries everywhere else. Per-key bases remain
the fast path for conflict detection either way, so neither limit can make a
merge unsafe: it can only make an ancestry answer optimistic.

**Agents are first-class resolvers.** A conflict here is a well-posed
structured task: given `{base, ours, theirs}` for a typed entity, produce
the merged entity. That is exactly the shape of task a model agent handles
well, with the human approving the result. The conflict payload is
designed for this consumer as much as for the studio's three-pane view.

## Current leaning (recorded, not decided)

An overlay: a branch is a small server-side store holding only deltas
(puts and tombstones, each with a copy-on-write snapshot of the baseline
value at first touch) over the baseline. Branch context travels as
connection metadata through the shared client, so every existing RPC works
on a branch unchanged. Reads resolve branch-first, then baseline. Writes
on a branch validate against the merged view and reuse semantic no-op
detection, which means a branch physically contains only true divergence.
Merge is a first-class server RPC under the existing mutation lock: it
three-way-compares (recorded base, baseline now, branch), validates the
post-merge world with the dry-run overlay machinery, and lands as one
atomic batch, then deletes the branch.

## Working decisions (day-zero defaults, each reversible)

- **Scope: global.** A branch can touch any namespace; models cross
  namespaces through seams and a change touching two contexts must merge
  atomically.
- **Naming: flat, owner-prefixed by convention.** `alex/retention-rework`
  is a name, not a hierarchy. Same character rules as id components, plus
  `/`.
- **Retention: no expiry.** Branches live until deleted; `ListBranches`
  shows age and staleness so dead ones are visible.
- **Validation: same strictness as live.** A branch write passes exactly
  the validation a live write would, evaluated against the merged view.
  One behavior everywhere beats a second write mode; relaxing this is a
  future decision with its own trigger (authoring friction in practice).
- **Work-in-progress is invisible outside the branch.** No mirror commits,
  no change-feed events, no search indexing for branch writes. Merging
  produces ordinary baseline writes with all side effects. The mirror
  remains the audit trail of what is real.

  Searching *under* a branch header therefore cannot use the live index. It
  searches an index built over the merged view instead, cached per branch
  and keyed by a hash of the view it was built from, so a branch that has
  not moved is searched rather than re-indexed. The hash is the whole
  invalidation story: branch writes deliberately bypass every baseline
  cache-invalidation path, so nothing else could tell the cache it went
  stale. A handful of branches stay resident; the rest rebuild on demand.

  The namespace registry is the one thing a branch write does touch outside
  its branch, and it is bounded so that it still keeps this promise. Writing
  into a namespace nobody has registered claims it, because otherwise the
  author's own registry-filtered reads could not see the work. The claim is
  provisional: deleting the branch releases the name, and merging makes it
  permanent. See "Claims made on a branch" in
  `docs/explanation/authorization.md`.
- **Studio is read-only on branches, as everywhere.** People preview and
  inspect a branch there; agents create, update, resolve, and merge it
  through the CLI or MCP.
- **Store-wide sweeps are branchable, and copy what they touch.**
  `RetargetReferences` and `DeleteByQuery` honor the branch header like any
  other write: they scan the merged view and land their results as deltas,
  so a version migration (mint `@2`, retarget every referrer, drain the
  stale references) is one reviewable change set rather than a sequence of
  irreversible baseline writes.

  A referrer that exists only on baseline is still a referrer the branch
  sees, so rewriting it copies it into the branch. Skipping it instead
  would leave the branch's own merged view holding references to the entity
  the migration exists to move away from, which is the opposite of what was
  asked for. This is ordinary copy-on-write; it is only worth stating
  because a sweep touches keys the author never named.

  Namespace scope is unaffected: a branch is a staging area for baseline,
  so a principal that may not write a namespace directly may not write it
  through a branch either. `RetargetReferences` in particular stays refused
  for scoped principals on a branch, since nothing bounds its reach in
  advance. See [authorization](authorization.md), "Namespace scope".

  Ownership is a harder boundary and branches do not clear it yet. A branch
  and its diff span every namespace at once, and there is no owner on either
  to filter by, so a principal bound to a `parent` is refused
  `ListBranches` and `DiffBranch` outright rather than shown a diff that
  silently omits half of what would land. Giving a branch an owner is what
  lifts that. See [authorization](authorization.md), "Ownership".
- **Auto-merge is opt-in, and only ever field-level.** Structural
  auto-merge (stage 2) covers edit/edit conflicts whose two sides changed
  disjoint field paths. Same-path collisions stay conflicts, as do
  edit/delete and delete/edit: keeping versus dropping a key has no
  field-level reading to combine.
- **Anyone with write access may resolve and merge.** Review gates are team
  convention, not mechanism, until proven otherwise.
- **Baseline protection: opt-in server flag; when on, only merges and
  admin principals change baseline.** It is consultable per namespace
  (`--protect-baseline-namespaces=orders,shop-*`), so a context under review
  and a sandbox can share one store. `--protect-baseline` alone still means
  every namespace. See [authorization](authorization.md), "Namespace scope".

## Delivery phases

1. **Isolation (SHIPPED).** Branch create/list/delete; branch context on
   every RPC via the `x-trogon-atlas-branch` metadata header; reads resolve
   branch-first; writes land in the branch with merged-view validation and
   semantic no-op detection; `trogon-atlas --branch` plus branch subcommands; MCP
   `--branch` and lifecycle tools. Concurrent writers can no longer hurt
   each other.
2. **Review and merge (SHIPPED).** `DiffBranch`, `MergeBranch`
   (all-or-nothing, three-way conflict detection, post-merge world
   validation, identity preserved across the merge), `UpdateBranch` for
   incremental reconciliation, `ResolveBranchEntry` (take-theirs /
   keep-ours / take-merged), conflict payloads with field-path reporting
   and the stage-2 structural merge per the merge conflicts section;
   `trogon-atlas branch diff/merge/update/resolve` and the matching MCP tools.
3. **Studio (SHIPPED).** The URL carries the branch (`?branch=`); the
   bridge forwards it as the same `x-trogon-atlas-branch` header every other
   surface uses; branch picker and read-only banner in the shell; added /
   changed / conflict badges on stickies; three-pane base/ours/theirs
   conflict drawer with field paths. Read-only by design (working
   decision): resolution stays with trogon-atlas and the MCP tools until studio
   editing is its own considered step.

## Litmus test

The model records decisions and data, never implementation, so this design must survive reimplementing the system
in another language. A branch is data (deltas plus a base cursor), its
operations are RPCs, and no behavior may depend on git, the filesystem, or
any single surface's tooling.
