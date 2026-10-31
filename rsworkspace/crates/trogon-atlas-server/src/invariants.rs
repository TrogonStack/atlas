//! Invariants declared on event models and the verdict a candidate world
//! earns against them. Pure: the service decides which worlds to compare and
//! whether a verdict is still current. See docs/explanation/invariant-gates.md.

use std::collections::{HashMap, HashSet};

use pb::{
    invariant_annotation::{Severity as ForbiddenSeverity, Unevaluable},
    invariant_outcome::{Reason, Status},
};
use prost::Message as _;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id, entity_kind},
    StoredEntity,
};

use crate::validation;

/// Invariants one evaluation checks before the rest are reported as
/// unevaluable.
pub const DEFAULT_BUDGET: usize = 64;

const TYPE_URL_SUFFIX: &str = "trogonatlas.eventmodel.v1alpha1.InvariantAnnotation";

fn is_invariant_annotation(any: &prost_types::Any) -> bool {
    any.type_url.ends_with(TYPE_URL_SUFFIX)
}

/// True when `entity` declares at least one invariant, well formed or not.
#[must_use]
pub fn declares_invariants(entity: &pb::Entity) -> bool {
    validation::entity_metadata(entity)
        .iter()
        .any(is_invariant_annotation)
}

/// Why a declaration cannot be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationError {
    NotAnEventModel,
    Undecodable { index: usize },
    MissingName { index: usize },
    ForbidsNothing { name: String },
    DuplicateName { name: String },
}

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnEventModel => {
                f.write_str("InvariantAnnotation may only be declared on an EventModel")
            }
            Self::Undecodable { index } => {
                write!(
                    f,
                    "metadata[{index}] is not a decodable InvariantAnnotation"
                )
            }
            Self::MissingName { index } => {
                write!(f, "metadata[{index}] InvariantAnnotation has no name")
            }
            Self::ForbidsNothing { name } => write!(
                f,
                "invariant {name:?} forbids nothing: set forbidden_codes or forbidden_severity"
            ),
            Self::DuplicateName { name } => {
                write!(f, "invariant {name:?} is declared more than once")
            }
        }
    }
}

/// Write-time check of every invariant `entity` declares.
pub fn check_declarations(entity: &pb::Entity) -> Result<(), DeclarationError> {
    if !declares_invariants(entity) {
        return Ok(());
    }
    if !matches!(entity.kind, Some(pb::entity::Kind::EventModel(_))) {
        return Err(DeclarationError::NotAnEventModel);
    }
    let mut names = HashSet::new();
    for declared in declarations(entity) {
        let annotation = declared.annotation?;
        if !names.insert(annotation.name.clone()) {
            return Err(DeclarationError::DuplicateName {
                name: annotation.name,
            });
        }
    }
    Ok(())
}

struct Declared {
    annotation: Result<pb::InvariantAnnotation, DeclarationError>,
}

fn declarations(entity: &pb::Entity) -> Vec<Declared> {
    validation::entity_metadata(entity)
        .iter()
        .enumerate()
        .filter(|(_, any)| is_invariant_annotation(any))
        .map(|(index, any)| Declared {
            annotation: decode(index, any),
        })
        .collect()
}

fn decode(
    index: usize,
    any: &prost_types::Any,
) -> Result<pb::InvariantAnnotation, DeclarationError> {
    let annotation = pb::InvariantAnnotation::decode(any.value.as_slice())
        .map_err(|_| DeclarationError::Undecodable { index })?;
    if annotation.name.trim().is_empty() {
        return Err(DeclarationError::MissingName { index });
    }
    if annotation.forbidden_codes.is_empty()
        && annotation.forbidden_severity() == ForbiddenSeverity::Unspecified
    {
        return Err(DeclarationError::ForbidsNothing {
            name: annotation.name,
        });
    }
    Ok(annotation)
}

/// Each declaration's name and whether it only advises when it cannot be
/// evaluated. A malformed declaration has no trustworthy name and blocks.
pub fn declared_names(entity: &pb::Entity) -> Vec<(String, bool)> {
    declarations(entity)
        .into_iter()
        .map(|declared| match declared.annotation {
            Ok(annotation) => (annotation.name.clone(), advisory(&annotation)),
            Err(_) => (String::new(), false),
        })
        .collect()
}

