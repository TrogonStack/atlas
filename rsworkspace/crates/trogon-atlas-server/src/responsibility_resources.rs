use trogon_atlas_proto as pb;

use crate::validation::architecture_resources::ArchitectureContext;

pub(crate) const MAX_RESPONSIBILITY_OCCURRENCES: usize = 10_000;

pub(crate) fn occurrence_count(entity: &pb::Entity) -> usize {
    assignments(entity)
        .len()
        .saturating_add(match &entity.kind {
            Some(pb::entity::Kind::Team(value)) => value.members.len(),
            Some(pb::entity::Kind::Component(value)) => value
                .relationships
                .len()
                .saturating_add(value.communications.len()),
            Some(pb::entity::Kind::ExternalSystem(value)) => value
                .relationships
                .len()
                .saturating_add(value.communications.len()),
            Some(pb::entity::Kind::Specification(value)) => value.message_channels.len(),
            Some(pb::entity::Kind::DataProduct(value)) => value.resource_associations.len(),
            _ => 0,
        })
}

pub(crate) fn assignments(entity: &pb::Entity) -> &[pb::Responsibility] {
    match &entity.kind {
        Some(pb::entity::Kind::Team(value)) => &value.responsibilities,
        Some(pb::entity::Kind::Person(value)) => &value.responsibilities,
        _ => &[],
    }
}

pub(crate) fn subject_owner(subject: &pb::ResponsibilitySubject) -> Option<pb::EntityRef> {
    use pb::responsibility_subject::Selection as S;
    match subject.selection.as_ref()? {
        S::Entity(reference) => Some(reference.clone()),
        S::Connection(reference) => reference.owner.clone(),
        S::Member(reference) => Some(pb::EntityRef {
            kind: pb::EntityKind::Specification as i32,
            id: reference.specification.as_ref()?.id.clone(),
        }),
        S::Port(reference) => Some(pb::EntityRef {
            kind: pb::EntityKind::DataProduct as i32,
            id: reference.product.as_ref()?.id.clone(),
        }),
    }
}

pub(crate) fn subject_resolves(
    subject: &pb::ResponsibilitySubject,
    context: &ArchitectureContext<'_>,
) -> bool {
    subject_owner(subject)
        .and_then(|reference| {
            context.find(
                pb::EntityKind::try_from(reference.kind).ok()?,
                reference.id.as_ref()?,
            )
        })
        .is_some_and(|owner| {
            trogon_atlas_core::responsibility::resolve_subject(subject, owner).is_ok()
                && selected_message(subject, owner).is_none_or(|selection| {
                    selection
                        .specification
                        .as_ref()
                        .and_then(|r| r.id.as_ref())
                        .and_then(|id| context.find(pb::EntityKind::Specification, id))
                        .is_some_and(|specification| {
                            trogon_atlas_core::specification::resolve_member(
                                specification,
                                &selection,
                            )
                            .is_ok()
                        })
                })
        })
}

