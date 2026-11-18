use trogon_atlas_proto as pb;

use crate::validation::architecture_resources::ArchitectureContext;

fn subject_resolves(subject: &pb::CatalogSubject, context: &ArchitectureContext<'_>) -> bool {
    let converted = crate::decision_document_resources::responsibility_subject(subject);
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
}

fn has_dedicated_check(field: &str) -> bool {
    field == "supersedes"
        || field.starts_with("subjects[")
        || (field.starts_with("steps[") && field.contains(".target.catalog_message"))
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
    if let Err(error) = trogon_atlas_core::design_topology::validate_values(entity) {
        issue("design_graph", &error.to_string());
    }
    let Some(graph) = trogon_atlas_core::design_topology::graph(entity) else {
        return issues;
    };
    for reference in trogon_atlas_core::refs::outbound_refs(entity) {
        if has_dedicated_check(&reference.from_field) {
            continue;
        }
        if !reference.to_kind.is_some_and(|kind| {
            reference.to_id.version > 0 && context.find(kind, &reference.to_id).is_some()
        }) {
            issue(
                &reference.from_field,
                "Design graph target is absent, stale or unavailable",
            );
        }
    }
    for (index, association) in graph.subjects.iter().enumerate() {
        if !association
            .subject
            .as_ref()
            .is_some_and(|subject| subject_resolves(subject, context))
        {
            issue(
                &format!("subjects[{index}].subject"),
                "Catalog subject is absent, stale or unavailable",
            );
        }
    }
    for (index, step) in graph.steps.iter().enumerate() {
        if let Some(pb::topology_step_target::Selection::CatalogMessage(subject)) =
            step.target.as_ref().and_then(|t| t.selection.as_ref())
        {
            if !subject_resolves(subject, context) {
                issue(
                    &format!("steps[{index}].target.catalog_message"),
                    "Design graph message target is absent, stale or unavailable",
                );
            }
        }
    }
    if let Some(prior) = &graph.supersedes {
        let current = graph.id.as_ref();
        if !current.is_some_and(|current| {
            current.namespace == prior.namespace
                && current.slug == prior.slug
                && prior.version > 0
                && prior.version < current.version
                && context.find(pb::EntityKind::DesignGraph, prior).is_some()
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
