# Consequence delta

Wire contract for
[`ConsequenceDelta`](../../proto/trogonatlas/api/eventmodel/v1alpha1/validation.proto).
For the reasoning behind it, see the
[consequence delta explanation](../explanation/consequence-delta.md).

## Where it appears

| Response | Field | Set when |
| --- | --- | --- |
| `PutEntityResponse` | `consequence_delta` | `validate_only` was set and the write would be admitted |
| `BatchMutateResponse` | `consequence_delta` | `validate_only` was set and the status is `STATUS_VALIDATED` |
| `MergeBranchResponse` | `consequence_delta` | `dry_run` was set and validation was reached (`STATUS_VALIDATED` or `STATUS_INVALID`) |

A real write never carries a delta. An absent delta means none was
computed, never that nothing changed. Failing to compute a delta never
fails the dry run it rides on.

The MCP tools `put_entity`, `put_entity_json`, `batch_mutate`,
`batch_mutate_json`, `apply_manifests` and `merge_branch`, and the
`trogon-atlas apply --dry-run` and `trogon-atlas branch merge --dry-run`
JSON output, add a `consequence_delta` object with the same fields.

`apply_manifests` and `trogon-atlas apply --dry-run` send no batch when
every manifest is unchanged, so they carry no delta in that case.

## `ConsequenceDelta`

| Field | Meaning |
| --- | --- |
| `changes` | Findings whose standing moved, in a stable order |
| `unchanged_count` | Occurrences present on both sides with the same severity |
| `before_coverage` | Coverage of the scan of the world before the change |
| `after_coverage` | Coverage of the scan of the world after the change |
| `complete` | True only when every compared suite completed on both sides, no snapshot was truncated, no applicable suite was left out, and both snapshot ids are set |
| `before_snapshot_id` | Snapshot id of the world the preview started from; empty when that world holds an entity without content identity |
| `after_snapshot_id` | Snapshot id of the world the change would produce; empty under the same condition |

## `FindingChange`

| Field | Meaning |
| --- | --- |
| `kind` | See the kinds table |
| `check_id` | Suite that produced the finding |
| `origin` | For the `model` and `scenarios` suites, which run once per entity, the entity whose pass produced the finding; absent for store-wide suites |
| `before` | A representative occurrence before the change, when one existed |
| `after` | A representative occurrence after the change, when one exists |
| `occurrences` | How many occurrences this change accounts for |
| `occurrences_before` | Occurrences of this identity before the change (for a successor, of the predecessor) |
| `occurrences_after` | Occurrences of this identity after the change |
| `reason` | For `KIND_UNKNOWN`, why the relevant scan cannot vouch for the finding |

## Kinds

| Kind | `before` | `after` | Meaning |
| --- | --- | --- | --- |
| `KIND_INTRODUCED` | absent | set | The change creates the finding; the before-scan's suite completed |
| `KIND_RESOLVED` | set | absent | The change clears the finding; the after-scan's suite completed |
| `KIND_SEVERITY_CHANGED` | set | set | Same finding, different severity |
| `KIND_SUBJECT_REMOVED` | set | absent | The finding's subject or origin entity no longer exists after the change; reported even when the after-scan's suite did not complete |
| `KIND_CARRIED_TO_SUCCESSOR` | set | set | The finding moved to a newer version of its subject or origin, with no version going down |
| `KIND_UNKNOWN` | either | either | A finding appeared or disappeared, but the scan on the other side did not complete its suite |

For `KIND_UNKNOWN`, `before` alone means a resolution is unproven, and
`after` alone means novelty is unproven.

## Finding identity

Two occurrences are the same finding when they agree on suite, `origin`
(kind, namespace, slug and version), rule `code`, subject kind, namespace,
slug and version, `subject_field`, and
every other structured field of `ValidationIssue`. Message prose, severity,
`rule_title`, `rule_category` and `rule_default_severity` are excluded.

## Coverage check ids

The coverage of each scan lists these checks:

| Check | Compared | Notes |
| --- | --- | --- |
| `store_hygiene` | yes | `PARTIAL` with `snapshot_truncated` when the snapshot hit the cap |
| `architecture` | yes | As above |
| `model` | yes | As above |
| `strategic` | yes | As above |
| `scenarios` | yes | As above |
| `specification_types` | no | `SKIPPED` with `delta_not_supported` when the world holds specifications, otherwise `kind_not_applicable` |
| `data_product_events` | no | `SKIPPED` with `delta_not_supported` when the world holds data products, otherwise `kind_not_applicable` |
| `data_flow` | no | Always `SKIPPED` with `analysis_not_compared` |
| `information_completeness` | no | Always `SKIPPED` with `analysis_not_compared` |

A compared suite missing from a scan's coverage is treated as not
completed, with reason `suite_not_reported`.

## Metrics

| Metric | Instrument | Unit |
| --- | --- | --- |
| `trogon_atlas.consequence_delta.duration` | histogram | s |
