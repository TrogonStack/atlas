use trogon_atlas_proto as pb;

use crate::validation::architecture_resources::ArchitectureContext;

pub(crate) fn validate_in_context(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Vec<pb::ValidationIssue> {
    let Some(pb::entity::Kind::DataProduct(product)) = &entity.kind else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    let mut issue = |code, field: &str| {
        issues.push(crate::validation::issue_error(
            code,
            "Data product declaration is invalid or its selection is absent, stale or inaccessible"
                .into(),
            crate::conv::entity_to_ref(entity).ok(),
            field,
        ));
    };
    if trogon_atlas_core::data_product::validate_values(product).is_err() {
        issue("DATA_PRODUCT_VALUE_INVALID", "data_product");
    }
    if let Some(prior) = &product.supersedes {
        if !product.id.as_ref().is_some_and(|id| {
            prior.namespace == id.namespace && prior.slug == id.slug && prior.version < id.version
        }) {
            issue("DATA_PRODUCT_VALUE_INVALID", "supersedes");
        }
    }
    for reference in trogon_atlas_core::refs::outbound_refs(entity) {
        if !reference.to_kind.is_some_and(|kind| {
            reference.to_id.version > 0 && context.find(kind, &reference.to_id).is_some()
        }) {
            issue("DATA_PRODUCT_REFERENCE_UNRESOLVED", &reference.from_field);
        }
    }
    for (index, claim) in product.storage_claims.iter().enumerate() {
        if !storage_claim_attachment_resolves(claim, context) {
            issue(
                "DATA_PRODUCT_REFERENCE_UNRESOLVED",
                &format!("storage_claims[{index}].attachment"),
            );
        }
    }
    for (index, output) in product.outputs.iter().enumerate() {
        if !contract_resolves(output.contract.as_ref(), context) {
            issue(
                "DATA_PRODUCT_REFERENCE_UNRESOLVED",
                &format!("outputs[{index}].contract"),
            );
        }
        for (access_index, access) in output.access.iter().enumerate() {
            if !access_resolves(
                access,
                output
                    .data
                    .as_ref()
                    .and_then(|data| data.read_model.as_ref())
                    .and_then(|reference| reference.id.as_ref()),
                context,
            ) {
                issue(
                    "DATA_PRODUCT_REFERENCE_UNRESOLVED",
                    &format!("outputs[{index}].access[{access_index}]"),
                );
            }
        }
    }
    issues
}

pub(crate) fn contract_resolves(
    contract: Option<&pb::OutputContract>,
    context: &ArchitectureContext<'_>,
) -> bool {
    match contract.and_then(|value| value.r#type.as_ref()) {
        None => contract.is_none(),
        Some(pb::output_contract::Type::Schema(reference)) => reference
            .id
            .as_ref()
            .is_some_and(|id| context.find(pb::EntityKind::Schema, id).is_some()),
        Some(pb::output_contract::Type::LibraryType(reference)) => reference
            .library
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .is_some_and(|id| context.find(pb::EntityKind::TypeLibrary, id).is_some()),
        Some(pb::output_contract::Type::ContractType(reference)) => reference
            .schema
            .as_ref()
            .and_then(|r| r.specification.as_ref())
            .and_then(|r| r.id.as_ref())
            .and_then(|id| context.find(pb::EntityKind::Specification, id))
            .is_some_and(|owner| {
                trogon_atlas_core::specification::resolve_type(owner, reference).is_ok()
            }),
    }
}

fn connection_owner<'a>(
    reference: &pb::ConnectionRef,
    context: &ArchitectureContext<'a>,
) -> Option<&'a pb::Entity> {
    let owner = reference.owner.as_ref()?;
    let kind = pb::EntityKind::try_from(owner.kind).ok()?;
    let entity = context.find(kind, owner.id.as_ref()?)?;
    trogon_atlas_core::connections::resolve_connection(entity, reference).ok()?;
    Some(entity)
}

pub(crate) fn storage_claim_attachment_resolves(
    claim: &pb::StorageAccessClaim,
    context: &ArchitectureContext<'_>,
) -> bool {
    let Some(reference) = &claim.attachment else {
        return true;
    };
    let Some(owner) = connection_owner(reference, context) else {
        return false;
    };
    let Some(pb::entity::Kind::DataStore(store)) = &owner.kind else {
        return false;
    };
    claim.store.as_ref().and_then(|r| r.id.as_ref()) == store.id.as_ref()
        && store
            .attachments
            .iter()
            .any(|edge| edge.connection_id == reference.id)
}

