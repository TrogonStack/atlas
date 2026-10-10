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
    manifest::{entity_json_value, LoadedManifest},
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
        "code": issue.code,
        "message": issue.message,
        "subject": subject,
    })
}

pub struct ApplyOutcome {
    /// One human-readable line per manifest, in manifest order.
    pub lines: Vec<String>,
    /// Validation issues the server reported for applied/validated ops.
    pub issues: Vec<pb::ValidationIssue>,
    /// The batch's idempotency receipt, present only when the caller passed
    /// an `operation_id` to `apply` and at least one op ran.
    pub operation_receipt: Option<pb::OperationReceipt>,
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
    let mut operation_receipt = None;
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
        for result in &resp.results {
            if let Some(pb::batch_mutate_op_result::Result::Put(put)) = &result.result {
                issues.extend(put.validation.iter().cloned());
            }
        }
    }

    let lines = plan
        .iter()
        .map(|c| format!("{} {}", id_label(&c.manifest), c.action.verb(dry_run)))
        .collect();
    Ok(ApplyOutcome {
        lines,
        issues,
        operation_receipt,
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

pub fn format_issue(issue: &pb::ValidationIssue) -> String {
    let severity = match pb::validation_issue::Severity::try_from(issue.severity) {
        Ok(pb::validation_issue::Severity::Error) => "error",
        Ok(pb::validation_issue::Severity::Warning) => "warning",
        Ok(pb::validation_issue::Severity::Info) => "info",
        _ => "issue",
    };
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
                ..Default::default()
            }],
        };
        let json = rejected.to_json();
        assert_eq!(json["failed"], "event:shop/order.placed@1");
        assert_eq!(json["issues"][0]["code"], "ENTITY_REFERENCED");
    }
}
