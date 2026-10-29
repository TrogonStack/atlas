// Apply engine: classify each manifest against live server state, then send
// only real changes through one atomic BatchMutate.
//
// Unchanged entities are skipped on purpose: a put commits to the git mirror,
// so re-applying an identical manifest set must be a no-op end to end
// (kubectl semantics), not a wall of empty mirror commits.
//
// Every write carries the revision it was planned against (`create_only` for
// an absent entity, `if_match` for a present one), so a plan that another
// writer overtook is refused as a `StalePlan` instead of overwriting them.
// A server too old to report revisions can still be diffed, but a plan made
// against it is refused at apply time rather than written unguarded.

use std::fmt;

use anyhow::{bail, Context as _, Result};
use prost::Message as _;
use serde_json::{json, Value};
use trogon_atlas_core::{semantic_eq::semantically_equal, transcode::TranscodePool};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    manifest::{entity_json_value, LoadedManifest, ManifestDocument},
    precondition::{Revision, StalePlan},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Configure,
    Unchanged,
}

impl Action {
    fn verb(self, dry_run: bool) -> &'static str {
        match (self, dry_run) {
            (Action::Create, false) => "created",
            (Action::Create, true) => "created (server dry run)",
            (Action::Configure, false) => "configured",
            (Action::Configure, true) => "configured (server dry run)",
            (Action::Unchanged, _) => "unchanged",
        }
    }
}

/// Server-owned metadata is stamped on write; strip it so a round-tripped
/// entity compares equal to its manifest.
fn strip_system(entity: &pb::Entity) -> pb::Entity {
    let mut e = entity.clone();
    e.system = None;
    e
}

/// Decide what applying `desired` over `current` means.
///
/// Fast path is a byte compare, but bytes alone over-report drift: proto
/// map encoding order (e.g. `google.protobuf.Struct` scenario payloads) is
/// nondeterministic, so a re-encoded entity can differ byte-wise while
/// being the same model. Fall back to canonical-JSON equality before
/// declaring Configure; never rewrite an entity that is semantically
/// identical (rewrites churn the git mirror and shuffle stored bytes).
pub fn classify(desired: &pb::Entity, current: Option<&pb::Entity>) -> Action {
    match current {
        None => Action::Create,
        Some(current) => {
            let (cur, want) = (strip_system(current), strip_system(desired));
            if cur.encode_to_vec() == want.encode_to_vec() {
                return Action::Unchanged;
            }
            if trogon_atlas_core::decision_document::equivalent_manifest(&cur, &want)
                .unwrap_or(false)
                || trogon_atlas_core::connections::equivalent_with_unassigned_ids(&cur, &want)
                    .unwrap_or(false)
            {
                return Action::Unchanged;
            }
            match (entity_json_value(&cur), entity_json_value(&want)) {
                (Ok(a), Ok(b)) if a == b => Action::Unchanged,
                _ => Action::Configure,
            }
        }
    }
}

#[derive(Debug)]
pub struct PlannedChange {
    pub manifest: LoadedManifest,
    pub action: Action,
    pub current: Option<pb::Entity>,
    /// `None` when the server holds the entity but did not report its
    /// revision, so the change cannot be guarded.
    pub observed: Option<Revision>,
}

fn dedupe_manifests(manifests: Vec<LoadedManifest>) -> Result<Vec<LoadedManifest>> {
    let mut order: Vec<pb::EntityKey> = Vec::new();
    let mut by_key: std::collections::HashMap<pb::EntityKey, LoadedManifest> =
        std::collections::HashMap::new();
    for m in manifests {
        let key = pb::EntityKey::new(m.kind, &m.id);
        match by_key.get(&key) {
            None => {
                order.push(key.clone());
                by_key.insert(key, m);
            }
            Some(first) if semantically_equal(&first.entity, &m.entity) => {}
            Some(first) => {
                return Err(anyhow::Error::new(tonic::Status::invalid_argument(
                    format!(
                        "duplicate manifest for {key}: defined in both {} and {}",
                        first.source, m.source
                    ),
                )));
            }
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|key| by_key.remove(&key))
        .collect())
}

