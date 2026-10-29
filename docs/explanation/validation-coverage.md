# Validation coverage and write scope

Validation responses today are lists of
[`ValidationIssue`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto)
findings. They do not say which check suites ran, against what scope, or
whether the underlying snapshot was truncated. Callers infer coverage only
from which RPC they invoked. That is not enough for agents, CI, or
dry-run/apply consumers that must refuse to treat an empty issue list as
"everything was checked."

This page records what each entry point actually runs on current `main`,
and the reporting contract coverage and truncation work must share. It is
the inventory and design target for the Open Ontologies adoption Tier 1
packages (coverage, then bounded results). It does not expand enforcement
by itself: truthful reporting can land before any route runs more checks.

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
| `ValidateEventModel` | `model` always; strategic chain only when `EventModel.id` is set (ID-less inline bodies skip the chain) | Branch snapshot + visibility; in-body model or by-id | `issues[]` only |
| `ValidateProject` | `store_hygiene` + architecture resource diagnostics once (including data-product Event confirmation); `model` per in-scope EventModel; strategic once | Project / domain / namespace | `reports[]` + severity totals |
| `PutEntity` (apply and `validate_only`) | `request_shape`; architecture admission on arch kinds; `scenarios` Errors on applicable slices; advisory `scenarios`/`strategic` | Slice/strategic use the branch snapshot | `validation[]`; hard refusals as Status |
| `BatchMutate` (apply and `validate_only`) | Same as Put for ops; strategic and scenario on overlay | Post-lock snapshot + overlay | Per-op `validation[]` / batch failure |
| `MergeBranch` (apply and dry-run) | Architecture landing on landed arch resources; blocking `store_hygiene` Errors; advisory `strategic` | Baseline visibility + projected post-merge world | `validation[]` + merge status |
| MCP `apply_manifests` | Client plan → `BatchMutate` | Client branch/scope | Tool JSON from batch results |

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
- **No validation RPC** reports suite coverage or snapshot truncation.
  `ValidateProject` can still surface an incomplete Event confirmation
  scan as the finding `DATA_PRODUCT_CONFIRMATION_INCOMPLETE` (budget or
  discovery exhaustion), which is an issue code rather than coverage
  metadata. Unrelated truncation exists elsewhere (projections, source
  coverage, delete `referrers_incomplete`).

## Proposed coverage schema

Coverage must stay separate from findings:

- A **completed** suite can still emit Error findings.
- A **skipped** suite must never be presented as a pass.
- A missing or default field must not imply `complete`.

Proposed fields (names are design targets until the proto lands):

| Field | Meaning |
| --- | --- |
| `check_id` | Stable suite or pass identifier (`store_hygiene`, `model`, …) |
| `state` | `COMPLETED` / `SKIPPED` / `PARTIAL` / `ERROR` |
| `reason` | Why skipped/partial/error (unsupported on this route, truncated snapshot, execution failure, narrower requested set) |
| `subjects_examined` / `subjects_skipped` | Counts or refs when meaningful; zero-subject completed checks are allowed and must say so |
| `requested_scope` / `effective_scope` | Branch, lens/visibility, project/domain/namespace/model ids |
| `snapshot_truncated` / related bounds | Shared with [snapshot identity](snapshot-identity.md) and the truncation package |

Write and dry-run envelopes (`PutEntity`, `BatchMutate`, `MergeBranch`)
should declare the suites they actually ran. Validate* responses should
list suite results alongside issues. Clients (Rust, CLI, MCP) must
propagate server fields and must not invent coverage client-side.

### Incomplete validation and CLI exits

Today `validate` / `validate-project` exit `1` when any Error-severity
finding exists ([exit codes](../reference/trogon-atlas-exit-codes.md));
warnings alone exit `0`. There is no exit for "validation ran incompletely."
Once coverage lands, incomplete runs need an explicit exit distinct from
"completed with findings," and precedence when both incomplete and Error
findings are present must be documented before CI depends on it.

### Enforcement is a separate decision

Reporting that Merge only ran hygiene does not by itself expand Merge to
`validate_model`. Expanding enforcement needs its own product answer
(whole post-merge model vs newly introduced errors vs current hygiene
subset). Coverage work makes that answer observable either way.

## Relation to truncation and consequence deltas

Truncation, depth, and visibility flags must use the same scope and
snapshot identity vocabulary as coverage, so a truncated empty issue list
cannot look complete. Consequence deltas then compare findings only within
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