pub(crate) fn validate_in_context(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Vec<pb::ValidationIssue> {
    let mut issues: Vec<_> = trogon_atlas_core::responsibility::validate_values(entity)
        .err()
        .map(|error| {
            crate::validation::issue_error(
                "RESPONSIBILITY_VALUE_INVALID",
                error.to_string(),
                crate::conv::entity_to_ref(entity).ok(),
                "responsibility",
            )
        })
        .into_iter()
        .collect();
    let mut issue = |code, field: &str| {
        issues.push(crate::validation::issue_error(
            code,
            "Responsibility declaration is invalid or its exact selection is absent, stale or inaccessible".into(),
            crate::conv::entity_to_ref(entity).ok(),
            field,
        ));
    };
    let prior = match &entity.kind {
        Some(pb::entity::Kind::Team(value)) => value.supersedes.as_ref(),
        Some(pb::entity::Kind::Person(value)) => value.supersedes.as_ref(),
        _ => None,
    };
    if let Some(prior) = prior {
        if !crate::conv::entity_id(entity).is_ok_and(|current| {
            prior.namespace == current.namespace
                && prior.slug == current.slug
                && prior.version < current.version
        }) {
            issue("RESPONSIBILITY_VALUE_INVALID", "supersedes");
        }
    }
    let references = trogon_atlas_core::refs::try_outbound_refs(entity).unwrap_or_else(|_| {
        issue("RESPONSIBILITY_VALUE_INVALID", "metadata");
        Vec::new()
    });
    for reference in references {
        if !reference.to_kind.is_some_and(|kind| {
            reference.to_id.version > 0 && context.find(kind, &reference.to_id).is_some()
        }) {
            issue("RESPONSIBILITY_REFERENCE_UNRESOLVED", &reference.from_field);
        }
    }
    for (index, assignment) in assignments(entity).iter().enumerate() {
        if !assignment
            .subject
            .as_ref()
            .is_some_and(|subject| subject_resolves(subject, context))
        {
            issue(
                "RESPONSIBILITY_REFERENCE_UNRESOLVED",
                &format!("responsibilities[{index}].subject"),
            );
        }
    }
    issues
}

pub(crate) fn selected_endpoints(
    subject: &pb::ResponsibilitySubject,
    owner: &pb::Entity,
) -> Vec<(pb::EntityKind, pb::Id)> {
    let Some(pb::responsibility_subject::Selection::Connection(reference)) = &subject.selection
    else {
        return Vec::new();
    };
    let Ok(occurrence) = trogon_atlas_core::connections::resolve_connection(owner, reference)
    else {
        return Vec::new();
    };
    let path = occurrence
        .path
        .split_once('.')
        .map_or(occurrence.path.as_str(), |(_, path)| path);
    let mut prefixes = vec![path.to_owned()];
    match &owner.kind {
        Some(pb::entity::Kind::CommandSlice(_)) => {
            if path.starts_with("emitted_events[") {
                prefixes.push("command".into());
            } else if matches!(path, "command" | "persona") {
                prefixes.push("ui".into());
            } else if path == "ui" {
                prefixes.extend(["persona", "command"].map(str::to_owned));
            }
        }
        Some(pb::entity::Kind::ReadModelSlice(_)) if path.starts_with("source_events[") => {
            prefixes.push("read_model".into());
        }
        Some(pb::entity::Kind::AutomationSlice(_))
            if path.starts_with("source_read_models[") || path == "emitted_command" =>
        {
            prefixes.push("processor".into());
        }
        Some(pb::entity::Kind::UiSlice(_)) => {
            if path == "persona" || path.starts_with("source_read_models[") {
                prefixes.push("ui".into());
            } else if path == "ui" {
                prefixes.push("persona".into());
            }
        }
        Some(pb::entity::Kind::Swimlane(_)) => {
            if let Some((transition, _)) = path.split_once(".next[") {
                prefixes.push(format!("{transition}.after"));
            }
        }
        Some(pb::entity::Kind::Storyboard(_)) => {
            if let Some((entry, role)) = path.split_once(".human.") {
                let entry = entry.strip_suffix(".observer").unwrap_or(entry);
                prefixes.push(format!("{entry}.read_model"));
                match role {
                    "persona" => prefixes.push(format!("{entry}.observer.human.ui")),
                    "ui" => prefixes.push(format!("{entry}.observer.human.persona")),
                    _ => {}
                }
            } else if let Some((entry, _)) = path.split_once(".automation.") {
                let entry = entry.strip_suffix(".observer").unwrap_or(entry);
                prefixes.push(format!("{entry}.read_model"));
            }
        }
        _ => {}
    }
    let Ok(references) = trogon_atlas_core::refs::try_outbound_refs(owner) else {
        return Vec::new();
    };
    let mut targets: Vec<_> = references
        .into_iter()
        .filter(|reference| {
            prefixes.iter().any(|prefix| {
                reference.from_field == *prefix
                    || reference.from_field.starts_with(&format!("{prefix}."))
            })
        })
        .filter_map(|reference| Some((reference.to_kind?, reference.to_id)))
        .collect();
    if let Some(target) = occurrence.target {
        if let (Ok(kind), Some(id)) = (pb::EntityKind::try_from(target.kind), target.id) {
            targets.push((kind, id));
        }
    }
    if let Some(selection) = selected_message(subject, owner) {
        if let Some(id) = selection.specification.and_then(|reference| reference.id) {
            targets.push((pb::EntityKind::Specification, id));
        }
    }
    targets
}

pub(crate) fn selected_message(
    subject: &pb::ResponsibilitySubject,
    owner: &pb::Entity,
) -> Option<pb::ContractMemberRef> {
    let Some(pb::responsibility_subject::Selection::Connection(reference)) = &subject.selection
    else {
        return None;
    };
    trogon_atlas_core::communication_intent::message_selection(owner, reference).ok()
}
