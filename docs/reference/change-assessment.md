# Change assessment

Wire contract for
[`ChangeAssessment`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto).
For the reasoning behind it, see the
[change assessment explanation](../explanation/change-assessment.md).

## Routes

| Route | Field | Set when |
| --- | --- | --- |
| `MergeBranch` | `MergeBranchResponse.change_assessment` | `dry_run` is true |
| `trogon-atlas branch merge` | `change_assessment` in the JSON output | `--dry-run` |
| MCP `merge_branch` | `change_assessment` in the structured result | `dry_run` is true |

The assessment is advisory. It never changes the response status or
refuses a merge.

## `ChangeAssessment`

| Field | Meaning |
| --- | --- |
| `verdict` | See the verdicts table |
| `reasons` | Reason codes, each at most once, in enum order. Empty only for `VERDICT_LAND` |
| `binding` | The state the verdict judged |

## Verdicts

| Verdict | JSON | Meaning |
| --- | --- | --- |
| `VERDICT_LAND` | `land` | Admitted, nothing introduced or worsened, every comparison complete |
| `VERDICT_NEEDS_REVIEW` | `needs_review` | Admitted, but at least one reason applies |
| `VERDICT_REJECT` | `reject` | `REASON_CONFLICTS` or `REASON_BLOCKING_FINDINGS` applies |

## Reasons

| Reason | JSON | Applies when |
| --- | --- | --- |
| `REASON_CONFLICTS` | `conflicts` | The branch conflicts with baseline |
| `REASON_BLOCKING_FINDINGS` | `blocking_findings` | The landed world fails a blocking validation check |
| `REASON_FINDINGS_INTRODUCED` | `findings_introduced` | The delta reports an introduced finding of any severity |
| `REASON_FINDINGS_WORSENED` | `findings_worsened` | A finding's severity rises, including one carried to a successor |
| `REASON_FINDINGS_UNKNOWN` | `findings_unknown` | The delta could not classify a finding |
| `REASON_DELTA_MISSING` | `delta_missing` | The dry run produced no consequence delta |
| `REASON_DELTA_INCOMPLETE` | `delta_incomplete` | The delta's `complete` is false |
| `REASON_INVARIANTS_INCOMPLETE` | `invariants_incomplete` | The invariant check's `complete` is false |
| `REASON_UNBOUND` | `unbound` | Either snapshot id in the binding is empty |

A conflicting dry run reports only `REASON_CONFLICTS`, because nothing past
conflict detection runs.

## `AssessmentBinding`

| Field | Meaning |
| --- | --- |
| `branch` | The branch being merged |
| `baseline_snapshot_id` | The delta's `before_snapshot_id`. Empty when there is no delta |
| `landed_snapshot_id` | The delta's `after_snapshot_id`. Empty when there is no delta |
| `entries` | Every branch entry that would land, sorted by kind then id |
| `scope` | The caller's read scope: `*` for unrestricted, otherwise `parent=<owner>` |
| `catalogue_revision` | `sha256:` followed by the hex digest of the assessment rules and the validation rule catalogue |

## `AssessmentPin`

| Field | Meaning |
| --- | --- |
| `ref` | The entity the branch would land |
| `baseline_etag` | The etag of the entity on baseline. Empty when baseline lacks it |
