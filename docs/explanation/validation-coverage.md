# Validation coverage and write scope

Validation findings are lists of
[`ValidationIssue`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto).
Coverage says which suites ran, against what scope, and whether the
snapshot was truncated. `ValidateEventModel`, `ValidateProject`,
`PutEntity`, `BatchMutate` and `MergeBranch` return
[`ValidationCoverage`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto)
alongside issues. Callers must not invent coverage client-side when it is
absent.

This page records what each entry point actually runs, and the shared
reporting contract for coverage and truncation. It does not expand
enforcement by itself: truthful reporting can land before any route runs
more checks.

## Check layers

Stable rule codes live in the server rule catalogue
(`ListValidationRules` / MCP `list_validation_rules`). Execution is grouped
into suites a coverage report can name:

| Suite | What it is | Typical emit shape |
| --- | --- | --- |
| `request_shape` | Per-entity structural and admission checks (`validate_entity_content`, context/project slug gates, type-library admit) | Usually hard `Status` refusal on write; rarely soft issues |
| `architecture` | Architecture resource rules and diagnostics | Write/landing: hard refusal; ValidateProject: soft issues on resource scan |
| `scenarios` | Gherkin / slice scenario rules | Errors refuse Put/Batch for applicable slice kinds; Warning/Info ride `validation[]` |
| `store_hygiene` | Cross-store consistency (`validate_store_hygiene`) | Soft issues; Merge treats Error as `STATUS_INVALID` |
| `model` | Per-EventModel content passes (`validate_model_content`) | Soft issues on Validate* |
| `strategic` | Bounded context / domain / subdomain chain findings | Advisory on Put/Batch/Merge; included in Validate* via strategic chain |

Preconditions (`create_only`, `if_match`, delete referrers) are write
admission, not validation coverage. They keep their own refusal wire
([stale plans](stale-plans.md), [failure reasons](../reference/failure-reasons.md)).

## What each entry point runs today

