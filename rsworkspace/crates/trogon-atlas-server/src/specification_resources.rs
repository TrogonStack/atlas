use trogon_atlas_proto as pb;

use crate::validation::architecture_resources::ArchitectureContext;

pub(crate) fn validate_in_context(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Vec<pb::ValidationIssue> {
    let Some(pb::entity::Kind::Specification(spec)) = entity.kind.as_ref() else {
        return Vec::new();
    };
    let subject = crate::conv::entity_to_ref(entity).ok();
    let mut issues = Vec::new();
    if let Err(error) = trogon_atlas_core::specification::validate_specification(spec) {
        issues.push(crate::validation::issue_error(
            "SPECIFICATION_VALUE_INVALID",
            error.to_string(),
            subject.clone(),
            "specification",
        ));
    }
    if let Some(prior) = &spec.supersedes {
        if !spec.id.as_ref().is_some_and(|id| {
            prior.namespace == id.namespace && prior.slug == id.slug && prior.version < id.version
        }) {
            issues.push(crate::validation::issue_error(
                "SPECIFICATION_VALUE_INVALID",
                "supersedes must select an earlier version of the same Specification".into(),
                subject.clone(),
                "supersedes",
            ));
        }
    }
    let references = trogon_atlas_core::refs::try_outbound_refs(entity).unwrap_or_else(|_| {
        issues.push(crate::validation::issue_error(
            "SPECIFICATION_VALUE_INVALID",
            "Native metadata is malformed".into(),
            subject.clone(),
            "metadata",
        ));
        Vec::new()
    });
    for reference in references {
        let resolved = reference.to_kind.is_some_and(|kind| {
            reference.to_id.version > 0 && context.find(kind, &reference.to_id).is_some()
        });
        if !resolved {
            issues.push(crate::validation::issue_error(
                "SPECIFICATION_REFERENCE_UNRESOLVED",
                "Specification target is absent or invalid".into(),
                subject.clone(),
                &reference.from_field,
            ));
        }
    }
    for (i, message) in spec.messages.iter().enumerate() {
        for (j, binding) in message.schemas.iter().enumerate() {
            if let Some(pb::schema_binding::Type::ContractType(reference)) = &binding.r#type {
                let owner = reference
                    .schema
                    .as_ref()
                    .and_then(|value| value.specification.as_ref())
                    .and_then(|value| value.id.as_ref())
                    .and_then(|id| context.find(pb::EntityKind::Specification, id));
                if !owner.is_some_and(|owner| {
                    trogon_atlas_core::specification::resolve_type(owner, reference).is_ok()
                }) {
                    issues.push(crate::validation::issue_error(
                        "SPECIFICATION_REFERENCE_UNRESOLVED",
                        "Contract schema member, incarnation, artifact, digest or selector pin does not resolve".into(),
                        subject.clone(),
                        &format!("messages[{i}].schemas[{j}].contract_type"),
                    ));
                }
            }
        }
    }
    issues
}

pub(crate) fn library_types(entity: &pb::Entity) -> Vec<&pb::TypeLibraryTypeRef> {
    match &entity.kind {
        Some(pb::entity::Kind::Specification(spec)) => spec
            .messages
            .iter()
            .flat_map(|value| &value.schemas)
            .filter_map(|value| match &value.r#type {
                Some(pb::schema_binding::Type::LibraryType(value)) => Some(value),
                _ => None,
            })
            .collect(),
        Some(pb::entity::Kind::DataProduct(product)) => {
            crate::data_product_resources::library_types(product).collect()
        }
        _ => Vec::new(),
    }
}
