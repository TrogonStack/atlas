# Consequence deltas

A dry run answers "would this write be accepted?". A reviewer usually also
wants to know "what would it do to the model's health?": which findings it
creates, which it clears, and which it leaves alone. A consequence delta
answers that second question by comparing the findings of the world as it
is with the findings of the world the change would produce.

The delta is advisory. It never refuses a write and never changes the
status a dry run would otherwise return. It rides the dry-run responses of
`PutEntity`, `BatchMutate` and `MergeBranch`, and the matching MCP tools.
Field-level detail lives in the
[consequence delta reference](../reference/consequence-delta.md).

## Full-scope comparison is the reference

The server scans the whole visible world twice, once before and once after
the change, with the same suites, and compares the two finding sets. It does
not try to work out which findings a change could reach. That kind of
narrowing is attractive, since most changes touch a small corner of the
graph, but a narrowing that misses a reverse reference silently drops a
real consequence. Full scope is the behaviour any later optimization must
reproduce exactly, and it is the only behaviour shipped today.

Running every suite over the whole world costs time on every dry run, twice,
and the per-model suite grows faster than the world does because each model
pass reads the whole snapshot. The server records that cost on the
`trogon_atlas.consequence_delta.duration` histogram, so an operator can see
when a narrower comparison becomes worth building.

Because each scan is a flat pass over a snapshot rather than a walk along
references, reference cycles cannot make it loop.

## What makes two findings the same

Prose changes between releases and must not turn one finding into two, so a
finding's identity is built only from structured data:

- the suite that produced it,
- the entity whose pass produced it, for suites that run once per entity,
- the rule code,
- the exact subject entity, including its version,
- the subject field,
- every remaining structured field of the issue.

The producing entity matters because a per-model check reports its finding
on the member it is about, not on the model. Without it, two models that
both point at the same missing event would collapse into one finding, and
deleting one of them would look like a fix.

Severity is deliberately outside identity, so a rule that is promoted from
warning to error shows up as one finding whose severity changed rather than
one resolved and one introduced. The rule title, category and default
severity are copies of catalogue data and are excluded too.

"Every remaining structured field" is computed from the issue message itself
rather than from a fixed list. When `ValidationIssue` gains structured
evidence, that evidence joins identity without any change to the delta, so
two findings of one rule on one subject that differ only in evidence stay
distinct.

## How findings are classified

Findings are grouped by identity on each side and then matched:

- Occurrences with equal identity and equal severity cancel out and are
  counted as unchanged.
- Leftover occurrences with equal identity but different severity are a
  severity change.
- A finding that appears on a newer version of its subject or producing
  entity, when the same finding existed on an older version, is carried to
  the successor rather than introduced.
- A finding whose subject or producing entity existed before and is gone
  after left with that entity, which is not the same as being fixed.
- Anything else that appears is introduced, and anything else that
  disappears is resolved.

Repeated occurrences of one identity are counted, not collapsed, so a
change that adds a second copy of an existing finding is reported.

## Unknown is not resolved

A finding can only be called resolved if the after-scan actually looked
for it. If the suite that produced it did not complete after the change
(skipped, partial, errored, or simply not reported), its absence proves
nothing, and the delta reports it as unknown with the reason the suite
gave. The same holds the other way: a finding only counts as introduced if
the before-scan's suite completed, otherwise it might have been there all
along.

A truncated snapshot marks every compared suite partial, so a delta over a
truncated world classifies nothing as introduced or resolved. The delta's
`complete` flag is true only when every compared suite completed on both
sides, neither snapshot was truncated, and no applicable suite was left
out.

Both scans see only what the caller may read. An entity hidden from the
caller contributes no findings to either side, so the delta says nothing
about it rather than calling its findings resolved. A finding whose subject
was deleted is reported as removed with its subject, never as resolved.

## Suites the delta does not compare

Two parts of validation read the store directly rather than a candidate
world: specification type checks and data-product event checks. The delta
cannot evaluate them against a world that does not exist yet. When the
world holds entities those checks apply to, the delta names them as skipped
with `delta_not_supported` and reports itself incomplete. When it does not,
they are skipped as not applicable and do not affect completeness.

Data-flow inference and information-completeness checks are scoped
analyses, optionally backed by a language model, so two runs over one world
need not agree. They are listed as skipped with `analysis_not_compared` and
do not affect completeness, because they are not validation suites.

## Binding to the worlds compared

Each delta carries the snapshot id of both worlds, computed the same way as
[`GetSnapshotId`](snapshot-identity.md). The before id names the state the
preview started from; the after id names the content that would result. A
delta whose before id no longer matches the live snapshot describes a
world that has moved on. When either world holds an entity that cannot be
rendered canonically, it has no snapshot id; the delta still reports its
findings but leaves both ids empty and calls itself incomplete, because a
comparison that cannot name its worlds cannot be checked for staleness.

The comparison runs after the dry run releases the mutation lock. The lock
only covers the same admission checks a real write would make; the scans
read worlds already held in memory, so a slow delta never delays other
writers. The price is that a concurrent write can land while the delta is
being computed, which is exactly what the before snapshot id lets a reader
detect.

## Default chosen for review

The delta compares full scope only and reports applicable suites it cannot
evaluate as gaps rather than approximating them. Both choices trade cost and
coverage for never reporting a resolution that was not proven.