Inventoried at `23ade8f` (post [#263](https://github.com/TrogonStack/atlas/pull/263)).

| Entry point | Suites that run | Scope | Response today |
| --- | --- | --- | --- |
| `ValidateEventModel` | `model` always; strategic chain only when `EventModel.id` is set (ID-less inline bodies skip the chain) | Branch snapshot + visibility; in-body model or by-id | `issues[]` + `coverage` (`model` / `strategic`, `snapshot_truncated`) |
| `ValidateProject` | `store_hygiene` + architecture resource diagnostics once (including data-product Event confirmation); `model` per in-scope EventModel; strategic once | Project / domain / namespace | `reports[]` + severity totals + `coverage` |
| `PutEntity` (apply and `validate_only`) | `request_shape`; architecture admission on arch kinds; `scenarios` Errors on applicable slices; advisory `scenarios`/`strategic` | Slice/strategic use the branch snapshot | `validation[]` + `coverage`; hard refusals as Status |
| `BatchMutate` (apply and `validate_only`) | Same as Put for ops; strategic and scenario on overlay | Post-lock snapshot + overlay | Per-op `validation[]` / batch failure + `coverage` |
| `MergeBranch` (apply and dry-run) | Architecture landing on landed arch resources; blocking `store_hygiene` Errors; advisory `strategic` | Baseline visibility + projected post-merge world | `validation[]` + merge status + `coverage` |
| MCP `apply_manifests` | Client plan → `BatchMutate` | Client branch/scope | Tool JSON from batch results + batch `coverage` |

Write coverage always lists `model` and `store_hygiene` as `SKIPPED` with
reason `write_path`; merge coverage lists `model`, `scenarios` and
`request_shape` as `SKIPPED` with reason `merge_path`. Suites a request's
kinds never reach are `SKIPPED` with reason `kind_not_applicable`. When
blocking hygiene Errors stop a merge that lands strategic kinds, `strategic`
is `SKIPPED` with reason `blocked_by_hygiene`. A real batch admits architecture resources only after
its scenario gate, so a scenario failure reports `architecture` as `SKIPPED`
with reason `blocked_by_scenarios`. Merge reports the projected baseline as
`effective_scope` and the merged branch as `requested_scope`.

Coverage is absent on `operation_id` replays (nothing re-ran), on merge
`STATUS_CONFLICTS` (validation was never reached), and on batch failures
raised before validation, such as a referrer gate. A batch that fails after
validation, such as on a stale `if_match`, still reports the suites that ran.

### Explicit non-runs (do not overclaim)

- **Merge does not run** full `model` passes, storyboard/field-provenance
  rules, or write-path `scenarios`. Compositional breakage that only those
  suites detect is outside the merge gate until enforcement is expanded
  deliberately. See [branching](branching.md).
- **Put / Batch / apply dry-run do not run** EventModel-wide `model` or
  store hygiene. An empty `validation[]` after dry-run means "no advisory
  findings from the suites that write path runs," not "the model is
  clean."
- **ValidateEventModel does not run** store hygiene (that is
  ValidateProject) and does not re-scan every member through full
  `SCENARIO_*` emission the way write-path `validate_scenarios` does.
- **Coverage is not a finding.** `ValidateProject` can still surface an
  incomplete Event confirmation scan as the finding
  `DATA_PRODUCT_CONFIRMATION_INCOMPLETE` (budget or discovery exhaustion),
  which is an issue code rather than coverage metadata. Unrelated
  truncation exists elsewhere (projections, source coverage, delete
  `referrers_incomplete`).

## Coverage schema

Coverage stays separate from findings:

- A **completed** suite can still emit Error findings.
- A **skipped** suite must never be presented as a pass.
- A missing or default field must not imply `complete`.
- Suites that ran against a truncated snapshot are reported as `PARTIAL`
  with reason `snapshot_truncated`, and `ValidationCoverage.snapshot_truncated`
  is true.

Wire fields on `ValidationCoverage` / `ValidationCheckResult` /
`ValidationScope`:

| Field | Meaning |
| --- | --- |
| `check_id` | Stable suite id (`store_hygiene`, `model`, `strategic`, `architecture`, …) |
| `state` | `COMPLETED` / `SKIPPED` / `PARTIAL` / `ERROR` |
| `reason` | Why skipped/partial/error (for example `event_model_id_absent`, `snapshot_truncated`) |
| `subjects_examined` | Count when meaningful; zero is a valid completed empty scan |
| `effective_scope` | What store-wide suites actually examined. On ValidateProject this is the full visible snapshot (empty namespace/project/domain), because hygiene and architecture are not narrowed to the request filter |
| `requested_scope` | What the caller asked to validate (may be narrower than `effective_scope`) |
| `snapshot_truncated` | Shared with [snapshot identity](snapshot-identity.md) and the truncation package |

Clients (Rust, CLI, MCP) propagate the server message as a `coverage`
JSON object and must not invent coverage client-side.

### Incomplete validation and CLI exits

`validate` / `validate-project` exit `1` when any Error-severity finding
exists, `11` when there is no Error finding but coverage has a gap, and `0`
otherwise ([validation verdicts](../reference/trogon-atlas-exit-codes.md#validation-verdicts)).
This is a proposed default for review, not yet a settled product rule.

Errors take precedence because an Error finding is definitive: examining
more of the model cannot remove it, so a gate should fail on the finding
rather than on a coverage question. Without errors, an empty finding list
over a partial scan says nothing about the part that was not scanned, so it
must not exit like a clean pass. `11` is the next code after the failure
categories, and it is not a failure category itself because the command
completed and reported what it saw.

Absent coverage is treated as a gap rather than as complete, consistent with
"never invent coverage client-side": a client cannot tell an older server's
silent full run from a run that skipped suites. A CI job pointed at such a
server exits `11` until the server reports coverage. Coverage that is
present but lists no checks is a gap for the same reason: a default message
proves no suite ran.

A `SKIPPED` check is only a gap when the suite applied and did not run.
Reasons that mark a suite as outside the entry point by design are not gaps;
reasons that mark a suite as blocked, or reasons the client does not know,
are. Failing closed on unknown reasons means a new by-design reason needs a
client update before it stops reading as incomplete, which is preferable to
a new "blocked" reason silently reading as a pass.

### Enforcement is a separate decision

Reporting that Merge only ran hygiene does not by itself expand Merge to
`validate_model`. Expanding enforcement needs its own product answer
(whole post-merge model vs newly introduced errors vs current hygiene
subset). Coverage work makes that answer observable either way.

## Relation to truncation and consequence deltas

Truncation, depth, and visibility flags must use the same scope and
snapshot identity vocabulary as coverage, so a truncated empty issue list
cannot look complete. Read paths outside validation follow the same rule:
`ListEntities`, `ListVersions`, `ListEntitiesByDomain`, `GetImpact`,
`InferDataFlow` and `CheckInformationCompleteness` carry
`snapshot_truncated`, and projections and `ExtractSubgraph` fold the cut
load into their existing `truncated` flag. A short or empty answer under
either flag says nothing about the entities the load never reached. The
Rust client refuses to export a namespace from such a listing rather than
write partial manifests. Consequence deltas then compare findings only within
declared comparable scopes; an after-scan that omitted a suite or truncated
the snapshot cannot claim a finding was resolved.

## Where the code lives

- Catalogue and model/hygiene passes:
  `rsworkspace/crates/trogon-atlas-server/src/validation.rs`
- Handlers: `ValidateEventModel` / `ValidateProject` / Put / Batch / Merge in
  `rsworkspace/crates/trogon-atlas-server/src/service.rs`
- Wire: `proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto`,
  mutating responses in `service.proto`
- MCP apply: `trogon-atlas-mcp` → client `apply` → `BatchMutate`