fn advisory(annotation: &pb::InvariantAnnotation) -> bool {
    annotation.on_unevaluable() == Unevaluable::Advise
}

fn forbids(annotation: &pb::InvariantAnnotation, issue: &pb::ValidationIssue) -> bool {
    use pb::validation_issue::Severity;
    if annotation.forbidden_codes.contains(&issue.code) {
        return true;
    }
    let threshold = match annotation.forbidden_severity() {
        ForbiddenSeverity::Unspecified => return false,
        ForbiddenSeverity::Warning => Severity::Warning,
        ForbiddenSeverity::Error => Severity::Error,
    };
    issue.severity >= threshold as i32
}

/// A finding with every version, severity and prose erased, so the same
/// finding on a successor version counts as already present.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Lineage {
    code: String,
    subject: Option<(i32, String, String)>,
    field: String,
    structure: Vec<u8>,
}

impl Lineage {
    fn of(issue: &pb::ValidationIssue) -> Self {
        let mut structure = issue.clone();
        structure.severity = 0;
        structure.code.clear();
        structure.message.clear();
        structure.subject = None;
        structure.subject_field.clear();
        structure.rule_title.clear();
        structure.rule_category.clear();
        structure.rule_default_severity = 0;
        Self {
            code: issue.code.clone(),
            subject: issue.subject.as_ref().map(|s| {
                let id = s.id.clone().unwrap_or_default();
                (s.kind, id.namespace, id.slug)
            }),
            field: issue.subject_field.clone(),
            structure: structure.encode_to_vec(),
        }
    }
}

fn key_of(stored: &StoredEntity) -> Option<pb::EntityKey> {
    Some(pb::EntityKey::new(
        entity_kind(&stored.entity)?,
        entity_id(&stored.entity)?,
    ))
}

fn ref_of(key: &pb::EntityKey) -> pb::EntityRef {
    pb::EntityRef {
        kind: key.kind as i32,
        id: Some(pb::Id {
            namespace: key.namespace.clone(),
            slug: key.slug.clone(),
            version: key.version,
        }),
    }
}

fn model_of(stored: &StoredEntity) -> Option<&pb::EventModel> {
    match stored.entity.kind.as_ref() {
        Some(pb::entity::Kind::EventModel(model)) => Some(model),
        _ => None,
    }
}

/// The candidate world and the world it replaces, with what is known about
/// how faithfully they were read.
pub struct Worlds<'a> {
    pub before: &'a [StoredEntity],
    pub after: &'a [StoredEntity],
    /// The snapshot `before` came from hit the entity cap.
    pub truncated: bool,
}

