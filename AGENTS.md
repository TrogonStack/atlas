# Atlas agent guidance

## Principles and progressive disclosure

At the start of a task, read the
[principles overview](docs/explanation/principles/README.md) if it is not
already in the current context. Use it to identify the constraints relevant
to the work, then read the matching principle pages before making those
decisions. Follow their links to detailed contracts and procedures as needed.

| Decision area | Principle to read |
| --- | --- |
| Authoritative state, persistence, projections, or caches | [Backend authority](docs/explanation/principles/backend-authority.md) |
| Connectivity, client responsibilities, or local hosting | [Connected clients](docs/explanation/principles/connected-clients.md) |
| Agent tools, task execution, or human-facing workflows | [AI-first work](docs/explanation/principles/ai-first.md) |
| Graph relationships, information layers, validation, or incomplete models | [Layered information](docs/explanation/principles/layered-information.md) |
| Concurrent participants, branches, conflicts, or attribution | [Multiplayer](docs/explanation/principles/multiplayer.md) |
| Role-specific views, preferences, or shared completion criteria | [Multiple roles](docs/explanation/principles/multiple-roles.md) |
| Modeling primitives, behavior, causal order, or additional overlays | [Event Modeling](docs/explanation/principles/event-modeling.md) |

Revisit the overview when scope changes or context has been compacted or
lost. Read additional pages when a decision reaches a new area. Before
final review, check the affected decisions against the relevant principles.
Read the full set when a change spans the product broadly; focused work
needs only the relevant pages. Verify current capabilities in the linked
contracts and implementation rather than treating a principle as evidence
that a feature has shipped.

## Validation repair workflow

Before repairing a model validation finding, read
[the repair playbook](docs/how-to/repair-validation-findings.md) and check
[the recorded repair patterns](docs/explanation/validation-repair-patterns.md).
Studio remains a review surface; use Atlas CLI/MCP for model changes.

Use the existing reference and impact tools to discover dependencies. Fetch
exact pinned versions and preserve storyboard order and repeated occurrences.
Validate the current scope before editing, preview guarded mutations, and
validate again after applying them.

Before ending a repair task, preserve the procedure, reasoning, commands or
scripts, and verification evidence. Keep reusable procedures in the playbook
and add new decision patterns to the pattern document. Store private run
records in a git-ignored directory at the git root, using descriptive,
collision-safe filenames. Include any temporary script's source in the
record before its temporary directory disappears.

Documentation is a valid first implementation of a workflow. Extend the
recorded procedure as repairs reveal repeated steps, then move deterministic
steps into maintained, tested tools. Do not leave the only reusable knowledge
in a chat, memory entry, session note, or temporary script.
