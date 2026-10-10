# Validation repair workflow

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