pub(crate) fn access_resolves(
    access: &pb::OutputAccess,
    output: Option<&pb::Id>,
    context: &ArchitectureContext<'_>,
) -> bool {
    let Some(output) = output else { return false };
    if context.find(pb::EntityKind::ReadModel, output).is_none() {
        return false;
    }
    match access.selection.as_ref() {
        Some(pb::output_access::Selection::Store(selection)) => {
            let Some(reference) = &selection.attachment else {
                return false;
            };
            let Some(owner) = connection_owner(reference, context) else {
                return false;
            };
            let Some(pb::entity::Kind::DataStore(store)) = &owner.kind else {
                return false;
            };
            selection.store.as_ref().and_then(|r| r.id.as_ref()) == store.id.as_ref()
                && store.attachments.iter().any(|edge| {
                    edge.connection_id == reference.id
                        && edge.read_model.as_ref().and_then(|r| r.id.as_ref()) == Some(output)
                })
        }
        Some(pb::output_access::Selection::Channel(selection)) => {
            let (Some(binding), Some(observation)) = (&selection.binding, &selection.observation)
            else {
                return false;
            };
            let Some(owner) = connection_owner(binding, context) else {
                return false;
            };
            let Some(pb::entity::Kind::Channel(channel)) = &owner.kind else {
                return false;
            };
            if selection.channel.as_ref().and_then(|r| r.id.as_ref()) != channel.id.as_ref()
                || !channel.bindings.iter().any(|edge| {
                    edge.connection_id == binding.id
                        && edge.observation.as_ref() == Some(observation)
                })
            {
                return false;
            }
            let Some(owner) = connection_owner(observation, context) else {
                return false;
            };
            let Some(pb::entity::Kind::AutomationSlice(slice)) = &owner.kind else {
                return false;
            };
            slice.source_read_models.iter().any(|edge| {
                edge.connection_id == observation.id
                    && edge.read_model.as_ref().and_then(|r| r.id.as_ref()) == Some(output)
            }) && slice.processor.as_ref().is_some_and(|edge| {
                edge.connection_id.is_some()
                    && edge
                        .processor
                        .as_ref()
                        .and_then(|r| r.id.as_ref())
                        .is_some_and(|id| context.find(pb::EntityKind::Processor, id).is_some())
            })
        }
        Some(pb::output_access::Selection::Operation(selection)) => {
            let (Some(operation), Some(response)) = (&selection.operation, &selection.response)
            else {
                return false;
            };
            let Some(id) = operation.specification.as_ref().and_then(|r| r.id.as_ref()) else {
                return false;
            };
            let Some(owner) = context.find(pb::EntityKind::Specification, id) else {
                return false;
            };
            let Ok(trogon_atlas_core::specification::ContractMember::Operation(selected)) =
                trogon_atlas_core::specification::resolve_member(owner, operation)
            else {
                return false;
            };
            if connection_owner(response, context) != Some(owner) {
                return false;
            }
            let Some(pb::entity::Kind::Specification(spec)) = &owner.kind else {
                return false;
            };
            let matched = selected.messages.iter().any(|edge| {
                edge.connection_id == response.id
                    && edge.role == pb::operation_message::Role::Response as i32
                    && spec.messages.iter().any(|message| {
                        message.member_id == edge.message_id
                            && message
                                .mapping
                                .as_ref()
                                .and_then(|mapping| mapping.read_model.as_ref())
                                .and_then(|r| r.id.as_ref())
                                == Some(output)
                    })
            });
            matched
                && selection
                    .provider_exposure
                    .as_ref()
                    .is_none_or(|reference| {
                        connection_owner(reference, context) == Some(owner)
                            && spec.exposures.iter().any(|edge| {
                                edge.connection_id == reference.id
                                    && edge.operation_id == selected.member_id
                                    && provider_exists(edge.provider.as_ref(), context)
                            })
                    })
        }
        None => false,
    }
}

