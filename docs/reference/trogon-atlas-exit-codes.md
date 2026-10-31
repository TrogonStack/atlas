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
| `1` | The command ran to completion but reports a finding rather than an error: a diff found drift, `fmt --check` found files that would change, a merge or rebase left conflicts, `validate` found an error. |
| `2` | Usage error: clap rejected the arguments before any command ran. |
| `3` | `stale_state` -- a plan was made against a revision the server no longer holds. Read fresh state and plan again. |
| `4` | `precondition` -- an etag or referrer check refused the request without staling a plan, or `export-attested` refused an incomplete export. |
| `5` | `validation` -- the request itself does not satisfy a rule the server enforces, or `verify` found a bundle malformed or altered. |
| `6` | `not_found` -- the target entity does not exist. |
| `7` | `unauthorized` -- the caller is not allowed to do this. |
| `8` | `incompatible` -- this client and server do not speak compatible contract revisions. |
| `9` | `operation_conflict` -- reserved; no current failure classifies here. |
| `10` | `partial_apply` -- a batch left entities in an intermediate state; see the named journal and `docs/how-to/replan-after-a-stale-refusal.md`-style recovery. |
| `11` | `validate` / `validate-project` found no Error finding, but the server's coverage shows the run did not examine everything it was asked to. See [validation verdicts](#validation-verdicts). Also `export-attested --allow-incomplete` and `verify` for an intact bundle marked incomplete; see [attested export](../explanation/attested-export.md). |
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

## Validation verdicts

`validate` and `validate-project` combine their findings with the server's
[validation coverage](../explanation/validation-coverage.md) into one
verdict:

| Verdict | Exit | When |
| --- | --- | --- |
| `findings` | `1` | At least one Error-severity finding, whatever the coverage says. |
| `incomplete` | `11` | No Error finding, and the coverage has at least one gap. |
| `clean` | `0` | No Error finding, and the coverage has no gap. Warnings and info findings alone stay `clean`. |

A coverage gap is any of:

| Gap | Cause |
| --- | --- |
| `coverage_absent` | The response carried no `coverage` (for example an older server). The client never invents coverage, so absent coverage is not complete. |
| `no_checks` | `coverage` was present but listed no checks, so it proves no suite ran. |
| `snapshot_truncated` | `coverage.snapshot_truncated` is true. |
| `check` | A check whose state is `PARTIAL`, `ERROR` or unspecified, or `SKIPPED` with a reason that is not by design. |

`SKIPPED` checks with reason `write_path`, `merge_path`,
`kind_not_applicable` or `event_model_id_absent` are by-design non-runs:
the suite is not part of that entry point, so skipping it leaves nothing
unexamined and is not a gap. Every other `SKIPPED` reason is a gap,
including `blocked_by_hygiene`, `blocked_by_scenarios`, an empty reason,
and any reason this client does not recognize.

Under `--format json`, `data` carries the same conclusion so a script does
not map exit codes itself:

| Field | Meaning |
| --- | --- |
| `verdict` | `clean`, `findings` or `incomplete`. |
| `coverage_gaps` | Each gap as `{"gap": "coverage_absent"}`, `{"gap": "no_checks"}`, `{"gap": "snapshot_truncated"}`, or `{"gap": "check", "check_id", "state", "reason"}`. Empty when complete. Gaps are listed even when the verdict is `findings`. |
| `coverage` | The server's coverage, passed through unchanged; absent when the server sent none. |

Text output prints one `incomplete:` line per gap after the findings.
