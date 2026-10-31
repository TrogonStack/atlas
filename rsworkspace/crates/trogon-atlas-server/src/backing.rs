//! Backing report: which visible subjects carry evidence, and which cannot be
//! established either way. Presence only: a Decision's soundness, a Document's
//! accuracy or a code reference's freshness are outside this metric.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use pb::BackingIncompleteReason as Reason;
use trogon_atlas_core::refs::{entity_id, entity_kind};
use trogon_atlas_proto as pb;
use trogon_atlas_store::StoredEntity;

use crate::decision_document_resources as resources;

pub(crate) const ENTITY_UNBACKED: &str = "ENTITY_UNBACKED";
const CODE_REF_TYPE: &str = "trogonatlas.annotation.v1alpha1.CodeRefAnnotation";

/// Namespaces whose entities count as subjects. Empty admits every namespace.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubjectScope(BTreeSet<String>);

impl SubjectScope {
    pub(crate) fn new(namespaces: impl IntoIterator<Item = String>) -> Self {
        Self(namespaces.into_iter().collect())
    }

    fn admits(&self, namespace: &str) -> bool {
        self.0.is_empty() || self.0.contains(namespace)
    }

    pub(crate) fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// Namespaces the caller may read.
#[derive(Debug, Clone)]
pub(crate) enum VisibleNamespaces {
    All,
    Only(BTreeSet<String>),
}

impl VisibleNamespaces {
    fn is_restricted(&self) -> bool {
        matches!(self, Self::Only(_))
    }

    fn admits(&self, namespace: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(namespaces) => namespaces.contains(namespace),
        }
    }
}