/// Fetch live state for every manifest and classify it. Identical duplicates
/// (a slices-layout export bundles shared entities into every slice file)
/// collapse into one change; differing duplicates are an authoring error.
pub async fn plan(
    client: &mut Client,
    manifests: Vec<LoadedManifest>,
) -> Result<Vec<PlannedChange>> {
    let manifests = dedupe_manifests(manifests)?;

    let keys = manifests
        .iter()
        .map(|m| pb::EntityRef {
            kind: m.kind as i32,
            id: Some(m.id.clone()),
        })
        .collect();
    let resp = client
        .batch_get_entities(pb::BatchGetEntitiesRequest { keys, dense: true })
        .await
        .context("BatchGetEntities failed")?
        .into_inner();
    if resp.found.len() != manifests.len() || resp.entities.len() != manifests.len() {
        bail!(
            "server returned {} entities / {} found flags for {} keys",
            resp.entities.len(),
            resp.found.len(),
            manifests.len()
        );
    }
    let etags = if resp.etags.is_empty() {
        vec![String::new(); manifests.len()]
    } else {
        resp.etags
    };
    if etags.len() != manifests.len() {
        bail!(
            "server returned {} etags for {} keys",
            etags.len(),
            manifests.len()
        );
    }

    manifests
        .into_iter()
        .zip(resp.entities.into_iter().zip(resp.found).zip(etags))
        .map(|(manifest, ((entity, found), etag))| {
            let observed = match Revision::from_wire(&etag)? {
                Revision::Absent if found => None,
                revision => Some(revision),
            };
            let current = found.then_some(entity);
            let action = classify(&manifest.entity, current.as_ref());
            Ok(PlannedChange {
                manifest,
                action,
                current,
                observed,
            })
        })
        .collect()
}

/// Refuse a board import whose target lacks a foreign entity the board
/// references. Another namespace owns those entities, so the import never
/// writes them, and only their identity is checked: a bundled spec that no
/// longer transcodes must not hide a missing entity or block a present one.
pub async fn require_external(client: &mut Client, external: Vec<ManifestDocument>) -> Result<()> {
    if external.is_empty() {
        return Ok(());
    }
    let keys = external
        .iter()
        .map(|d| pb::EntityRef {
            kind: d.kind as i32,
            id: Some(d.id.clone()),
        })
        .collect();
    let found = client
        .batch_get_entities(pb::BatchGetEntitiesRequest { keys, dense: true })
        .await
        .context("BatchGetEntities failed")?
        .into_inner()
        .found;
    if found.len() != external.len() {
        bail!(
            "server returned {} found flags for {} keys",
            found.len(),
            external.len()
        );
    }
    let missing: Vec<String> = external
        .iter()
        .zip(found)
        .filter(|(_, found)| !found)
        .map(|(d, _)| pb::canonical::id_string(d.kind, &d.id))
        .collect();
    if !missing.is_empty() {
        crate::invalid_input!(
            "the target is missing external entities this board references; apply them from their own namespace first: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

/// A batch was refused (`BatchMutateResponse.status == FAILED`) for a reason
/// other than a stale precondition: a validation rule, a referrer conflict,
/// or any other per-op diagnostic in `issues`. The precondition case is
/// `StalePlan` instead, since it carries different (and more actionable)
/// detail: the two revisions in play.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyRejected {
    /// The manifest being applied when the batch failed, or `<unknown op>`
    /// when the server's index does not map back to one (which should not
    /// happen for a well-formed response).
    pub failed: String,
    pub issues: Vec<pb::ValidationIssue>,
}

impl ApplyRejected {
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "failed": self.failed,
            "issues": self.issues.iter().map(issue_to_json).collect::<Vec<_>>(),
        })
    }
}

impl fmt::Display for ApplyRejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "apply failed at {}:\n{}",
            self.failed,
            self.issues
                .iter()
                .map(format_issue)
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

impl std::error::Error for ApplyRejected {}

