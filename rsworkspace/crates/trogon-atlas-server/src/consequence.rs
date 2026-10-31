//! Consequence deltas: how a candidate world moves the findings of the world
//! it would replace.
//!
//! Both worlds are scanned in full with the same suites, then compared by
//! `FindingIdentity`. A finding missing from one side is only called
//! introduced or resolved when that side's scan covered the finding's suite;
//! otherwise it is reported as unknown. See
//! `docs/explanation/consequence-delta.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use prost::Message as _;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id, entity_kind},
    StoredEntity,
};

use crate::validation;

/// Validation reached through stored type libraries and data-product event
/// lookups, which read the store rather than a candidate world.
pub const CHECK_SPECIFICATION_TYPES: &str = "specification_types";
pub const CHECK_DATA_PRODUCT_EVENTS: &str = "data_product_events";
/// Scoped analyses whose results may come from a language model, so two
/// runs over one world need not agree.
pub const CHECK_DATA_FLOW: &str = "data_flow";
pub const CHECK_INFORMATION_COMPLETENESS: &str = "information_completeness";

/// An applicable suite the delta cannot evaluate against a candidate world.
pub const REASON_DELTA_NOT_SUPPORTED: &str = "delta_not_supported";
/// A non-validation analysis the delta deliberately does not compare.
pub const REASON_ANALYSIS_NOT_COMPARED: &str = "analysis_not_compared";
/// A compared suite the scan's coverage does not mention.
const REASON_SUITE_NOT_REPORTED: &str = "suite_not_reported";

/// Suites whose findings the delta compares.
const COMPARED_SUITES: [&str; 5] = [
    validation::CHECK_STORE_HYGIENE,
    validation::CHECK_ARCHITECTURE,
    validation::CHECK_MODEL,
    validation::CHECK_STRATEGIC,
    validation::CHECK_SCENARIOS,
];

/// The exact entity a finding is attached to, split so a successor version
/// of the same entity can be recognized.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Subject {
    kind: i32,
    namespace: String,
    slug: String,
    version: u64,
}

impl Subject {
    fn of(subject: &pb::EntityRef) -> Self {
        let id = subject.id.clone().unwrap_or_default();
        Self {
            kind: subject.kind,
            namespace: id.namespace,
            slug: id.slug,
            version: id.version,
        }
    }

    fn of_stored(stored: &StoredEntity) -> Option<Self> {
        let kind = entity_kind(&stored.entity)?;
        Some(Self::of(&pb::EntityRef {
            kind: kind as i32,
            id: entity_id(&stored.entity).cloned(),
        }))
    }

    fn entity_ref(&self) -> pb::EntityRef {
        pb::EntityRef {
            kind: self.kind,
            id: Some(pb::Id {
                namespace: self.namespace.clone(),
                slug: self.slug.clone(),
                version: self.version,
            }),
        }
    }

    fn key(&self) -> Option<pb::EntityKey> {
        let kind = pb::EntityKind::try_from(self.kind).ok()?;
        Some(pb::EntityKey {
            kind,
            namespace: self.namespace.clone(),
            slug: self.slug.clone(),
            version: self.version,
        })
    }
}

/// What makes two findings the same finding: suite, the entity whose pass
/// produced it, rule code, exact subject and version, field, and every
/// structured field of the issue that is not prose, severity or denormalized
/// rule context. Structured fields added to `ValidationIssue` later join the
/// identity without changes here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct FindingIdentity {
    suite: &'static str,
    origin: Option<Subject>,
    code: String,
    subject: Option<Subject>,
    field: String,
    structure: Vec<u8>,
}

impl FindingIdentity {
    fn of(finding: &Finding) -> Self {
        let Finding {
            suite,
            origin,
            issue,
        } = finding;
        let mut structure = issue.clone();
        structure.severity = 0;
        structure.code.clear();
        structure.message.clear();
        structure.subject = None;
        structure.subject_field.clear();
        structure.rule_title.clear();
        structure.rule_category.clear();
        structure.rule_default_severity = 0;
        let structure = structure.encode_to_vec();
        Self {
            suite,
            origin: origin.clone(),
            code: issue.code.clone(),
            subject: issue.subject.as_ref().map(Subject::of),
            field: issue.subject_field.clone(),
            structure,
        }
    }