/// One outcome per invariant declared in `after`, ordered by carrier key
/// then declaration order. Unredacted.
#[must_use]
pub fn evaluate(worlds: &Worlds<'_>, budget: usize) -> Vec<pb::InvariantOutcome> {
    let mut carriers: Vec<(pb::EntityKey, &StoredEntity)> = worlds
        .after
        .iter()
        .filter(|se| declares_invariants(&se.entity))
        .filter_map(|se| Some((key_of(se)?, se)))
        .collect();
    carriers.sort_by(|a, b| sort_key(&a.0).cmp(&sort_key(&b.0)));

    let known: HashSet<pb::EntityKey> = worlds.after.iter().filter_map(key_of).collect();
    let before_by_lineage = latest_by_lineage(worlds.before);
    let before_by_key: HashMap<pb::EntityKey, &StoredEntity> = worlds
        .before
        .iter()
        .filter_map(|se| Some((key_of(se)?, se)))
        .collect();

    let mut outcomes = Vec::new();
    let mut evaluated = 0usize;
    for (key, stored) in carriers {
        let carrier = ref_of(&key);
        let Some(model) = model_of(stored) else {
            continue;
        };
        let previous = before_by_key.get(&key).copied().or_else(|| {
            before_by_lineage
                .get(&(key.namespace.clone(), key.slug.clone()))
                .copied()
        });
        let mut after_findings: Option<Vec<pb::ValidationIssue>> = None;
        let mut before_findings: Option<Vec<pb::ValidationIssue>> = None;
        for declared in declarations(&stored.entity) {
            let mut outcome = pb::InvariantOutcome {
                carrier: Some(carrier.clone()),
                ..Default::default()
            };
            let Ok(annotation) = declared.annotation else {
                outcome.set_status(Status::Unevaluable);
                outcome.set_reason(Reason::MalformedDeclaration);
                outcomes.push(outcome);
                continue;
            };
            outcome.name.clone_from(&annotation.name);
            outcome.advisory = advisory(&annotation);
            if evaluated >= budget {
                outcome.set_status(Status::Unevaluable);
                outcome.set_reason(Reason::BudgetExhausted);
                outcomes.push(outcome);
                continue;
            }
            evaluated += 1;
            if worlds.truncated {
                outcome.set_status(Status::Unevaluable);
                outcome.set_reason(Reason::SnapshotTruncated);
                outcomes.push(outcome);
                continue;
            }
            let after = after_findings
                .get_or_insert_with(|| validation::validate_model_content(model, worlds.after));
            let before = before_findings.get_or_insert_with(|| {
                previous
                    .and_then(model_of)
                    .map(|model| validation::validate_model_content(model, worlds.before))
                    .unwrap_or_default()
            });
            classify(&mut outcome, &annotation, before, after);
            if outcome.status() != Status::Violated {
                let missing: Vec<pb::EntityRef> = missing_members(model, &known);
                if !missing.is_empty() {
                    outcome.set_status(Status::Unevaluable);
                    outcome.set_reason(Reason::MissingInputs);
                    outcome.missing = missing;
                }
            }
            outcomes.push(outcome);
        }
    }
    outcomes
}

fn sort_key(key: &pb::EntityKey) -> (&str, &str, u64) {
    (&key.namespace, &key.slug, key.version)
}

fn latest_by_lineage(world: &[StoredEntity]) -> HashMap<(String, String), &StoredEntity> {
    let mut latest: HashMap<(String, String), &StoredEntity> = HashMap::new();
    for se in world {
        if model_of(se).is_none() {
            continue;
        }
        let Some(id) = entity_id(&se.entity) else {
            continue;
        };
        let lineage = (id.namespace.clone(), id.slug.clone());
        let newer = latest
            .get(&lineage)
            .and_then(|current| entity_id(&current.entity))
            .is_none_or(|current| current.version < id.version);
        if newer {
            latest.insert(lineage, se);
        }
    }
    latest
}

fn classify(
    outcome: &mut pb::InvariantOutcome,
    annotation: &pb::InvariantAnnotation,
    before: &[pb::ValidationIssue],
    after: &[pb::ValidationIssue],
) {
    let mut remaining: HashMap<Lineage, usize> = HashMap::new();
    for issue in before.iter().filter(|i| forbids(annotation, i)) {
        *remaining.entry(Lineage::of(issue)).or_default() += 1;
    }
    for issue in after {
        if !forbids(annotation, issue) {
            continue;
        }
        match remaining.get_mut(&Lineage::of(issue)) {
            Some(count) if *count > 0 => {
                *count -= 1;
                outcome.preexisting.push(issue.clone());
            }
            _ => outcome.violations.push(issue.clone()),
        }
    }
    outcome.set_status(if !outcome.violations.is_empty() {
        Status::Violated
    } else if outcome.preexisting.is_empty() {
        Status::Holds
    } else {
        Status::AlreadyViolated
    });
}

fn missing_members(model: &pb::EventModel, known: &HashSet<pb::EntityKey>) -> Vec<pb::EntityRef> {
    model
        .members
        .iter()
        .filter(|member| {
            let Ok(kind) = pb::EntityKind::try_from(member.kind) else {
                return false;
            };
            member
                .id
                .as_ref()
                .is_some_and(|id| !known.contains(&pb::EntityKey::new(kind, id)))
        })
        .cloned()
        .collect()
}

/// Whether an outcome stops the write it describes.
#[must_use]
pub fn blocks(outcome: &pb::InvariantOutcome) -> bool {
    match outcome.status() {
        Status::Violated => true,
        Status::Unevaluable => !outcome.advisory,
        Status::Holds | Status::AlreadyViolated | Status::Unspecified => false,
    }
}

