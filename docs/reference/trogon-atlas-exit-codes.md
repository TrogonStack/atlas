# trogon-atlas exit codes

`trogon-atlas`'s exit code tells a calling script or agent what happened without
parsing text: a found-but-not-wrong result (a diff with drift, `fmt
--check` reporting non-canonical files) exits differently from an actual
failure, and every failure exits by its `FailureCategory`
(`docs/reference/failure-reasons.md`), never by which subcommand produced
it.

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | The command ran to completion but reports a finding rather than an error: a diff found drift, `fmt --check` found files that would change, a merge or rebase left conflicts. |
| `2` | Usage error: clap rejected the arguments before any command ran. |
| `3` | `stale_state` -- a plan was made against a revision the server no longer holds. Read fresh state and plan again. |
| `4` | `precondition` -- an etag or referrer check refused the request without staling a plan. |
| `5` | `validation` -- the request itself does not satisfy a rule the server enforces. |
| `6` | `not_found` -- the target entity does not exist. |
| `7` | `unauthorized` -- the caller is not allowed to do this. |
| `8` | `incompatible` -- this client and server do not speak compatible contract revisions. |
| `9` | `operation_conflict` -- reserved; no current failure classifies here. |
| `10` | `partial_apply` -- a batch left entities in an intermediate state; see the named journal and `docs/how-to/replan-after-a-stale-refusal.md`-style recovery. |
| `70` | `internal` -- an unclassified server or local error. |
| `75` | `unavailable` -- the condition is expected to clear; retry the same request. |

A local validation failure caught before `trogon-atlas` ever connects to a server
(a missing manifest, a malformed branch name, an empty namespace) also
classifies through `Failure::classify` and exits `5` (`validation`): these
are `InvalidInput` errors, the same category and `fix_input` next action a
server-side validation rejection gives, so a caller does not need to tell
"my request was malformed" apart by whether a connection was ever made.

Under `--format json`, the same category is also on the printed
`CommandOutcome.failure`, so a script that wants the reason rather than
just the exit code does not have to maintain its own lookup table.