    /// The identity with every entity version erased, and those versions.
    fn lineage(&self) -> (Self, Versions) {
        let version = |s: &Option<Subject>| s.as_ref().map_or(0, |s| s.version);
        let versions = Versions {
            origin: version(&self.origin),
            subject: version(&self.subject),
        };
        let mut lineage = self.clone();
        for s in [&mut lineage.origin, &mut lineage.subject]
            .into_iter()
            .flatten()
        {
            s.version = 0;
        }
        (lineage, versions)
    }

    fn entities(&self) -> impl Iterator<Item = pb::EntityKey> + '_ {
        [&self.origin, &self.subject]
            .into_iter()
            .flatten()
            .filter_map(Subject::key)
    }
}

/// Entity versions a finding is pinned to. One identity succeeds another
/// when no version went down and at least one went up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Versions {
    origin: u64,
    subject: u64,
}

impl Versions {
    fn precedes(self, other: Self) -> bool {
        self != other && self.origin <= other.origin && self.subject <= other.subject
    }
}

/// One occurrence of an issue, with the suite that reported it and, for
/// suites that run once per entity, the entity the pass ran over.
#[derive(Debug, Clone)]
struct Finding {
    suite: &'static str,
    origin: Option<Subject>,
    issue: pb::ValidationIssue,
}

/// One world's findings, tagged by suite, with the coverage the scan earned
/// and the entity keys it held.
#[derive(Debug, Clone)]
pub struct Scan {
    findings: Vec<Finding>,
    coverage: pb::ValidationCoverage,
    keys: HashSet<pb::EntityKey>,
}

impl Scan {
    /// Every compared suite over the whole of `world`. Architecture resources
    /// resolve against `world` plus `architecture_context`.
    #[must_use]
    pub fn of(
        world: &[StoredEntity],
        architecture_context: &[StoredEntity],
        snapshot_truncated: bool,
        scope: &pb::ValidationScope,
    ) -> Self {
        let mut findings: Vec<Finding> = Vec::new();
        let mut tag = |suite: &'static str,
                       origin: Option<&StoredEntity>,
                       issues: Vec<pb::ValidationIssue>| {
            let origin = origin.and_then(Subject::of_stored);
            findings.extend(issues.into_iter().map(|issue| Finding {
                suite,
                origin: origin.clone(),
                issue,
            }));
        };
        tag(
            validation::CHECK_STORE_HYGIENE,
            None,
            validation::validate_store_hygiene(world),
        );
        tag(
            validation::CHECK_ARCHITECTURE,
            None,
            validation::validate_architecture_resources(world, architecture_context),
        );
        let mut models = 0usize;
        let mut slices = 0usize;
        let mut namespaces: BTreeSet<&str> = BTreeSet::new();
        let mut has_specifications = false;
        let mut has_data_products = false;
        for stored in world {
            if let Some(id) = entity_id(&stored.entity) {
                namespaces.insert(id.namespace.as_str());
            }
            match stored.entity.kind.as_ref() {
                Some(pb::entity::Kind::EventModel(model)) => {
                    models += 1;
                    tag(
                        validation::CHECK_MODEL,
                        Some(stored),
                        validation::validate_model_content(model, world),
                    );
                }
                Some(
                    pb::entity::Kind::CommandSlice(_)
                    | pb::entity::Kind::ReadModelSlice(_)
                    | pb::entity::Kind::AutomationSlice(_),
                ) => {
                    slices += 1;
                    tag(
                        validation::CHECK_SCENARIOS,
                        Some(stored),
                        validation::validate_scenarios(&stored.entity, world),
                    );
                }
                Some(pb::entity::Kind::Specification(_)) => has_specifications = true,
                Some(pb::entity::Kind::DataProduct(_)) => has_data_products = true,
                _ => {}
            }
        }
        tag(
            validation::CHECK_STRATEGIC,
            None,
            validation::strategic_chain_issues(namespaces.iter().copied(), world),
        );

        let skipped = |check_id: &str, reason: &str| {
            validation::coverage_check(
                check_id,
                pb::validation_check_result::State::Skipped,
                reason,
                0,
            )
        };
        let store_backed = |check_id: &str, applies: bool| {
            skipped(
                check_id,
                if applies {
                    REASON_DELTA_NOT_SUPPORTED
                } else {
                    validation::REASON_KIND_NOT_APPLICABLE
                },
            )
        };
        let count = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
        let checks = vec![
            validation::ran_against_snapshot(
                validation::CHECK_STORE_HYGIENE,
                1,
                snapshot_truncated,
            ),
            validation::ran_against_snapshot(validation::CHECK_ARCHITECTURE, 1, snapshot_truncated),
            validation::ran_against_snapshot(
                validation::CHECK_MODEL,
                count(models),
                snapshot_truncated,
            ),
            validation::ran_against_snapshot(
                validation::CHECK_STRATEGIC,
                count(namespaces.len()),
                snapshot_truncated,
            ),
            validation::ran_against_snapshot(
                validation::CHECK_SCENARIOS,
                count(slices),
                snapshot_truncated,
            ),
            store_backed(CHECK_SPECIFICATION_TYPES, has_specifications),
            store_backed(CHECK_DATA_PRODUCT_EVENTS, has_data_products),
            skipped(CHECK_DATA_FLOW, REASON_ANALYSIS_NOT_COMPARED),
            skipped(CHECK_INFORMATION_COMPLETENESS, REASON_ANALYSIS_NOT_COMPARED),
        ];
        let keys = world.iter().filter_map(stored_key).collect();
        Self {
            findings,
            coverage: validation::validation_coverage(
                checks,
                scope.clone(),
                scope.clone(),
                snapshot_truncated,
            ),
            keys,
        }
    }

