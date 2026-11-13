# Backing evidence

A model is easier to trust when its entities point at the reasoning behind
them: the Decision that chose them, the Document that describes them, the
code that implements them. The [backing report](../reference/backing-report.md)
measures how much of a model has such a pointer. This page explains what
that number does and does not mean.

## Presence, not truth

The report counts whether evidence exists, not whether it is right. An
accepted Decision may be obsolete in practice, a Document may describe
something else, and a code reference may point at a file that moved. Each
of those still counts as backing. Judging them takes reading them, which is
work for a reviewer, not a deterministic server check. Keeping the report
to presence means two servers given the same snapshot always agree on it.

Only adopted reasoning counts. A proposed Decision has not been accepted
yet, and a rejected, withdrawn or superseded one no longer describes the
model, so none of them back anything. When a Decision is superseded, its
subjects stay backed only if the accepted replacement names them too.

## Unknown is not absent

The report never turns "could not see" into "not there". A caller limited
to some namespaces cannot rule out a Decision in a namespace it cannot
read, and telling it whether one exists would leak that namespace's
contents. So for a restricted caller, a subject without visible Decision or
Document evidence is unknown, not unbacked. The same holds when the
snapshot was truncated, because the missing rows may hold the evidence, and
when a code reference is present but unreadable.

An unknown subject withholds the rate. A rate over a partial view looks
exactly like a rate over the full one, and a gate reading it could pass or
fail on rows it never saw. The CLI exits `11` for the same reason
`validate` does when its coverage is incomplete.

## One subject, one count

Evidence piles up: an entity can have two Decisions, a Document and a code
reference. The per-category counts show each kind of evidence, but
`backed` counts the entity once. Otherwise adding a second Document would
raise the backed share without backing anything new.

## Bound to a snapshot

The report carries the [snapshot id](snapshot-identity.md), branch and
visibility it was computed under. Two reports are comparable only when
those match, and a report quoted later can be checked against the state it
described.