/// JSON shape for one `ValidationIssue`, shared by `ApplyRejected::to_json`
/// and by any structured-output surface (trogon-atlas's `--format json`) that needs
/// to render issues without string-matching their text.
#[must_use]
pub fn issue_to_json(issue: &pb::ValidationIssue) -> Value {
    let subject = issue.subject.as_ref().and_then(|s| {
        let kind = pb::EntityKind::try_from(s.kind).ok()?;
        let id = s.id.as_ref()?;
        Some(pb::canonical::id_string(kind, id))
    });
    json!({
        "severity": severity_name(issue),
        "code": issue.code,
        "message": issue.message,
        "subject": subject,
    })
}

/// The validation findings one batch op produced, with the op's position in
/// the batch and the canonical id of the entity it wrote.
pub struct OpIssues {
    /// Zero-based position among the batch's ops (unchanged manifests are
    /// not sent, so this is not the manifest index).
    pub op_index: usize,
    pub op_entity: String,
    pub issues: Vec<pb::ValidationIssue>,
}

pub struct ApplyOutcome {
    /// One human-readable line per manifest, in manifest order.
    pub lines: Vec<String>,
    /// Validation issues the server reported for applied/validated ops.
    pub issues: Vec<pb::ValidationIssue>,
    /// The same findings grouped by the op that produced them; ops with no
    /// findings are omitted.
    pub op_issues: Vec<OpIssues>,
    /// The batch's idempotency receipt, present only when the caller passed
    /// an `operation_id` to `apply` and at least one op ran.
    pub operation_receipt: Option<pb::OperationReceipt>,
    /// Server-reported suite coverage for the batch. Absent when no ops ran
    /// or the server omitted it (for example operation_id replay). Never
    /// invent coverage client-side.
    pub coverage: Option<pb::ValidationCoverage>,
}

/// Adds server-reported coverage to a JSON object. Absent coverage stays
/// absent so a reader never mistakes an omitted report for a full-model pass.
pub fn insert_coverage(body: &mut Value, coverage: Option<&pb::ValidationCoverage>) {
    if let (Some(fields), Some(coverage)) = (body.as_object_mut(), coverage) {
        fields.insert("coverage".into(), coverage_to_json(coverage));
    }
}

/// JSON shape for [`pb::ValidationCoverage`], for CLI/MCP structured output.
/// Callers must pass through the server value; do not invent fields.
#[must_use]
pub fn coverage_to_json(coverage: &pb::ValidationCoverage) -> Value {
    json!({
        "snapshot_truncated": coverage.snapshot_truncated,
        "effective_scope": coverage.effective_scope.as_ref().map(|s| json!({
            "branch": s.branch,
            "namespace": s.namespace,
        })),
        "requested_scope": coverage.requested_scope.as_ref().map(|s| json!({
            "branch": s.branch,
            "namespace": s.namespace,
        })),
        "checks": coverage.checks.iter().map(|c| json!({
            "check_id": c.check_id,
            "state": check_state_name(c.state),
            "reason": c.reason,
            "subjects_examined": c.subjects_examined,
        })).collect::<Vec<_>>(),
    })
}

fn check_state_name(state: i32) -> &'static str {
    match pb::validation_check_result::State::try_from(state) {
        Ok(pb::validation_check_result::State::Completed) => "COMPLETED",
        Ok(pb::validation_check_result::State::Skipped) => "SKIPPED",
        Ok(pb::validation_check_result::State::Partial) => "PARTIAL",
        Ok(pb::validation_check_result::State::Error) => "ERROR",
        _ => "UNSPECIFIED",
    }
}

/// Groups the findings of put and delete results by op, tagging each group
/// with the op's position and the entity label at that position.
fn issues_by_op(results: &[pb::BatchMutateOpResult], labels: &[String]) -> Vec<OpIssues> {
    results
        .iter()
        .enumerate()
        .filter_map(|(op_index, result)| {
            let findings = match &result.result {
                Some(pb::batch_mutate_op_result::Result::Put(put)) => &put.validation,
                Some(pb::batch_mutate_op_result::Result::Delete(delete)) => &delete.validation,
                None => return None,
            };
            (!findings.is_empty()).then(|| OpIssues {
                op_index,
                op_entity: labels.get(op_index).cloned().unwrap_or_default(),
                issues: findings.clone(),
            })
        })
        .collect()
}