fn provider_exists(
    provider: Option<&pb::ProviderTarget>,
    context: &ArchitectureContext<'_>,
) -> bool {
    match provider.and_then(|p| p.target.as_ref()) {
        Some(pb::provider_target::Target::Component(r)) => {
            r.id.as_ref()
                .is_some_and(|id| context.find(pb::EntityKind::Component, id).is_some())
        }
        Some(pb::provider_target::Target::ExternalSystem(r)) => {
            r.id.as_ref()
                .is_some_and(|id| context.find(pb::EntityKind::ExternalSystem, id).is_some())
        }
        None => false,
    }
}

pub(crate) fn library_types(
    product: &pb::DataProduct,
) -> impl Iterator<Item = &pb::TypeLibraryTypeRef> {
    product.outputs.iter().filter_map(|output| {
        match output.contract.as_ref().and_then(|c| c.r#type.as_ref()) {
            Some(pb::output_contract::Type::LibraryType(value)) => Some(value),
            _ => None,
        }
    })
}

pub(crate) fn selected_endpoints(
    resources: &[&pb::Entity],
    world: &std::collections::HashMap<pb::EntityKey, trogon_atlas_store::StoredEntity>,
) -> Vec<(pb::EntityKind, pb::Id)> {
    let context = ArchitectureContext::new(world.values());
    let mut endpoints = Vec::new();
    for resource in resources {
        let Some(pb::entity::Kind::DataProduct(product)) = &resource.kind else {
            continue;
        };
        for output in &product.outputs {
            for access in &output.access {
                match &access.selection {
                    Some(pb::output_access::Selection::Channel(value)) => {
                        if let Some(pb::entity::Kind::AutomationSlice(slice)) = value
                            .observation
                            .as_ref()
                            .and_then(|r| connection_owner(r, &context))
                            .and_then(|e| e.kind.as_ref())
                        {
                            if let Some(id) = slice
                                .processor
                                .as_ref()
                                .and_then(|e| e.processor.as_ref())
                                .and_then(|r| r.id.as_ref())
                            {
                                endpoints.push((pb::EntityKind::Processor, id.clone()));
                            }
                            for edge in &slice.source_read_models {
                                if edge.connection_id
                                    == value.observation.as_ref().and_then(|r| r.id.clone())
                                {
                                    if let Some(id) =
                                        edge.read_model.as_ref().and_then(|r| r.id.as_ref())
                                    {
                                        endpoints.push((pb::EntityKind::ReadModel, id.clone()));
                                    }
                                }
                            }
                        }
                    }
                    Some(pb::output_access::Selection::Operation(value)) => {
                        if let Some(pb::entity::Kind::Specification(spec)) = value
                            .operation
                            .as_ref()
                            .and_then(|r| r.specification.as_ref())
                            .and_then(|r| r.id.as_ref())
                            .and_then(|id| context.find(pb::EntityKind::Specification, id))
                            .and_then(|e| e.kind.as_ref())
                        {
                            for operation in &spec.operations {
                                for edge in &operation.messages {
                                    if edge.connection_id
                                        == value.response.as_ref().and_then(|r| r.id.clone())
                                    {
                                        for message in &spec.messages {
                                            if message.member_id == edge.message_id {
                                                if let Some(id) = message
                                                    .mapping
                                                    .as_ref()
                                                    .and_then(|m| m.read_model.as_ref())
                                                    .and_then(|r| r.id.as_ref())
                                                {
                                                    endpoints.push((
                                                        pb::EntityKind::ReadModel,
                                                        id.clone(),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            for exposure in &spec.exposures {
                                if exposure.connection_id
                                    == value.provider_exposure.as_ref().and_then(|r| r.id.clone())
                                {
                                    match exposure.provider.as_ref().and_then(|p| p.target.as_ref())
                                    {
                                        Some(pb::provider_target::Target::Component(r)) => {
                                            if let Some(id) = &r.id {
                                                endpoints
                                                    .push((pb::EntityKind::Component, id.clone()));
                                            }
                                        }
                                        Some(pb::provider_target::Target::ExternalSystem(r)) => {
                                            if let Some(id) = &r.id {
                                                endpoints.push((
                                                    pb::EntityKind::ExternalSystem,
                                                    id.clone(),
                                                ));
                                            }
                                        }
                                        None => {}
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    endpoints
}