    /// Why this scan cannot vouch for `suite`, or `None` when it completed.
    fn gap(&self, suite: &str) -> Option<String> {
        let Some(check) = self.coverage.checks.iter().find(|c| c.check_id == suite) else {
            return Some(REASON_SUITE_NOT_REPORTED.to_owned());
        };
        if check.state == pb::validation_check_result::State::Completed as i32 {
            return None;
        }
        Some(if check.reason.is_empty() {
            pb::validation_check_result::State::try_from(check.state)
                .map_or("STATE_UNSPECIFIED", |s| s.as_str_name())
                .to_owned()
        } else {
            check.reason.clone()
        })
    }

    fn leaves_out_applicable_suites(&self) -> bool {
        self.coverage
            .checks
            .iter()
            .any(|c| c.reason == REASON_DELTA_NOT_SUPPORTED)
    }

    fn grouped(&self) -> BTreeMap<FindingIdentity, Vec<&pb::ValidationIssue>> {
        let mut groups: BTreeMap<FindingIdentity, Vec<&pb::ValidationIssue>> = BTreeMap::new();
        for finding in &self.findings {
            groups
                .entry(FindingIdentity::of(finding))
                .or_default()
                .push(&finding.issue);
        }
        groups
    }
}

fn stored_key(stored: &StoredEntity) -> Option<pb::EntityKey> {
    Some(pb::EntityKey::new(
        entity_kind(&stored.entity)?,
        entity_id(&stored.entity)?,
    ))
}

/// Entities of `context` whose keys neither world holds. A context entry for
/// a key the worlds do hold is either a duplicate or, after the change, a
/// stale copy of something the change replaced or removed.
#[must_use]
pub fn context_outside(
    context: Vec<StoredEntity>,
    worlds: &[&[StoredEntity]],
) -> Vec<StoredEntity> {
    let held: HashSet<pb::EntityKey> = worlds
        .iter()
        .flat_map(|world| world.iter().filter_map(stored_key))
        .collect();
    context
        .into_iter()
        .filter(|stored| stored_key(stored).is_none_or(|key| !held.contains(&key)))
        .collect()
}

fn occurrences(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

struct Pending<'a> {
    identity: FindingIdentity,
    issues: Vec<&'a pb::ValidationIssue>,
}