fn id_label(m: &LoadedManifest) -> String {
    pb::canonical::id_string(m.kind, &m.id)
}

/// Apply the plan. With `dry_run` the batch runs server-side validation for
/// every op but persists nothing (`BatchMutateRequest.validate_only`).
///
/// `operation_id`, when given, is the batch's idempotency key: it is set
/// only on the outer `BatchMutateRequest`, never on a nested op, which the
/// server rejects (a batch is one unit of idempotency, not several). It is
/// dropped entirely when `dry_run` is set: the server rejects an
/// `operation_id` paired with `validate_only`, since a dry run never
/// produces a write for a later replay to recover.
pub async fn apply(
    client: &mut Client,
    plan: Vec<PlannedChange>,
    dry_run: bool,
    operation_id: Option<&str>,
) -> Result<ApplyOutcome> {
    let changed: Vec<&PlannedChange> = plan
        .iter()
        .filter(|c| c.action != Action::Unchanged)
        .collect();

    let mut issues = Vec::new();
    let mut op_issues = Vec::new();
    let mut operation_receipt = None;
    let mut coverage = None;
    if !changed.is_empty() {
        let ops = changed
            .iter()
            .map(|c| {
                let Some(observed) = &c.observed else {
                    bail!(
                        "server did not report the revision of {}; it predates stale-plan protection, so refusing a write that could overwrite a concurrent change",
                        id_label(&c.manifest)
                    );
                };
                Ok(pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(c.manifest.entity.clone()),
                        create_only: *observed == Revision::Absent,
                        if_match: observed.to_wire(),
                        validate_only: false,
                        force: false,
                        operation_id: String::new(),
                    })),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let operation_id = if dry_run {
            String::new()
        } else {
            operation_id.unwrap_or_default().to_string()
        };
        let resp = client
            .batch_mutate(pb::BatchMutateRequest {
                ops,
                validate_only: dry_run,
                operation_id,
            })
            .await
            .context("BatchMutate failed")?
            .into_inner();
        operation_receipt.clone_from(&resp.operation_receipt);

        let status = pb::batch_mutate_response::Status::try_from(resp.status)
            .unwrap_or(pb::batch_mutate_response::Status::Unspecified);
        if status == pb::batch_mutate_response::Status::Failed {
            if let Some(failure) = &resp.precondition_failure {
                return Err(anyhow::Error::new(StalePlan::from_wire(failure)?));
            }
            let failed = usize::try_from(resp.failed_op_index)
                .ok()
                .and_then(|i| changed.get(i))
                .map_or_else(|| "<unknown op>".to_string(), |c| id_label(&c.manifest));
            return Err(anyhow::Error::new(ApplyRejected {
                failed,
                issues: resp.failure.clone(),
            }));
        }
        let expected = if dry_run {
            pb::batch_mutate_response::Status::Validated
        } else {
            pb::batch_mutate_response::Status::Applied
        };
        if status != expected {
            bail!(
                "apply returned unexpected status {status:?}, expected {expected:?} (dry_run={dry_run})"
            );
        }
        let labels: Vec<String> = changed.iter().map(|c| id_label(&c.manifest)).collect();
        op_issues = issues_by_op(&resp.results, &labels);
        issues = op_issues
            .iter()
            .flat_map(|o| o.issues.iter().cloned())
            .collect();
        coverage = resp.coverage;
    }

    let lines = plan
        .iter()
        .map(|c| format!("{} {}", id_label(&c.manifest), c.action.verb(dry_run)))
        .collect();
    Ok(ApplyOutcome {
        lines,
        issues,
        op_issues,
        operation_receipt,
        coverage,
    })
}