/// The outcome a refusal names: a violation before anything unevaluable.
#[must_use]
pub fn refusal(outcomes: &[pb::InvariantOutcome]) -> Option<&pb::InvariantOutcome> {
    outcomes
        .iter()
        .find(|o| o.status() == Status::Violated)
        .or_else(|| outcomes.iter().find(|o| blocks(o)))
}

/// Clears what identifies `outcome` when the caller may not read its carrier.
pub fn redact(outcome: &mut pb::InvariantOutcome) {
    outcome.carrier = None;
    outcome.name.clear();
    outcome.violations.clear();
    outcome.preexisting.clear();
    outcome.missing.clear();
    outcome.redacted = true;
}

/// True when every outcome was evaluated and none is redacted.
#[must_use]
pub fn complete(outcomes: &[pb::InvariantOutcome]) -> bool {
    outcomes
        .iter()
        .all(|o| o.status() != Status::Unevaluable && !o.redacted)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn id(slug: &str) -> pb::Id {
        pb::Id {
            namespace: "shop".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn annotation(name: &str, codes: &[&str]) -> pb::InvariantAnnotation {
        pb::InvariantAnnotation {
            name: name.into(),
            forbidden_codes: codes.iter().map(|c| (*c).into()).collect(),
            ..Default::default()
        }
    }

    fn any(annotation: &pb::InvariantAnnotation) -> prost_types::Any {
        prost_types::Any {
            type_url: format!("type.googleapis.com/{TYPE_URL_SUFFIX}"),
            value: annotation.encode_to_vec(),
        }
    }

    fn model(members: &[pb::EntityRef], annotations: &[pb::InvariantAnnotation]) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
                    id: Some(id("checkout")),
                    title: "Checkout".into(),
                    members: members.to_vec(),
                    metadata: annotations.iter().map(any).collect(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            etag: "1".into(),
        }
    }

    fn event_ref(slug: &str) -> pb::EntityRef {
        pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(slug)),
        }
    }

    fn event(slug: &str) -> StoredEntity {
        StoredEntity {
            entity: pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(slug)),
                    ..Default::default()
                })),
                ..Default::default()
            },
            etag: "1".into(),
        }
    }

    fn member_codes(world: &[StoredEntity]) -> Vec<String> {
        let Some(model) = world.iter().find_map(model_of) else {
            return Vec::new();
        };
        validation::validate_model_content(model, world)
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    fn missing_member_code() -> String {
        let world = vec![model(&[event_ref("placed")], &[])];
        member_codes(&world)
            .into_iter()
            .next()
            .expect("a dangling member is a model finding")
    }

    #[test]
    fn declarations_only_ride_event_models() {
        let mut stored = event("placed");
        if let Some(pb::entity::Kind::Event(e)) = stored.entity.kind.as_mut() {
            e.metadata.push(any(&annotation("x", &["A"])));
        }
        assert_eq!(
            check_declarations(&stored.entity),
            Err(DeclarationError::NotAnEventModel)
        );
    }

    #[test]
    fn declaration_must_forbid_something_and_be_unique() {
        let empty = model(&[], &[annotation("x", &[])]);
        assert!(matches!(
            check_declarations(&empty.entity),
            Err(DeclarationError::ForbidsNothing { .. })
        ));
        let twice = model(&[], &[annotation("x", &["A"]), annotation("x", &["B"])]);
        assert!(matches!(
            check_declarations(&twice.entity),
            Err(DeclarationError::DuplicateName { .. })
        ));
        let unnamed = model(&[], &[annotation(" ", &["A"])]);
        assert!(matches!(
            check_declarations(&unnamed.entity),
            Err(DeclarationError::MissingName { .. })
        ));
    }

    #[test]
    fn introduced_forbidden_finding_violates() {
        let code = missing_member_code();
        let inv = annotation("members-resolve", &[&code]);
        let before = vec![
            model(&[event_ref("placed")], std::slice::from_ref(&inv)),
            event("placed"),
        ];
        let after = vec![model(&[event_ref("placed")], &[inv])];
        let outcomes = evaluate(
            &Worlds {
                before: &before,
                after: &after,
                truncated: false,
            },
            DEFAULT_BUDGET,
        );
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status(), Status::Violated);
        assert!(blocks(&outcomes[0]));
        assert_eq!(
            refusal(&outcomes).map(|o| o.name.as_str()),
            Some("members-resolve")
        );
    }

    #[test]
    fn finding_present_before_does_not_block() {
        let inv = annotation("members-have-ids", &["MISSING_ID"]);
        let unnamed = pb::EntityRef {
            kind: pb::EntityKind::Event as i32,
            id: None,
        };
        let world = vec![model(&[unnamed], &[inv])];
        let outcomes = evaluate(
            &Worlds {
                before: &world,
                after: &world,
                truncated: false,
            },
            DEFAULT_BUDGET,
        );
        assert_eq!(outcomes[0].status(), Status::AlreadyViolated);
        assert!(!blocks(&outcomes[0]));
    }

    #[test]
    fn missing_member_without_violation_is_unevaluable_and_fails_closed() {
        let inv = annotation("no-such-rule", &["NOT_A_REAL_CODE"]);
        let after = vec![model(&[event_ref("placed")], std::slice::from_ref(&inv))];
        let outcomes = evaluate(
            &Worlds {
                before: &[],
                after: &after,
                truncated: false,
            },
            DEFAULT_BUDGET,
        );
        assert_eq!(outcomes[0].status(), Status::Unevaluable);
        assert_eq!(outcomes[0].reason(), Reason::MissingInputs);
        assert_eq!(outcomes[0].missing, vec![event_ref("placed")]);
        assert!(blocks(&outcomes[0]));

        let mut advise = inv;
        advise.set_on_unevaluable(Unevaluable::Advise);
        let after = vec![model(&[event_ref("placed")], &[advise])];
        let outcomes = evaluate(
            &Worlds {
                before: &[],
                after: &after,
                truncated: false,
            },
            DEFAULT_BUDGET,
        );
        assert!(outcomes[0].advisory);
        assert!(!blocks(&outcomes[0]));
    }

    #[test]
    fn truncated_snapshot_is_unevaluable() {
        let inv = annotation("x", &["A"]);
        let after = vec![model(&[], &[inv])];
        let outcomes = evaluate(
            &Worlds {
                before: &[],
                after: &after,
                truncated: true,
            },
            DEFAULT_BUDGET,
        );
        assert_eq!(outcomes[0].reason(), Reason::SnapshotTruncated);
        assert!(!complete(&outcomes));
    }

    #[test]
    fn budget_leaves_the_rest_unevaluable() {
        let after = vec![model(
            &[],
            &[annotation("a", &["A"]), annotation("b", &["B"])],
        )];
        let outcomes = evaluate(
            &Worlds {
                before: &[],
                after: &after,
                truncated: false,
            },
            1,
        );
        assert_eq!(outcomes[0].status(), Status::Holds);
        assert_eq!(outcomes[1].reason(), Reason::BudgetExhausted);
    }

    #[test]
    fn severity_threshold_forbids_errors() {
        let mut inv = annotation("no-errors", &[]);
        inv.set_forbidden_severity(ForbiddenSeverity::Error);
        let issue = |severity: pb::validation_issue::Severity| pb::ValidationIssue {
            severity: severity as i32,
            code: "ANY".into(),
            ..Default::default()
        };
        assert!(forbids(&inv, &issue(pb::validation_issue::Severity::Error)));
        assert!(!forbids(
            &inv,
            &issue(pb::validation_issue::Severity::Warning)
        ));
    }

    #[test]
    fn redaction_keeps_status_and_marks_incomplete() {
        let mut outcome = pb::InvariantOutcome {
            carrier: Some(event_ref("placed")),
            name: "x".into(),
            ..Default::default()
        };
        outcome.set_status(Status::Holds);
        redact(&mut outcome);
        assert_eq!(outcome.status(), Status::Holds);
        assert!(outcome.carrier.is_none() && outcome.name.is_empty());
        assert!(!complete(&[outcome]));
    }
}
