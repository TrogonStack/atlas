use std::collections::HashSet;

use tonic::Status;
use trogon_atlas_proto as pb;

use crate::validation::architecture_resources::ArchitectureContext;

pub(crate) fn subjects(entity: &pb::Entity) -> &[pb::SubjectAssociation] {
    match &entity.kind {
        Some(pb::entity::Kind::Decision(value)) => &value.subjects,
        Some(pb::entity::Kind::Document(value)) => &value.subjects,
        _ => &[],
    }
}

pub(crate) fn documents(entity: &pb::Entity) -> Vec<&pb::DocumentSelection> {
    match &entity.kind {
        Some(pb::entity::Kind::Decision(value)) => value
            .documents
            .iter()
            .filter_map(|v| v.document.as_ref())
            .collect(),
        Some(pb::entity::Kind::Document(value)) => value
            .navigation
            .iter()
            .filter_map(|v| v.document.as_ref())
            .chain(
                value
                    .related_content
                    .iter()
                    .filter_map(|v| v.document.as_ref()),
            )
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn responsibility_subject(subject: &pb::CatalogSubject) -> pb::ResponsibilitySubject {
    trogon_atlas_core::decision_document::as_responsibility_subject(subject)
}

pub(crate) fn occurrence_count(entity: &pb::Entity) -> usize {
    match &entity.kind {
        Some(pb::entity::Kind::Decision(value)) => {
            value.subjects.len()
                + value.related_decisions.len()
                + value.documents.len()
                + usize::from(value.replaced_by.is_some())
        }
        Some(pb::entity::Kind::Document(value)) => {
            value.subjects.len()
                + value.navigation.len()
                + value.related_content.len()
                + value.anchors.len()
        }
        _ => crate::responsibility_resources::occurrence_count(entity),
    }
}

#[derive(Default)]
pub(crate) struct CatalogBudget {
    rows: HashSet<pb::EntityKey>,
    inventoried: HashSet<pb::EntityKey>,
    selections: HashSet<(pb::EntityKey, u8)>,
    examined: usize,
    output: usize,
}

impl CatalogBudget {
    pub(crate) fn examine(&mut self, count: usize) -> Result<(), Status> {
        self.examined = self.examined.saturating_add(count);
        if self.examined > 10_000 {
            return Err(Status::resource_exhausted("catalog work budget exceeded"));
        }
        Ok(())
    }

    pub(crate) fn owner(&mut self, entity: &pb::Entity) -> Result<(), Status> {
        let key = pb::EntityKey::new(
            crate::conv::entity_kind(entity)?,
            crate::conv::entity_id(entity)?,
        );
        if crate::validation::architecture_resources::is_resource(entity)
            && self.rows.insert(key.clone())
            && self.rows.len() > 5_000
        {
            return Err(Status::resource_exhausted(
                "catalog resource budget exceeded",
            ));
        }
        if self.inventoried.insert(key) {
            self.examine(occurrence_count(entity))?;
        }
        Ok(())
    }

    pub(crate) fn output(&mut self) -> Result<(), Status> {
        self.output += 1;
        if self.output > 4_096 {
            return Err(Status::resource_exhausted("catalog output budget exceeded"));
        }
        Ok(())
    }

    pub(crate) fn selection(
        &mut self,
        subject: &pb::CatalogSubject,
        owner: &pb::Entity,
    ) -> Result<(), Status> {
        let key = pb::EntityKey::new(
            crate::conv::entity_kind(owner)?,
            crate::conv::entity_id(owner)?,
        );
        let kind = match &subject.selection {
            Some(pb::catalog_subject::Selection::Connection(_)) => 1,
            Some(pb::catalog_subject::Selection::Member(_)) => 2,
            Some(pb::catalog_subject::Selection::Port(_)) => 3,
            _ => 0,
        };
        if !self.selections.insert((key, kind)) {
            return Ok(());
        }
        let count = match kind {
            1 if matches!(
                owner.kind,
                Some(pb::entity::Kind::Component(_) | pb::entity::Kind::ExternalSystem(_))
            ) =>
            {
                0
            }
            1 => trogon_atlas_core::connections::connection_occurrences(owner)
                .map_err(|_| Status::internal("cannot inspect native connection inventory"))?
                .len(),
            2 => match &owner.kind {
                Some(pb::entity::Kind::Specification(value)) => {
                    value.artifacts.len()
                        + value.schemas.len()
                        + value.messages.len()
                        + value.operations.len()
                }
                _ => 0,
            },
            3 => match &owner.kind {
                Some(pb::entity::Kind::DataProduct(value)) => {
                    value.inputs.len() + value.outputs.len()
                }
                _ => 0,
            },
            _ => 0,
        };
        self.examine(count)
    }
}

fn replacement_resolves(decision: &pb::Decision, context: &ArchitectureContext<'_>) -> bool {
    let Some(replacement) = &decision.replaced_by else {
        return true;
    };
    let mut next = replacement
        .decision
        .as_ref()
        .and_then(|reference| reference.id.as_ref());
    let mut visited = HashSet::new();
    if let Some(id) = &decision.id {
        visited.insert(id);
    }
    for _ in 0..10_000 {
        let Some(id) = next else { return false };
        if id.version == 0 || !visited.insert(id) {
            return false;
        }
        let Some(pb::Entity {
            kind: Some(pb::entity::Kind::Decision(target)),
            ..
        }) = context.find(pb::EntityKind::Decision, id)
        else {
            return false;
        };
        match pb::DecisionStatus::try_from(target.status) {
            Ok(pb::DecisionStatus::Accepted) => return true,
            Ok(pb::DecisionStatus::Superseded) => {
                next = target
                    .replaced_by
                    .as_ref()
                    .and_then(|replacement| replacement.decision.as_ref())
                    .and_then(|reference| reference.id.as_ref());
            }
            _ => return false,
        }
    }
    false
}

fn has_dedicated_check(field: &str) -> bool {
    matches!(field, "replaced_by.decision" | "supersedes")
        || ["subjects[", "documents[", "navigation[", "related_content["]
            .iter()
            .any(|prefix| field.starts_with(prefix))
}

pub(crate) fn validate_in_context(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Vec<pb::ValidationIssue> {
    let mut issues = Vec::new();
    let mut issue = |field: &str, message: &str| {
        issues.push(crate::validation::issue_error(
            "CATALOG_REFERENCE_UNRESOLVED",
            message.into(),
            crate::conv::entity_to_ref(entity).ok(),
            field,
        ));
    };
    if let Err(error) = trogon_atlas_core::decision_document::validate_values(entity) {
        issue("catalog", &error.to_string());
    }
    if let Some(pb::entity::Kind::Decision(decision)) = &entity.kind {
        if !replacement_resolves(decision, context) {
            issue(
                "replaced_by.decision",
                "Replacement chain is absent, stale or unavailable",
            );
        }
    }
    for reference in trogon_atlas_core::refs::outbound_refs(entity) {
        if has_dedicated_check(&reference.from_field) {
            continue;
        }
        if !reference.to_kind.is_some_and(|kind| {
            reference.to_id.version > 0 && context.find(kind, &reference.to_id).is_some()
        }) {
            issue(
                &reference.from_field,
                "Catalog declaration is absent, stale or unavailable",
            );
        }
    }
    for (index, association) in subjects(entity).iter().enumerate() {
        let resolves = association.subject.as_ref().is_some_and(|subject| {
            let converted = responsibility_subject(subject);
            crate::responsibility_resources::subject_owner(&converted)
                .and_then(|reference| {
                    context.find(
                        pb::EntityKind::try_from(reference.kind).ok()?,
                        reference.id.as_ref()?,
                    )
                })
                .is_some_and(|owner| {
                    trogon_atlas_core::decision_document::resolve_subject(subject, owner).is_ok()
                        && crate::responsibility_resources::subject_resolves(&converted, context)
                        && crate::responsibility_resources::selected_endpoints(&converted, owner)
                            .into_iter()
                            .all(|(kind, id)| context.find(kind, &id).is_some())
                })
        });
        if !resolves {
            issue(
                &format!("subjects[{index}].subject"),
                "Catalog subject is absent, stale or unavailable",
            );
        }
    }
    let (field, selections): (_, Vec<_>) = match &entity.kind {
        Some(pb::entity::Kind::Decision(value)) => (
            "documents",
            value
                .documents
                .iter()
                .map(|association| &association.document)
                .collect(),
        ),
        Some(pb::entity::Kind::Document(value)) => {
            for (index, association) in value.related_content.iter().enumerate() {
                let Some(selection) = &association.document else {
                    continue;
                };
                let role = pb::RelatedContentRole::try_from(association.role).ok();
                let resolves = trogon_atlas_core::decision_document::document_owner(selection)
                    .ok()
                    .and_then(|reference| {
                        context.find(pb::EntityKind::Document, reference.id.as_ref()?)
                    })
                    .is_some_and(|owner| {
                        trogon_atlas_core::decision_document::resolve_document_selection(
                            selection, owner,
                        )
                        .is_ok()
                            && role.is_some_and(|role| {
                                trogon_atlas_core::document_catalog_context::resolve_related_purpose_agreement(role, owner).is_ok()
                            })
                    });
                if !resolves {
                    issue(
                        &format!("related_content[{index}].document"),
                        "Related content selection is absent, stale, unavailable or conflicts with declared purpose",
                    );
                }
            }
            (
                "navigation",
                value
                    .navigation
                    .iter()
                    .map(|association| &association.document)
                    .collect(),
            )
        }
        _ => ("", Vec::new()),
    };
    for (index, selection) in selections.into_iter().enumerate() {
        let Some(selection) = selection else { continue };

        let resolves = trogon_atlas_core::decision_document::document_owner(selection)
            .ok()
            .and_then(|reference| context.find(pb::EntityKind::Document, reference.id.as_ref()?))
            .is_some_and(|owner| {
                trogon_atlas_core::decision_document::resolve_document_selection(selection, owner)
                    .is_ok()
            });
        if !resolves {
            issue(
                &format!("{field}[{index}].document"),
                "Document selection is absent, stale or unavailable",
            );
        }
    }
    let (kind, current, prior) = match &entity.kind {
        Some(pb::entity::Kind::Decision(value)) => (
            pb::EntityKind::Decision,
            value.id.as_ref(),
            value.supersedes.as_ref(),
        ),
        Some(pb::entity::Kind::Document(value)) => (
            pb::EntityKind::Document,
            value.id.as_ref(),
            value.supersedes.as_ref(),
        ),
        _ => return issues,
    };
    if let Some(prior) = prior {
        if !current.is_some_and(|current| {
            current.namespace == prior.namespace
                && current.slug == prior.slug
                && prior.version > 0
                && prior.version < current.version
                && context.find(kind, prior).is_some()
        }) {
            issue(
                "supersedes",
                "Version ancestry must select an existing exact earlier version in the same family",
            );
        }
    }
    crate::validation::enrich_issues_with_catalog(&mut issues);
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_supersedes_target_is_reported_once() {
        let id = |version| pb::Id {
            namespace: "catalog".into(),
            slug: "choice".into(),
            version,
        };
        for supersedes in [id(1), id(0)] {
            let entity = pb::Entity {
                kind: Some(pb::entity::Kind::Decision(pb::Decision {
                    id: Some(id(2)),
                    supersedes: Some(supersedes),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let issues = validate_in_context(&entity, &ArchitectureContext::new([]));
            let ancestry: Vec<_> = issues
                .iter()
                .filter(|issue| issue.subject_field == "supersedes")
                .collect();
            assert_eq!(ancestry.len(), 1, "{issues:?}");
        }
    }

    #[test]
    fn broken_associations_are_reported_once_each() {
        let missing = |slug: &str| pb::Id {
            namespace: "catalog".into(),
            slug: slug.into(),
            version: 1,
        };
        let selection = || pb::DocumentSelection {
            selection: Some(pb::document_selection::Selection::Document(
                pb::DocumentRef {
                    id: Some(missing("note")),
                },
            )),
        };
        let subjects = vec![pb::SubjectAssociation {
            subject: Some(pb::CatalogSubject {
                selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                    kind: pb::EntityKind::ReadModel as i32,
                    id: Some(missing("receipt")),
                })),
            }),
            ..Default::default()
        }];
        let decision = pb::Entity {
            kind: Some(pb::entity::Kind::Decision(pb::Decision {
                id: Some(missing("choice")),
                subjects: subjects.clone(),
                related_decisions: vec![pb::RelatedDecision {
                    decision: Some(pb::DecisionRef {
                        id: Some(missing("other")),
                    }),
                    ..Default::default()
                }],
                documents: vec![pb::SupportingDocument {
                    document: Some(selection()),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let document = pb::Entity {
            kind: Some(pb::entity::Kind::Document(pb::Document {
                id: Some(missing("guide")),
                subjects,
                navigation: vec![pb::DocumentNavigation {
                    document: Some(selection()),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        for (entity, expected) in [
            (
                decision,
                vec![
                    "related_decisions[0].decision",
                    "subjects[0].subject",
                    "documents[0].document",
                ],
            ),
            (
                document,
                vec!["subjects[0].subject", "navigation[0].document"],
            ),
        ] {
            let issues = validate_in_context(&entity, &ArchitectureContext::new([]));
            let fields: Vec<_> = issues
                .iter()
                .map(|issue| issue.subject_field.as_str())
                .filter(|field| *field != "catalog")
                .collect();
            assert_eq!(fields, expected, "{issues:?}");
        }
    }
}