/// Render the plan as a unified diff of canonical proto JSON, kubectl-diff
/// style. Returns true when at least one entity would change.
pub fn render_diff(plan: &[PlannedChange], types: &TranscodePool) -> Result<(String, bool)> {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mut changed = false;
    for c in plan {
        if c.action == Action::Unchanged {
            continue;
        }
        changed = true;
        let label = id_label(&c.manifest);
        let before = match &c.current {
            Some(current) => types.entity_to_json(&strip_system(current))? + "\n",
            None => String::new(),
        };
        let after = types.entity_to_json(&c.manifest.entity)? + "\n";
        let diff = similar::TextDiff::from_lines(&before, &after);
        let _ = write!(
            out,
            "diff {label}\n{}",
            diff.unified_diff()
                .context_radius(3)
                .header(&format!("live/{label}"), &format!("manifest/{label}"))
        );
    }
    Ok((out, changed))
}

fn severity_name(issue: &pb::ValidationIssue) -> &'static str {
    match pb::validation_issue::Severity::try_from(issue.severity) {
        Ok(pb::validation_issue::Severity::Error) => "error",
        Ok(pb::validation_issue::Severity::Warning) => "warning",
        Ok(pb::validation_issue::Severity::Info) => "info",
        _ => "issue",
    }
}

