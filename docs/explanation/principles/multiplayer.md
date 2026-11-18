# Multiplayer is the default

Part of the [Atlas principles](README.md).

The model is shared by people, individual agents, and swarms of agents
working concurrently. A feature must account for multiple participants,
tasks, and proposals from the start. It cannot assume one active person or
one agent owns the whole session or graph.

Isolated work, live inspection, reviewable diffs, and explicit conflict
handling let participants develop ideas without overwriting one another.
Every proposed mutation is a decision about observed state. An absent
entity and a particular revision are distinct conditions; typed revisions
and preconditions preserve that distinction. If relevant state moved, the
agent must read and decide again.

Authority and attribution remain explicit for every participant. Shared
work must retain enough evidence to understand who acted and what changed
together, within the documented history and recovery limits. Multiplayer
describes concurrent participants; storage writer coordination remains a
backend responsibility. See [branching](../branching.md),
[stale plans](../stale-plans.md), [changesets](../changesets.md), and
[single-writer coordination](../single-writer.md).