/// Compare two scans of the same scope. The snapshot ids bind the report to
/// the worlds it was computed from.
#[must_use]
pub fn compare(
    before: &Scan,
    after: &Scan,
    before_snapshot_id: String,
    after_snapshot_id: String,
) -> pb::ConsequenceDelta {
    use pb::finding_change::Kind;

    let before_groups = before.grouped();
    let after_groups = after.grouped();
    let identities: BTreeSet<&FindingIdentity> =
        before_groups.keys().chain(after_groups.keys()).collect();
    let total = |groups: &BTreeMap<FindingIdentity, Vec<&pb::ValidationIssue>>,
                 identity: &FindingIdentity| {
        occurrences(groups.get(identity).map_or(0, Vec::len))
    };

    let mut changes: Vec<(FindingIdentity, pb::FindingChange)> = Vec::new();
    let mut unchanged = 0usize;
    let mut gone: Vec<Pending> = Vec::new();
    let mut new: Vec<Pending> = Vec::new();
    for identity in identities {
        let mut was: Vec<&pb::ValidationIssue> =
            before_groups.get(identity).cloned().unwrap_or_default();
        let mut now: Vec<&pb::ValidationIssue> =
            after_groups.get(identity).cloned().unwrap_or_default();
        was.retain(|issue| {
            match now
                .iter()
                .position(|other| other.severity == issue.severity)
            {
                Some(at) => {
                    now.remove(at);
                    unchanged += 1;
                    false
                }
                None => true,
            }
        });
        let shifted = was.len().min(now.len());
        if shifted > 0 {
            changes.push((
                identity.clone(),
                pb::FindingChange {
                    kind: Kind::SeverityChanged as i32,
                    check_id: identity.suite.to_owned(),
                    before: Some(was[0].clone()),
                    after: Some(now[0].clone()),
                    occurrences: occurrences(shifted),
                    occurrences_before: total(&before_groups, identity),
                    occurrences_after: total(&after_groups, identity),
                    reason: String::new(),
                    origin: identity.origin.as_ref().map(Subject::entity_ref),
                },
            ));
            was.drain(..shifted);
            now.drain(..shifted);
        }
        if !was.is_empty() {
            gone.push(Pending {
                identity: identity.clone(),
                issues: was,
            });
        }
        if !now.is_empty() {
            new.push(Pending {
                identity: identity.clone(),
                issues: now,
            });
        }
    }

    let mut predecessors: HashMap<FindingIdentity, Vec<(Versions, &FindingIdentity)>> =
        HashMap::new();
    for identity in before_groups.keys() {
        let (lineage, versions) = identity.lineage();
        predecessors
            .entry(lineage)
            .or_default()
            .push((versions, identity));
    }
    let mut gone_by_identity: HashMap<FindingIdentity, Pending> = gone
        .into_iter()
        .map(|pending| (pending.identity.clone(), pending))
        .collect();

    for pending in new {
        let (lineage, versions) = pending.identity.lineage();
        let predecessor = predecessors.get(&lineage).and_then(|candidates| {
            candidates
                .iter()
                .filter(|(v, _)| v.precedes(versions))
                .max_by_key(|(v, _)| *v)
                .map(|(_, identity)| *identity)
        });
        let identity = &pending.identity;
        let change = if let Some(predecessor) = predecessor {
            if let Some(left) = gone_by_identity.get_mut(predecessor) {
                let carried = left.issues.len().min(pending.issues.len());
                left.issues.drain(..carried);
            }
            pb::FindingChange {
                kind: Kind::CarriedToSuccessor as i32,
                check_id: identity.suite.to_owned(),
                before: before_groups
                    .get(predecessor)
                    .and_then(|issues| issues.first())
                    .map(|issue| (*issue).clone()),
                after: Some(pending.issues[0].clone()),
                occurrences: occurrences(pending.issues.len()),
                occurrences_before: total(&before_groups, predecessor),
                occurrences_after: total(&after_groups, identity),
                reason: String::new(),
                origin: identity.origin.as_ref().map(Subject::entity_ref),
            }
        } else {
            let gap = before.gap(identity.suite);
            pb::FindingChange {
                kind: if gap.is_some() {
                    Kind::Unknown
                } else {
                    Kind::Introduced
                } as i32,
                check_id: identity.suite.to_owned(),
                before: None,
                after: Some(pending.issues[0].clone()),
                occurrences: occurrences(pending.issues.len()),
                occurrences_before: total(&before_groups, identity),
                occurrences_after: total(&after_groups, identity),
                reason: gap.unwrap_or_default(),
                origin: identity.origin.as_ref().map(Subject::entity_ref),
            }
        };
        changes.push((identity.clone(), change));
    }

    for pending in gone_by_identity.into_values() {
        let Some(first) = pending.issues.first() else {
            continue;
        };
        let identity = &pending.identity;
        let removed = identity
            .entities()
            .any(|key| before.keys.contains(&key) && !after.keys.contains(&key));
        let gap = (!removed).then(|| after.gap(identity.suite)).flatten();
        let kind = match (removed, &gap) {
            (true, _) => Kind::SubjectRemoved,
            (false, Some(_)) => Kind::Unknown,
            (false, None) => Kind::Resolved,
        };
        changes.push((
            identity.clone(),
            pb::FindingChange {
                kind: kind as i32,
                check_id: identity.suite.to_owned(),
                before: Some((*first).clone()),
                after: None,
                occurrences: occurrences(pending.issues.len()),
                occurrences_before: total(&before_groups, identity),
                occurrences_after: total(&after_groups, identity),
                reason: gap.unwrap_or_default(),
                origin: identity.origin.as_ref().map(Subject::entity_ref),
            },
        ));
    }

    changes.sort_by(|(a, x), (b, y)| a.cmp(b).then(x.kind.cmp(&y.kind)));
    let complete = !before.coverage.snapshot_truncated
        && !after.coverage.snapshot_truncated
        && !before.leaves_out_applicable_suites()
        && !after.leaves_out_applicable_suites()
        && COMPARED_SUITES
            .iter()
            .all(|suite| before.gap(suite).is_none() && after.gap(suite).is_none());
    pb::ConsequenceDelta {
        changes: changes.into_iter().map(|(_, change)| change).collect(),
        unchanged_count: occurrences(unchanged),
        before_coverage: Some(before.coverage.clone()),
        after_coverage: Some(after.coverage.clone()),
        complete,
        before_snapshot_id,
        after_snapshot_id,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use pb::{finding_change::Kind, validation_check_result::State, validation_issue::Severity};

    use super::*;

    fn subject(slug: &str, version: u64) -> pb::EntityRef {
        pb::EntityRef {
            kind: pb::EntityKind::EventModel as i32,
            id: Some(pb::Id {
                namespace: "shop".into(),
                slug: slug.into(),
                version,
            }),
        }
    }

    fn issue(code: &str, subject: pb::EntityRef, severity: Severity) -> pb::ValidationIssue {
        pb::ValidationIssue {
            severity: severity as i32,
            code: code.into(),
            message: format!("{code} prose"),
            subject: Some(subject),
            ..Default::default()
        }
    }

    fn scan(
        findings: Vec<pb::ValidationIssue>,
        keys: &[pb::EntityRef],
        state: impl Fn(&str) -> (State, &'static str),
    ) -> Scan {
        Scan {
            findings: findings
                .into_iter()
                .map(|issue| Finding {
                    suite: validation::CHECK_MODEL,
                    origin: None,
                    issue,
                })
                .collect(),
            coverage: pb::ValidationCoverage {
                checks: COMPARED_SUITES
                    .iter()
                    .map(|suite| {
                        let (state, reason) = state(suite);
                        validation::coverage_check(*suite, state, reason, 1)
                    })
                    .collect(),
                ..Default::default()
            },
            keys: keys.iter().filter_map(|r| Subject::of(r).key()).collect(),
        }
    }

    fn complete(_: &str) -> (State, &'static str) {
        (State::Completed, "")
    }

    fn delta(before: &Scan, after: &Scan) -> pb::ConsequenceDelta {
        compare(before, after, "before".into(), "after".into())
    }

    fn kinds(delta: &pb::ConsequenceDelta) -> Vec<(i32, u32)> {
        delta
            .changes
            .iter()
            .map(|c| (c.kind, c.occurrences))
            .collect()
    }

    #[test]
    fn identical_scans_report_no_change() {
        let m = subject("checkout", 1);
        let findings = vec![issue("DANGLING_REF", m.clone(), Severity::Error)];
        let before = scan(findings.clone(), std::slice::from_ref(&m), complete);
        let after = scan(findings, &[m], complete);
        let delta = delta(&before, &after);
        assert!(delta.changes.is_empty(), "{delta:?}");
        assert_eq!(delta.unchanged_count, 1);
        assert!(delta.complete);
    }

    #[test]
    fn prose_does_not_change_identity() {
        let m = subject("checkout", 1);
        let mut reworded = issue("DANGLING_REF", m.clone(), Severity::Error);
        reworded.message = "different words".into();
        let before = scan(
            vec![issue("DANGLING_REF", m.clone(), Severity::Error)],
            std::slice::from_ref(&m),
            complete,
        );
        let after = scan(vec![reworded], &[m], complete);
        assert_eq!(delta(&before, &after).changes, Vec::new());
    }

    #[test]
    fn introduced_and_resolved_need_complete_scans() {
        let m = subject("checkout", 1);
        let before = scan(
            vec![issue("A", m.clone(), Severity::Error)],
            std::slice::from_ref(&m),
            complete,
        );
        let after = scan(
            vec![issue("B", m.clone(), Severity::Error)],
            std::slice::from_ref(&m),
            complete,
        );
        assert_eq!(
            kinds(&delta(&before, &after)),
            vec![(Kind::Resolved as i32, 1), (Kind::Introduced as i32, 1)]
        );
    }

    #[test]
    fn a_finding_is_not_resolved_when_the_after_scan_skipped_its_suite() {
        let m = subject("checkout", 1);
        let before = scan(
            vec![issue("A", m.clone(), Severity::Error)],
            std::slice::from_ref(&m),
            complete,
        );
        let after = scan(Vec::new(), &[m], |suite| {
            if suite == validation::CHECK_MODEL {
                (State::Skipped, "write_path")
            } else {
                (State::Completed, "")
            }
        });
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::Unknown as i32, 1)]);
        assert_eq!(delta.changes[0].reason, "write_path");
        assert!(delta.changes[0].before.is_some() && delta.changes[0].after.is_none());
        assert!(!delta.complete);
    }

    #[test]
    fn a_finding_is_not_resolved_when_the_after_scan_omitted_its_suite() {
        let m = subject("checkout", 1);
        let before = scan(
            vec![issue("A", m.clone(), Severity::Error)],
            std::slice::from_ref(&m),
            complete,
        );
        let mut after = scan(Vec::new(), &[m], complete);
        after
            .coverage
            .checks
            .retain(|c| c.check_id != validation::CHECK_MODEL);
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::Unknown as i32, 1)]);
        assert_eq!(delta.changes[0].reason, REASON_SUITE_NOT_REPORTED);
    }

    #[test]
    fn a_finding_is_not_new_when_the_before_scan_was_partial() {
        let m = subject("checkout", 1);
        let before = scan(Vec::new(), std::slice::from_ref(&m), |_| {
            (State::Partial, "snapshot_truncated")
        });
        let after = scan(vec![issue("A", m.clone(), Severity::Error)], &[m], complete);
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::Unknown as i32, 1)]);
        assert_eq!(delta.changes[0].reason, "snapshot_truncated");
        assert!(delta.changes[0].after.is_some() && delta.changes[0].before.is_none());
    }

    #[test]
    fn truncation_makes_the_delta_incomplete() {
        let m = subject("checkout", 1);
        let before = scan(Vec::new(), std::slice::from_ref(&m), complete);
        let mut after = scan(Vec::new(), &[m], complete);
        after.coverage.snapshot_truncated = true;
        assert!(!delta(&before, &after).complete);
    }

    #[test]
    fn an_applicable_suite_the_delta_cannot_run_makes_it_incomplete() {
        let m = subject("checkout", 1);
        let before = scan(Vec::new(), std::slice::from_ref(&m), complete);
        let mut after = scan(Vec::new(), &[m], complete);
        after.coverage.checks.push(validation::coverage_check(
            CHECK_SPECIFICATION_TYPES,
            State::Skipped,
            REASON_DELTA_NOT_SUPPORTED,
            0,
        ));
        assert!(!delta(&before, &after).complete);
    }

    #[test]
    fn severity_changes_are_reported_once_per_identity() {
        let m = subject("checkout", 1);
        let before = scan(
            vec![issue("A", m.clone(), Severity::Warning)],
            std::slice::from_ref(&m),
            complete,
        );
        let after = scan(vec![issue("A", m.clone(), Severity::Error)], &[m], complete);
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::SeverityChanged as i32, 1)]);
        assert_eq!(
            delta.changes[0].before.as_ref().unwrap().severity,
            Severity::Warning as i32
        );
        assert_eq!(
            delta.changes[0].after.as_ref().unwrap().severity,
            Severity::Error as i32
        );
    }

    #[test]
    fn repeated_occurrences_are_counted() {
        let m = subject("checkout", 1);
        let one = issue("A", m.clone(), Severity::Warning);
        let before = scan(
            vec![one.clone(), one.clone()],
            std::slice::from_ref(&m),
            complete,
        );
        let after = scan(vec![one.clone(), one.clone(), one], &[m], complete);
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::Introduced as i32, 1)]);
        assert_eq!(
            (
                delta.changes[0].occurrences_before,
                delta.changes[0].occurrences_after
            ),
            (2, 3)
        );
        assert_eq!(delta.unchanged_count, 2);
    }

    #[test]
    fn findings_on_a_removed_subject_go_with_it() {
        let m = subject("checkout", 1);
        let before = scan(vec![issue("A", m.clone(), Severity::Error)], &[m], complete);
        let after = scan(Vec::new(), &[], complete);
        assert_eq!(
            kinds(&delta(&before, &after)),
            vec![(Kind::SubjectRemoved as i32, 1)]
        );
    }

    #[test]
    fn a_successor_version_carries_its_predecessors_finding() {
        let v1 = subject("checkout", 1);
        let v2 = subject("checkout", 2);
        let before = scan(
            vec![issue("A", v1.clone(), Severity::Error)],
            std::slice::from_ref(&v1),
            complete,
        );
        let after = scan(
            vec![
                issue("A", v1.clone(), Severity::Error),
                issue("A", v2.clone(), Severity::Error),
            ],
            &[v1, v2],
            complete,
        );
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::CarriedToSuccessor as i32, 1)]);
        assert_eq!(delta.unchanged_count, 1);
    }

    #[test]
    fn a_successor_replacing_its_predecessor_consumes_the_old_finding() {
        let v1 = subject("checkout", 1);
        let v2 = subject("checkout", 2);
        let before = scan(
            vec![issue("A", v1.clone(), Severity::Error)],
            &[v1],
            complete,
        );
        let after = scan(
            vec![issue("A", v2.clone(), Severity::Error)],
            &[v2],
            complete,
        );
        assert_eq!(
            kinds(&delta(&before, &after)),
            vec![(Kind::CarriedToSuccessor as i32, 1)]
        );
    }

    fn originating(mut scan: Scan, origins: &[pb::EntityRef]) -> Scan {
        for (finding, origin) in scan.findings.iter_mut().zip(origins) {
            finding.origin = Some(Subject::of(origin));
        }
        scan
    }

    #[test]
    fn one_rule_on_one_subject_from_two_origins_stays_two_findings() {
        let event = subject("placed", 1);
        let (a, b) = (subject("a", 1), subject("b", 1));
        let before = originating(
            scan(
                vec![issue("A", event.clone(), Severity::Error)],
                &[a.clone(), b.clone()],
                complete,
            ),
            std::slice::from_ref(&a),
        );
        let after = originating(
            scan(
                vec![
                    issue("A", event.clone(), Severity::Error),
                    issue("A", event, Severity::Error),
                ],
                &[a.clone(), b.clone()],
                complete,
            ),
            &[a, b.clone()],
        );
        let delta = delta(&before, &after);
        assert_eq!(kinds(&delta), vec![(Kind::Introduced as i32, 1)]);
        assert_eq!(delta.changes[0].origin, Some(b));
    }

    #[test]
    fn a_finding_whose_origin_is_removed_is_not_resolved() {
        let event = subject("placed", 1);
        let model = subject("checkout", 1);
        let before = originating(
            scan(
                vec![issue("A", event.clone(), Severity::Error)],
                &[event.clone(), model.clone()],
                complete,
            ),
            &[model],
        );
        let after = scan(vec![], &[event], complete);
        assert_eq!(
            kinds(&delta(&before, &after)),
            vec![(Kind::SubjectRemoved as i32, 1)]
        );
    }

    #[test]
    fn a_finding_on_a_newer_origin_is_carried() {
        let event = subject("placed", 1);
        let (v1, v2) = (subject("checkout", 1), subject("checkout", 2));
        let before = originating(
            scan(
                vec![issue("A", event.clone(), Severity::Error)],
                &[event.clone(), v1.clone()],
                complete,
            ),
            &[v1],
        );
        let after = originating(
            scan(
                vec![issue("A", event.clone(), Severity::Error)],
                &[event, v2.clone()],
                complete,
            ),
            &[v2],
        );
        assert_eq!(
            kinds(&delta(&before, &after)),
            vec![(Kind::CarriedToSuccessor as i32, 1)]
        );
    }

    #[test]
    fn structured_issue_fields_beyond_the_known_ones_join_identity() {
        let m = subject("checkout", 1);
        let identity = FindingIdentity::of(&Finding {
            suite: validation::CHECK_MODEL,
            origin: None,
            issue: issue("A", m.clone(), Severity::Error),
        });
        assert_eq!(identity.structure, Vec::<u8>::new());
        let mut other = identity.clone();
        other.structure = vec![1];
        assert_ne!(identity, other);
    }
}