pub fn format_issue(issue: &pb::ValidationIssue) -> String {
    let severity = severity_name(issue);
    let subject = issue
        .subject
        .as_ref()
        .and_then(|s| {
            let kind = pb::EntityKind::try_from(s.kind).ok()?;
            let id = s.id.as_ref()?;
            Some(format!(" [{}]", pb::canonical::id_string(kind, id)))
        })
        .unwrap_or_default();
    format!("  {severity} {}{subject}: {}", issue.code, issue.message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn event(title: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order.placed".into(),
                    version: 1,
                }),
                title: title.into(),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn classify_missing_is_create() {
        assert_eq!(classify(&event("A"), None), Action::Create);
    }

    #[test]
    fn idless_connection_manifest_is_a_noop_only_when_everything_else_matches() {
        let desired = pb::Entity {
            kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "place-order".into(),
                    version: 1,
                }),
                command: Some(pb::CommandEdge {
                    command: Some(pb::CommandRef {
                        id: Some(pb::Id {
                            namespace: "shop".into(),
                            slug: "place".into(),
                            version: 1,
                        }),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
            system: None,
        };
        let mut current = desired.clone();
        trogon_atlas_core::connections::assign_connection_ids(&mut current).unwrap();
        assert_eq!(classify(&desired, Some(&current)), Action::Unchanged);

        let mut changed = desired;
        let Some(pb::entity::Kind::CommandSlice(slice)) = changed.kind.as_mut() else {
            panic!("expected slice")
        };
        slice.command.as_mut().unwrap().doc = "changed connection".into();
        assert_eq!(classify(&changed, Some(&current)), Action::Configure);

        let mut changed_id = current.clone();
        let Some(pb::entity::Kind::CommandSlice(slice)) = changed_id.kind.as_mut() else {
            panic!("expected slice")
        };
        slice.command.as_mut().unwrap().connection_id =
            Some(trogon_atlas_core::connections::ConnectionId::new().into());
        assert_eq!(classify(&changed_id, Some(&current)), Action::Configure);
    }

    #[test]
    fn classify_equal_is_unchanged_ignoring_system_meta() {
        let mut stored = event("A");
        stored.system = Some(pb::SystemMeta {
            uid: "0198-uid".into(),
            created_at: "2026-07-18T00:00:00Z".into(),
        });
        assert_eq!(classify(&event("A"), Some(&stored)), Action::Unchanged);
    }

    #[test]
    fn classify_different_is_configure() {
        assert_eq!(classify(&event("B"), Some(&event("A"))), Action::Configure);
    }

    fn manifest(entity: pb::Entity, source: &str) -> LoadedManifest {
        LoadedManifest {
            source: source.into(),
            kind: pb::EntityKind::Event,
            id: pb::Id {
                namespace: "shop".into(),
                slug: "order.placed".into(),
                version: 1,
            },
            entity,
        }
    }

    #[test]
    fn dedupe_collapses_identical_duplicates() {
        let manifests = vec![
            manifest(event("A"), "slice-one.yaml"),
            manifest(event("A"), "slice-two.yaml"),
        ];
        let deduped = dedupe_manifests(manifests).unwrap();
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].source, "slice-one.yaml");
    }

    #[test]
    fn dedupe_rejects_conflicting_duplicates() {
        let manifests = vec![
            manifest(event("A"), "slice-one.yaml"),
            manifest(event("B"), "slice-two.yaml"),
        ];
        let err = dedupe_manifests(manifests).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("slice-one.yaml"), "{msg}");
        assert!(msg.contains("slice-two.yaml"), "{msg}");
    }

    #[test]
    fn apply_rejected_displays_the_failed_manifest_and_every_issue() {
        let rejected = ApplyRejected {
            failed: "event:shop/order.placed@1".into(),
            issues: vec![pb::ValidationIssue {
                severity: pb::validation_issue::Severity::Error as i32,
                code: "SCENARIO_MISSING_REF".into(),
                message: "missing ref".into(),
                ..Default::default()
            }],
        };
        let rendered = rejected.to_string();
        assert!(rendered.contains("apply failed at event:shop/order.placed@1"));
        assert!(rendered.contains("SCENARIO_MISSING_REF"));
        assert!(rendered.contains("missing ref"));
    }

    #[test]
    fn apply_rejected_to_json_carries_every_issue_code() {
        let rejected = ApplyRejected {
            failed: "event:shop/order.placed@1".into(),
            issues: vec![pb::ValidationIssue {
                code: "ENTITY_REFERENCED".into(),
                message: "entity is referenced".into(),
                severity: pb::validation_issue::Severity::Error as i32,
                ..Default::default()
            }],
        };
        let json = rejected.to_json();
        assert_eq!(json["failed"], "event:shop/order.placed@1");
        assert_eq!(json["issues"][0]["code"], "ENTITY_REFERENCED");
        assert_eq!(json["issues"][0]["severity"], "error");
    }

    fn finding(code: &str) -> pb::ValidationIssue {
        pb::ValidationIssue {
            severity: pb::validation_issue::Severity::Warning as i32,
            code: code.into(),
            ..Default::default()
        }
    }

    #[test]
    fn issues_by_op_tags_put_and_delete_findings_and_skips_clean_ops() {
        use pb::batch_mutate_op_result::Result as OpResult;
        let results = vec![
            pb::BatchMutateOpResult {
                result: Some(OpResult::Put(pb::PutEntityResponse {
                    validation: vec![finding("REALIZES_MIXED_CLASSIFICATIONS")],
                    ..Default::default()
                })),
            },
            pb::BatchMutateOpResult {
                result: Some(OpResult::Put(pb::PutEntityResponse::default())),
            },
            pb::BatchMutateOpResult {
                result: Some(OpResult::Delete(pb::DeleteEntityResponse {
                    validation: vec![finding("REALIZES_DANGLING")],
                    ..Default::default()
                })),
            },
            pb::BatchMutateOpResult { result: None },
        ];
        let labels = vec![
            "bounded_context:shop/shop@1".to_string(),
            "event:shop/order.placed@1".to_string(),
            "subdomain:acme/ordering@1".to_string(),
        ];

        let grouped = issues_by_op(&results, &labels);

        let tagged: Vec<(usize, &str, &str)> = grouped
            .iter()
            .flat_map(|o| {
                o.issues
                    .iter()
                    .map(move |i| (o.op_index, o.op_entity.as_str(), i.code.as_str()))
            })
            .collect();
        assert_eq!(
            tagged,
            vec![
                (
                    0,
                    "bounded_context:shop/shop@1",
                    "REALIZES_MIXED_CLASSIFICATIONS"
                ),
                (2, "subdomain:acme/ordering@1", "REALIZES_DANGLING"),
            ]
        );
    }
}