/// What the caller's view of the snapshot cannot rule out.
#[derive(Debug, Clone)]
pub(crate) struct ViewLimits {
    pub(crate) snapshot_truncated: bool,
    pub(crate) visibility: VisibleNamespaces,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    Present,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backing {
    Backed,
    Unbacked,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
struct SubjectEvidence {
    decision: Evidence,
    document: Evidence,
    code_ref: Evidence,
}

impl SubjectEvidence {
    fn backing(self) -> Backing {
        let categories = [self.decision, self.document, self.code_ref];
        if categories.contains(&Evidence::Present) {
            Backing::Backed
        } else if categories.contains(&Evidence::Unknown) {
            Backing::Unknown
        } else {
            Backing::Unbacked
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct CategoryTally {
    present: u32,
    absent: u32,
    unknown: u32,
}

impl CategoryTally {
    fn add(&mut self, evidence: Evidence) {
        match evidence {
            Evidence::Present => self.present += 1,
            Evidence::Absent => self.absent += 1,
            Evidence::Unknown => self.unknown += 1,
        }
    }

    fn to_pb(self) -> pb::BackingCategoryCounts {
        pb::BackingCategoryCounts {
            present: self.present,
            absent: self.absent,
            unknown: self.unknown,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    total: u32,
    backed: u32,
    unbacked: u32,
    unknown: u32,
    decision: CategoryTally,
    document: CategoryTally,
    code_ref: CategoryTally,
}

impl Tally {
    fn add(&mut self, evidence: SubjectEvidence) {
        self.total += 1;
        match evidence.backing() {
            Backing::Backed => self.backed += 1,
            Backing::Unbacked => self.unbacked += 1,
            Backing::Unknown => self.unknown += 1,
        }
        self.decision.add(evidence.decision);
        self.document.add(evidence.document);
        self.code_ref.add(evidence.code_ref);
    }

    fn to_pb(self, snapshot_truncated: bool) -> pb::BackingTotals {
        let established = self.total > 0 && self.unknown == 0 && !snapshot_truncated;
        pb::BackingTotals {
            total: self.total,
            backed: self.backed,
            unbacked: self.unbacked,
            unknown: self.unknown,
            decision: Some(self.decision.to_pb()),
            document: Some(self.document.to_pb()),
            code_ref: Some(self.code_ref.to_pb()),
            unbacked_rate: established.then(|| f64::from(self.unbacked) / f64::from(self.total)),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct BackingReport {
    pub(crate) totals: pb::BackingTotals,
    pub(crate) kinds: Vec<pb::KindBackingTotals>,
    pub(crate) unresolved_declarations: u32,
    pub(crate) incomplete_reasons: Vec<pb::BackingIncompleteReason>,
    pub(crate) unbacked: Vec<pb::EntityRef>,
}

#[derive(Default)]
struct Declared {
    accepted_decisions: HashSet<pb::EntityKey>,
    unrecognized_decisions: HashSet<pb::EntityKey>,
    documents: HashSet<pb::EntityKey>,
    unresolved: u32,
}

fn is_subject_kind(kind: pb::EntityKind) -> bool {
    !matches!(
        kind,
        pb::EntityKind::Unspecified | pb::EntityKind::Decision | pb::EntityKind::Document
    )
}

fn selected_entity(association: &pb::SubjectAssociation) -> Option<pb::EntityKey> {
    let Some(pb::catalog_subject::Selection::Entity(reference)) = association
        .subject
        .as_ref()
        .and_then(|subject| subject.selection.as_ref())
    else {
        return None;
    };
    let kind = pb::EntityKind::try_from(reference.kind).ok()?;
    Some(pb::EntityKey::new(kind, reference.id.as_ref()?))
}

fn declarations(
    rows: &[StoredEntity],
    visible: &HashSet<pb::EntityKey>,
    scope: &SubjectScope,
    visibility: &VisibleNamespaces,
) -> Declared {
    #[derive(Clone, Copy)]
    enum Role {
        Qualifying,
        Unrecognized,
        Excluded,
    }
    let mut declared = Declared::default();
    for row in rows {
        let role = match &row.entity.kind {
            Some(pb::entity::Kind::Decision(decision)) => {
                match pb::DecisionStatus::try_from(decision.status) {
                    Ok(pb::DecisionStatus::Accepted) => Role::Qualifying,
                    Ok(
                        pb::DecisionStatus::Proposed
                        | pb::DecisionStatus::Rejected
                        | pb::DecisionStatus::Withdrawn
                        | pb::DecisionStatus::Superseded,
                    ) => Role::Excluded,
                    Ok(pb::DecisionStatus::Unspecified) | Err(_) => Role::Unrecognized,
                }
            }
            Some(pb::entity::Kind::Document(_)) => Role::Qualifying,
            _ => continue,
        };
        let is_decision = matches!(row.entity.kind, Some(pb::entity::Kind::Decision(_)));
        for association in resources::subjects(&row.entity) {
            let Some(key) = selected_entity(association) else {
                continue;
            };
            match role {
                Role::Excluded => continue,
                Role::Unrecognized => {
                    declared.unrecognized_decisions.insert(key);
                    continue;
                }
                Role::Qualifying => {}
            }
            if !visible.contains(&key)
                && scope.admits(&key.namespace)
                && visibility.admits(&key.namespace)
            {
                declared.unresolved += 1;
            }
            if is_decision {
                declared.accepted_decisions.insert(key);
            } else {
                declared.documents.insert(key);
            }
        }
    }
    declared
}

fn code_ref_evidence(entity: &pb::Entity) -> Evidence {
    let mut evidence = Evidence::Absent;
    for any in crate::validation::entity_metadata(entity) {
        if !any.type_url.ends_with(CODE_REF_TYPE) {
            continue;
        }
        match <pb::CodeRefAnnotation as prost::Message>::decode(any.value.as_slice()) {
            Ok(code_ref) if !code_ref.repo.is_empty() && !code_ref.path.is_empty() => {
                return Evidence::Present;
            }
            _ => evidence = Evidence::Unknown,
        }
    }
    evidence
}

pub(crate) fn report(
    rows: &[StoredEntity],
    scope: &SubjectScope,
    limits: &ViewLimits,
) -> BackingReport {
    let visible: HashSet<pb::EntityKey> = rows
        .iter()
        .filter_map(|row| {
            Some(pb::EntityKey::new(
                entity_kind(&row.entity)?,
                entity_id(&row.entity)?,
            ))
        })
        .collect();
    let declared = declarations(rows, &visible, scope, &limits.visibility);
    let visibility_restricted = limits.visibility.is_restricted();
    let hidden_possible = limits.snapshot_truncated || visibility_restricted;
    let mut reasons = BTreeSet::new();
    if limits.snapshot_truncated {
        reasons.insert(Reason::SnapshotTruncated as i32);
    }
    let mut overall = Tally::default();
    let mut kinds: BTreeMap<i32, Tally> = BTreeMap::new();
    let mut unbacked = Vec::new();
    for row in rows {
        let (Some(kind), Some(id)) = (entity_kind(&row.entity), entity_id(&row.entity)) else {
            continue;
        };
        if !is_subject_kind(kind) || !scope.admits(&id.namespace) {
            continue;
        }
        let key = pb::EntityKey::new(kind, id);
        let unrecognized_decision = declared.unrecognized_decisions.contains(&key);
        let decision = if declared.accepted_decisions.contains(&key) {
            Evidence::Present
        } else if unrecognized_decision || hidden_possible {
            Evidence::Unknown
        } else {
            Evidence::Absent
        };
        let document = if declared.documents.contains(&key) {
            Evidence::Present
        } else if hidden_possible {
            Evidence::Unknown
        } else {
            Evidence::Absent
        };
        let code_ref = code_ref_evidence(&row.entity);
        let evidence = SubjectEvidence {
            decision,
            document,
            code_ref,
        };
        if evidence.backing() == Backing::Unknown {
            if unrecognized_decision {
                reasons.insert(Reason::UnrecognizedDecisionStatus as i32);
            }
            if code_ref == Evidence::Unknown {
                reasons.insert(Reason::UnresolvedCodeRef as i32);
            }
            if visibility_restricted {
                reasons.insert(Reason::VisibilityRestricted as i32);
            }
        }
        overall.add(evidence);
        kinds.entry(kind as i32).or_default().add(evidence);
        if evidence.backing() == Backing::Unbacked {
            unbacked.push(pb::EntityRef {
                kind: kind as i32,
                id: Some(id.clone()),
            });
        }
    }
    unbacked.sort_by(|a, b| {
        let key = |r: &pb::EntityRef| {
            let id = r.id.clone().unwrap_or_default();
            (r.kind, id.namespace, id.slug, id.version)
        };
        key(a).cmp(&key(b))
    });
    BackingReport {
        totals: overall.to_pb(limits.snapshot_truncated),
        kinds: kinds
            .into_iter()
            .map(|(kind, tally)| pb::KindBackingTotals {
                kind,
                totals: Some(tally.to_pb(limits.snapshot_truncated)),
            })
            .collect(),
        unresolved_declarations: declared.unresolved,
        incomplete_reasons: reasons
            .into_iter()
            .filter_map(|reason| Reason::try_from(reason).ok())
            .collect(),
        unbacked,
    }
}

pub(crate) fn unbacked_finding(subject: &pb::EntityRef) -> pb::ValidationIssue {
    let id = subject.id.clone().unwrap_or_default();
    let kind = pb::EntityKind::try_from(subject.kind)
        .map_or("unknown", |kind| kind.as_str_name())
        .trim_start_matches("ENTITY_KIND_")
        .to_lowercase();
    pb::ValidationIssue {
        severity: pb::validation_issue::Severity::Info as i32,
        code: ENTITY_UNBACKED.into(),
        message: format!(
            "{kind}:{}/{}@{} has no accepted Decision, Document or code reference backing it",
            id.namespace, id.slug, id.version
        ),
        subject: Some(subject.clone()),
        ..Default::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn id(namespace: &str, slug: &str) -> pb::Id {
        pb::Id {
            namespace: namespace.into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn row(kind: pb::entity::Kind) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                kind: Some(kind),
                ..Default::default()
            },
            etag: String::new(),
        }
    }

    fn event(slug: &str, metadata: Vec<prost_types::Any>) -> StoredEntity {
        row(pb::entity::Kind::Event(pb::Event {
            id: Some(id("ns", slug)),
            metadata,
            ..Default::default()
        }))
    }

    fn event_ref(namespace: &str, slug: &str) -> pb::SubjectAssociation {
        pb::SubjectAssociation {
            subject: Some(pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id(namespace, slug)),
                })),
            }),
            ..Default::default()
        }
    }

    fn decision(slug: &str, status: i32, subjects: Vec<pb::SubjectAssociation>) -> StoredEntity {
        row(pb::entity::Kind::Decision(pb::Decision {
            id: Some(id("ns", slug)),
            status,
            subjects,
            ..Default::default()
        }))
    }

    fn document(slug: &str, subjects: Vec<pb::SubjectAssociation>) -> StoredEntity {
        row(pb::entity::Kind::Document(pb::Document {
            id: Some(id("ns", slug)),
            subjects,
            ..Default::default()
        }))
    }

    fn code_ref(repo: &str, path: &str) -> prost_types::Any {
        prost_types::Any {
            type_url: format!("type.googleapis.com/{CODE_REF_TYPE}"),
            value: prost::Message::encode_to_vec(&pb::CodeRefAnnotation {
                repo: repo.into(),
                path: path.into(),
                ..Default::default()
            }),
        }
    }

    const COMPLETE: ViewLimits = ViewLimits {
        snapshot_truncated: false,
        visibility: VisibleNamespaces::All,
    };

    fn restricted_to(namespaces: &[&str]) -> ViewLimits {
        ViewLimits {
            snapshot_truncated: false,
            visibility: VisibleNamespaces::Only(
                namespaces
                    .iter()
                    .map(|&namespace| namespace.to_owned())
                    .collect(),
            ),
        }
    }

    #[test]
    fn overlapping_categories_count_each_subject_once() {
        let rows = vec![
            event("a", vec![code_ref("repo", "a.rs")]),
            event("b", Vec::new()),
            decision(
                "d1",
                pb::DecisionStatus::Accepted as i32,
                vec![event_ref("ns", "a")],
            ),
            decision(
                "d2",
                pb::DecisionStatus::Accepted as i32,
                vec![event_ref("ns", "a")],
            ),
            document("doc", vec![event_ref("ns", "a")]),
        ];
        let report = report(&rows, &SubjectScope::default(), &COMPLETE);
        let totals = report.totals;
        assert_eq!((totals.total, totals.backed, totals.unbacked), (2, 1, 1));
        assert_eq!(totals.decision.unwrap().present, 1);
        assert_eq!(totals.document.unwrap().present, 1);
        assert_eq!(totals.code_ref.unwrap().present, 1);
        assert_eq!(totals.unbacked_rate, Some(0.5));
        assert_eq!(report.incomplete_reasons, Vec::new());
    }

    #[test]
    fn only_accepted_decisions_back_a_subject() {
        let rows = vec![
            event("a", Vec::new()),
            decision(
                "rejected",
                pb::DecisionStatus::Rejected as i32,
                vec![event_ref("ns", "a")],
            ),
            decision(
                "superseded",
                pb::DecisionStatus::Superseded as i32,
                vec![event_ref("ns", "a")],
            ),
            decision(
                "proposed",
                pb::DecisionStatus::Proposed as i32,
                vec![event_ref("ns", "a")],
            ),
        ];
        let report = report(&rows, &SubjectScope::default(), &COMPLETE);
        assert_eq!(report.totals.unbacked, 1);
        assert_eq!(report.unbacked.len(), 1);
    }

    #[test]
    fn unrecognized_status_and_malformed_code_ref_are_unknown() {
        let rows = vec![
            event("a", Vec::new()),
            event("b", vec![code_ref("", "b.rs")]),
            decision("future", 99, vec![event_ref("ns", "a")]),
        ];
        let report = report(&rows, &SubjectScope::default(), &COMPLETE);
        assert_eq!((report.totals.unknown, report.totals.unbacked), (2, 0));
        assert_eq!(report.totals.unbacked_rate, None);
        assert_eq!(
            report.incomplete_reasons,
            vec![
                Reason::UnresolvedCodeRef,
                Reason::UnrecognizedDecisionStatus
            ]
        );
    }

    #[test]
    fn hidden_or_truncated_views_never_prove_absence() {
        let rows = vec![event("a", Vec::new()), event("b", vec![code_ref("r", "p")])];
        for (limits, reason) in [
            (
                ViewLimits {
                    snapshot_truncated: true,
                    visibility: VisibleNamespaces::All,
                },
                Reason::SnapshotTruncated,
            ),
            (restricted_to(&["ns"]), Reason::VisibilityRestricted),
        ] {
            let report = report(&rows, &SubjectScope::default(), &limits);
            let totals = report.totals;
            assert_eq!((totals.backed, totals.unbacked, totals.unknown), (1, 0, 1));
            assert_eq!(totals.unbacked_rate, None);
            assert_eq!(report.unbacked, Vec::new());
            assert_eq!(report.incomplete_reasons, vec![reason]);
        }
    }

    #[test]
    fn empty_scope_has_no_rate_and_unresolved_targets_are_counted() {
        let rows = vec![document(
            "doc",
            vec![event_ref("ns", "gone"), event_ref("elsewhere", "gone")],
        )];
        let report = report(&rows, &SubjectScope::new(["ns".to_owned()]), &COMPLETE);
        assert_eq!(report.totals.total, 0);
        assert_eq!(report.totals.unbacked_rate, None);
        assert_eq!(report.kinds, Vec::new());
        assert_eq!(report.unresolved_declarations, 1);
    }

    #[test]
    fn settled_subjects_leave_no_incomplete_reason() {
        let rows = vec![
            event("a", vec![code_ref("r", "p")]),
            event("b", vec![code_ref("r", "p"), code_ref("", "broken")]),
            decision("future", 99, vec![event_ref("ns", "a")]),
        ];
        for limits in [COMPLETE, restricted_to(&["ns"])] {
            let report = report(&rows, &SubjectScope::default(), &limits);
            assert_eq!((report.totals.backed, report.totals.unknown), (2, 0));
            assert_eq!(report.totals.unbacked_rate, Some(0.0));
            assert_eq!(report.incomplete_reasons, Vec::new());
        }
    }

    #[test]
    fn empty_scope_ignores_targets_in_hidden_namespaces() {
        let rows = vec![document(
            "doc",
            vec![event_ref("ns", "gone"), event_ref("hidden", "elsewhere")],
        )];
        let report = report(&rows, &SubjectScope::default(), &restricted_to(&["ns"]));
        assert_eq!(report.unresolved_declarations, 1);
    }
}
