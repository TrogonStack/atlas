use std::collections::{hash_map::Entry, HashMap, HashSet};

use tonic::Code;
use trogon_atlas_store::store::BranchLandOp;

use super::{
    entity_id, entity_kind, pb, store_err, validation, EventModelServiceImpl, Lens, OpMetaLike,
    Status, StoredEntity, Visibility,
};

pub(super) struct ArchitectureProposal<'a> {
    pub(super) overlay: HashMap<pb::EntityKey, Option<&'a pb::Entity>>,
    pub(super) landing: Option<&'a [BranchLandOp]>,
}

#[derive(Clone, Copy)]
enum ArchitectureTargetAccess<'a> {
    Required(&'a Visibility),
    Diagnostic(&'a Lens),
    ExactRequired(&'a Visibility),
    ExactDiagnostic(&'a Lens),
}

fn admission_references(entity: &pb::Entity) -> Vec<trogon_atlas_core::refs::OutboundRef> {
    let architectural = matches!(
        entity.kind,
        Some(pb::entity::Kind::Component(_) | pb::entity::Kind::ExternalSystem(_))
    );
    trogon_atlas_core::refs::outbound_refs(entity)
        .into_iter()
        .filter(|reference| {
            !architectural
                || reference.from_field.starts_with("relationships[")
                || reference.from_field.starts_with("communications[")
        })
        .collect()
}

fn is_catalog(entity: &pb::Entity) -> bool {
    matches!(
        entity.kind,
        Some(pb::entity::Kind::Decision(_) | pb::entity::Kind::Document(_))
    )
}

impl EventModelServiceImpl {
    pub(super) async fn validate_architecture_selection(
        &self,
        entity: &pb::Entity,
        base: &pb::Entity,
        branch: &str,
        vis: &Visibility,
    ) -> Result<(), Status> {
        let kind = entity_kind(entity)?;
        let id = entity_id(entity)?.clone();
        let ops = [BranchLandOp::Put {
            kind,
            ours: Box::new(entity.clone()),
            base: Some(Box::new(base.clone())),
            expected_baseline_etag: None,
        }];
        self.validate_architecture_proposal(
            &[OpMetaLike {
                kind,
                id,
                is_put: true,
                entity: Some(entity.clone()),
            }],
            Some(branch),
            vis,
            Some(&ops),
        )
        .await
    }

    pub(super) async fn validate_architecture_writes(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        self.validate_architecture_proposal(metas, branch, vis, None)
            .await
    }

    pub(super) async fn validate_architecture_landing(
        &self,
        metas: &[OpMetaLike],
        ops: &[BranchLandOp],
        vis: &Visibility,
    ) -> Result<(), Status> {
        self.validate_architecture_proposal(metas, None, vis, Some(ops))
            .await
    }

    async fn validate_architecture_proposal(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
        landing: Option<&[BranchLandOp]>,
    ) -> Result<(), Status> {
        let resources: Vec<_> = metas
            .iter()
            .filter(|meta| {
                meta.is_put
                    && matches!(
                        meta.kind,
                        pb::EntityKind::Component
                            | pb::EntityKind::ExternalSystem
                            | pb::EntityKind::Channel
                            | pb::EntityKind::DataStore
                            | pb::EntityKind::Specification
                            | pb::EntityKind::DataProduct
                            | pb::EntityKind::Team
                            | pb::EntityKind::Person
                            | pb::EntityKind::Decision
                            | pb::EntityKind::Document
                    )
            })
            .filter_map(|meta| meta.entity.as_ref())
            .collect();
        if resources.is_empty() {
            return Ok(());
        }
        let mut catalog_budget = crate::decision_document_resources::CatalogBudget::default();
        if resources.iter().any(|entity| is_catalog(entity)) {
            for entity in &resources {
                catalog_budget.owner(entity)?;
            }
        }
        let overlay: HashMap<_, _> = metas
            .iter()
            .map(|meta| {
                (
                    pb::EntityKey::new(meta.kind, &meta.id),
                    meta.entity.as_ref().filter(|_| meta.is_put),
                )
            })
            .collect();
        let proposal = ArchitectureProposal { overlay, landing };
        let mut world = HashMap::new();
        let mut required = Vec::new();
        for entity in &resources {
            if is_catalog(entity) {
                continue;
            }
            required.extend(
                admission_references(entity)
                    .into_iter()
                    .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id))),
            );
        }
        let mut loaded = HashSet::new();
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::Required(vis),
            required,
            &mut loaded,
            &mut world,
        )
        .await?;
        let catalog_targets = resources
            .iter()
            .filter(|entity| is_catalog(entity))
            .flat_map(|entity| trogon_atlas_core::refs::outbound_refs(entity))
            .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id)))
            .collect();
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::ExactRequired(vis),
            catalog_targets,
            &mut loaded,
            &mut world,
        )
        .await?;
        let mut endpoints = observation_endpoints(&resources, &world);
        endpoints.extend(crate::data_product_resources::selected_endpoints(
            &resources, &world,
        ));
        endpoints.extend(responsibility_endpoints(&resources, &world));
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::Required(vis),
            endpoints,
            &mut loaded,
            &mut world,
        )
        .await?;
        let catalog_targets = catalog_endpoints(&resources, &world);
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::ExactRequired(vis),
            catalog_targets,
            &mut loaded,
            &mut world,
        )
        .await?;
        self.validate_catalog_replacements(
            &proposal,
            branch,
            vis,
            &mut loaded,
            &mut world,
            &mut catalog_budget,
        )
        .await?;
        let context: Vec<_> = world.into_values().collect();
        let event_products: Vec<_> = resources
            .iter()
            .filter_map(|entity| match &entity.kind {
                Some(pb::entity::Kind::DataProduct(product))
                    if super::data_product_events::inputs(product).next().is_some() =>
                {
                    Some(product)
                }
                _ => None,
            })
            .collect();
        let mut confirmations = super::data_product_events::EventConfirmation::default();
        if !event_products.is_empty() {
            let (rows, complete) = self
                .data_product_emission_rows(branch, vis, &proposal)
                .await?;
            for product in event_products {
                for (_, id) in super::data_product_events::inputs(product) {
                    if confirmations.confirm(id, &rows, complete)?.is_empty() {
                        return Err(Status::invalid_argument(
                            "Data product Event input has no authorized local emission",
                        ));
                    }
                }
            }
        }
        let mut compiled = HashSet::new();
        for resource in resources {
            let kind = entity_kind(resource)?;
            let id = entity_id(resource)?;
            let previous = match self.store.get(kind, id, branch).await {
                Ok(stored) => Some(stored.entity),
                Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                Err(error) => return Err(store_err(error)),
            };
            let prepared =
                prepare_proposal_entity(&proposal, kind, id, previous.as_ref(), resource)?;
            super::validate_entity_content(&prepared)?;
            let issues = validation::architecture_resources::validate_with_previous(
                &prepared,
                &context,
                previous.as_ref(),
            );
            if !issues.is_empty() {
                return Err(Status::invalid_argument(super::format_scenario_issues(
                    &issues,
                )));
            }
            for selection in crate::specification_resources::library_types(&prepared) {
                if let (Some(id), Some(message)) = (
                    selection.library.as_ref().and_then(|v| v.id.as_ref()),
                    selection.message.as_ref(),
                ) {
                    if compiled.insert((
                        pb::EntityKey::new(pb::EntityKind::TypeLibrary, id),
                        message.value.clone(),
                    )) {
                        self.type_libraries
                            .check_contract_type(id, &message.value, &context)
                            .await.map_err(|status| {
                                if matches!(prepared.kind, Some(pb::entity::Kind::DataProduct(_)))
                                    && matches!(status.code(), Code::InvalidArgument | Code::FailedPrecondition)
                                {
                                    Status::invalid_argument("Selected data product protobuf type is unresolved or invalid")
                                } else { status }
                            })?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) async fn architecture_validation_context(
        &self,
        snapshot: &[StoredEntity],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Vec<StoredEntity>, Status> {
        let resources: Vec<_> = snapshot
            .iter()
            .map(|stored| &stored.entity)
            .filter(|entity| {
                matches!(
                    entity.kind,
                    Some(
                        pb::entity::Kind::Component(_)
                            | pb::entity::Kind::ExternalSystem(_)
                            | pb::entity::Kind::Channel(_)
                            | pb::entity::Kind::DataStore(_)
                            | pb::entity::Kind::Specification(_)
                            | pb::entity::Kind::DataProduct(_)
                            | pb::entity::Kind::Team(_)
                            | pb::entity::Kind::Person(_)
                            | pb::entity::Kind::Decision(_)
                            | pb::entity::Kind::Document(_)
                    )
                )
            })
            .collect();
        if resources.is_empty() {
            return Ok(Vec::new());
        }
        let proposal = ArchitectureProposal {
            overlay: HashMap::new(),
            landing: None,
        };
        let lens = self.lens(vis).await?;
        let access = ArchitectureTargetAccess::Diagnostic(&lens);
        let required = resources
            .iter()
            .filter(|entity| !is_catalog(entity))
            .flat_map(|entity| {
                admission_references(entity)
                    .into_iter()
                    .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id)))
            })
            .collect();
        let mut loaded = HashSet::new();
        let mut world = HashMap::new();
        self.load_architecture_targets(
            &proposal,
            branch,
            access,
            required,
            &mut loaded,
            &mut world,
        )
        .await?;
        let catalog_targets = resources
            .iter()
            .filter(|entity| is_catalog(entity))
            .flat_map(|entity| trogon_atlas_core::refs::outbound_refs(entity))
            .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id)))
            .collect();
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::ExactDiagnostic(&lens),
            catalog_targets,
            &mut loaded,
            &mut world,
        )
        .await?;
        let mut endpoints = observation_endpoints(&resources, &world);
        endpoints.extend(crate::data_product_resources::selected_endpoints(
            &resources, &world,
        ));
        endpoints.extend(responsibility_endpoints(&resources, &world));
        self.load_architecture_targets(
            &proposal,
            branch,
            access,
            endpoints,
            &mut loaded,
            &mut world,
        )
        .await?;
        let catalog_targets = catalog_endpoints(&resources, &world);
        self.load_architecture_targets(
            &proposal,
            branch,
            ArchitectureTargetAccess::ExactDiagnostic(&lens),
            catalog_targets,
            &mut loaded,
            &mut world,
        )
        .await?;
        let mut budget = crate::decision_document_resources::CatalogBudget::default();
        for entity in &resources {
            let Some(pb::entity::Kind::Decision(decision)) = &entity.kind else {
                continue;
            };
            let mut next = decision
                .replaced_by
                .as_ref()
                .and_then(|replacement| replacement.decision.as_ref())
                .and_then(|reference| reference.id.clone());
            let mut visited = HashSet::new();
            visited.insert(entity_id(entity)?.clone());
            while let Some(id) = next.take() {
                budget.examine(1)?;
                if !visited.insert(id.clone()) {
                    break;
                }
                self.load_architecture_targets(
                    &proposal,
                    branch,
                    ArchitectureTargetAccess::ExactDiagnostic(&lens),
                    vec![(pb::EntityKind::Decision, id.clone())],
                    &mut loaded,
                    &mut world,
                )
                .await?;
                let Some(row) = world.get(&pb::EntityKey::new(pb::EntityKind::Decision, &id))
                else {
                    break;
                };
                budget.owner(&row.entity)?;
                let Some(pb::entity::Kind::Decision(target)) = &row.entity.kind else {
                    break;
                };
                if target.status != pb::DecisionStatus::Superseded as i32 {
                    break;
                }
                next = target
                    .replaced_by
                    .as_ref()
                    .and_then(|replacement| replacement.decision.as_ref())
                    .and_then(|reference| reference.id.clone());
            }
        }
        Ok(world.into_values().collect())
    }

    pub(super) async fn specification_type_diagnostics(
        &self,
        snapshot: &[StoredEntity],
        context: &[StoredEntity],
    ) -> Result<Vec<pb::ValidationIssue>, Status> {
        let mut issues = Vec::new();
        let mut checked = HashMap::new();
        for stored in snapshot {
            for selection in crate::specification_resources::library_types(&stored.entity) {
                let (Some(id), Some(message)) = (
                    selection.library.as_ref().and_then(|v| v.id.as_ref()),
                    selection.message.as_ref(),
                ) else {
                    continue;
                };
                let key = (
                    pb::EntityKey::new(pb::EntityKind::TypeLibrary, id),
                    message.value.clone(),
                );
                let result = match checked.entry(key) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => entry.insert(
                        self.type_libraries
                            .check_contract_type(id, &message.value, context)
                            .await,
                    ),
                };
                if let Err(status) = result {
                    if !matches!(
                        status.code(),
                        Code::InvalidArgument | Code::FailedPrecondition
                    ) {
                        return Err(status.clone());
                    }
                    issues.push(validation::issue_error(
                        if matches!(stored.entity.kind, Some(pb::entity::Kind::DataProduct(_))) { "DATA_PRODUCT_REFERENCE_UNRESOLVED" } else { "SPECIFICATION_REFERENCE_UNRESOLVED" },
                        "Selected protobuf message is absent, unreadable or does not compile in its exact library".into(),
                        crate::conv::entity_to_ref(&stored.entity).ok(),
                        if matches!(stored.entity.kind, Some(pb::entity::Kind::DataProduct(_))) { "outputs.contract.library_type" } else { "messages.schemas.library_type" },
                    ));
                }
            }
        }
        Ok(issues)
    }

    pub(super) async fn architecture_reference_snapshot(
        &self,
        snapshot: &[StoredEntity],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Vec<StoredEntity>, Status> {
        let lens = self.lens(vis).await?;
        let namespaces: Vec<String> = match &lens {
            super::Lens::Everything => Vec::new(),
            super::Lens::Only(namespaces) if namespaces.is_empty() => return Ok(snapshot.to_vec()),
            super::Lens::Only(namespaces) => {
                namespaces.iter().map(|id| id.as_str().to_owned()).collect()
            }
        };
        let resources = self
            .store
            .list(
                super::ListFilter {
                    kinds: &[
                        pb::EntityKind::Component,
                        pb::EntityKind::ExternalSystem,
                        pb::EntityKind::Channel,
                        pb::EntityKind::DataStore,
                        pb::EntityKind::Specification,
                        pb::EntityKind::DataProduct,
                        pb::EntityKind::Team,
                        pb::EntityKind::Person,
                        pb::EntityKind::Decision,
                        pb::EntityKind::Document,
                    ],
                    namespaces: &namespaces,
                    ..Default::default()
                },
                Some(super::MAX_PROJECTION_ENTITIES + 1),
                branch,
            )
            .await
            .map_err(store_err)?;
        if resources.len() > super::MAX_PROJECTION_ENTITIES {
            return Err(Status::resource_exhausted(
                "architecture reference scan exceeds its entity budget",
            ));
        }
        let mut complete: HashMap<_, _> = snapshot
            .iter()
            .filter_map(|stored| {
                Some((
                    pb::EntityKey::new(
                        entity_kind(&stored.entity).ok()?,
                        entity_id(&stored.entity).ok()?,
                    ),
                    stored.clone(),
                ))
            })
            .collect();
        for stored in resources {
            let id = entity_id(&stored.entity)?;
            if lens.admits(&id.namespace) {
                complete.insert(pb::EntityKey::new(entity_kind(&stored.entity)?, id), stored);
            }
        }
        Ok(complete.into_values().collect())
    }

    async fn load_architecture_targets(
        &self,
        proposal: &ArchitectureProposal<'_>,
        branch: Option<&str>,
        access: ArchitectureTargetAccess<'_>,
        targets: Vec<(pb::EntityKind, pb::Id)>,
        loaded: &mut HashSet<pb::EntityKey>,
        world: &mut HashMap<pb::EntityKey, StoredEntity>,
    ) -> Result<(), Status> {
        let mut pending = targets;
        while let Some((kind, id)) = pending.pop() {
            match access {
                ArchitectureTargetAccess::Required(vis)
                | ArchitectureTargetAccess::ExactRequired(vis) => {
                    match self.require_visible(vis, &id.namespace).await {
                        Ok(()) => {}
                        Err(status) if status.code() == Code::NotFound => continue,
                        Err(status) => return Err(status),
                    }
                }
                ArchitectureTargetAccess::Diagnostic(lens)
                | ArchitectureTargetAccess::ExactDiagnostic(lens) => {
                    if !lens.admits(&id.namespace)
                        || id.version == 0
                        || super::require_id(Some(&id), "target").is_err()
                    {
                        continue;
                    }
                }
            }
            let key = pb::EntityKey::new(kind, &id);
            if !loaded.insert(key.clone()) {
                continue;
            }
            let stored = match self.store.get(kind, &id, branch).await {
                Ok(stored) => Some(stored),
                Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                Err(error) => return Err(store_err(error)),
            };
            let next = match proposal.overlay.get(&key) {
                Some(Some(entity)) => Some(StoredEntity {
                    entity: prepare_proposal_entity(
                        proposal,
                        kind,
                        &id,
                        stored.as_ref().map(|stored| &stored.entity),
                        entity,
                    )?,
                    etag: String::new(),
                }),
                Some(None) => None,
                None => stored,
            };
            if let Some(stored) = next {
                if let Some(pb::entity::Kind::TypeLibrary(library)) = &stored.entity.kind {
                    if matches!(
                        access,
                        ArchitectureTargetAccess::ExactRequired(_)
                            | ArchitectureTargetAccess::ExactDiagnostic(_)
                    ) {
                        world.insert(key, stored);
                        continue;
                    }
                    for dependency in &library.dependencies {
                        if let Some(target) = &dependency.id {
                            if target.namespace != id.namespace {
                                continue;
                            }
                            pending.push((pb::EntityKind::TypeLibrary, target.clone()));
                        }
                    }
                }
                world.insert(key, stored);
            }
        }
        Ok(())
    }

    async fn validate_catalog_replacements(
        &self,
        proposal: &ArchitectureProposal<'_>,
        branch: Option<&str>,
        vis: &Visibility,
        loaded: &mut HashSet<pb::EntityKey>,
        world: &mut HashMap<pb::EntityKey, StoredEntity>,
        budget: &mut crate::decision_document_resources::CatalogBudget,
    ) -> Result<(), Status> {
        let resources: Vec<_> = proposal
            .overlay
            .values()
            .filter_map(|entity| *entity)
            .filter(|entity| is_catalog(entity))
            .collect();
        if resources.is_empty() {
            return Ok(());
        }
        for row in world.values() {
            budget.owner(&row.entity)?;
        }
        for entity in &resources {
            for association in crate::decision_document_resources::subjects(entity) {
                if let Some(subject) = &association.subject {
                    if let Ok(reference) =
                        trogon_atlas_core::decision_document::subject_owner(subject)
                    {
                        if let (Ok(kind), Some(id)) = (
                            pb::EntityKind::try_from(reference.kind),
                            reference.id.as_ref(),
                        ) {
                            if let Some(owner) = world.get(&pb::EntityKey::new(kind, id)) {
                                budget.selection(subject, &owner.entity)?;
                            }
                        }
                    }
                }
            }
            let Some(pb::entity::Kind::Decision(decision)) = &entity.kind else {
                continue;
            };
            let Some(replacement) = &decision.replaced_by else {
                continue;
            };
            let mut next = replacement
                .decision
                .as_ref()
                .and_then(|reference| reference.id.clone());
            let mut visited = HashSet::new();
            visited.insert(pb::EntityKey::new(
                pb::EntityKind::Decision,
                entity_id(entity)?,
            ));
            loop {
                budget.examine(1)?;
                let id = next.take().ok_or_else(|| {
                    Status::invalid_argument("Replacement chain is absent, stale or unavailable")
                })?;
                let key = pb::EntityKey::new(pb::EntityKind::Decision, &id);
                if !visited.insert(key.clone()) {
                    return Err(Status::invalid_argument("Decision replacement cycle"));
                }
                self.load_architecture_targets(
                    proposal,
                    branch,
                    ArchitectureTargetAccess::ExactRequired(vis),
                    vec![(pb::EntityKind::Decision, id)],
                    loaded,
                    world,
                )
                .await?;
                let Some(row) = world.get(&key) else {
                    return Err(Status::invalid_argument(
                        "Replacement chain is absent, stale or unavailable",
                    ));
                };
                budget.owner(&row.entity)?;
                let Some(pb::entity::Kind::Decision(target)) = &row.entity.kind else {
                    return Err(Status::invalid_argument(
                        "Replacement target must be a Decision",
                    ));
                };
                match pb::DecisionStatus::try_from(target.status) {
                    Ok(pb::DecisionStatus::Accepted) => break,
                    Ok(pb::DecisionStatus::Superseded) => {
                        next = target
                            .replaced_by
                            .as_ref()
                            .and_then(|value| value.decision.as_ref())
                            .and_then(|reference| reference.id.clone());
                    }
                    _ => {
                        return Err(Status::invalid_argument(
                            "Replacement chain must terminate at an ACCEPTED decision",
                        ))
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) async fn data_product_emission_rows(
        &self,
        branch: Option<&str>,
        vis: &Visibility,
        proposal: &ArchitectureProposal<'_>,
    ) -> Result<(Vec<StoredEntity>, bool), Status> {
        let lens = self.lens(vis).await?;
        let namespaces: Vec<_> = match &lens {
            Lens::Everything => Vec::new(),
            Lens::Only(namespaces) if namespaces.is_empty() => return Ok((Vec::new(), true)),
            Lens::Only(namespaces) => namespaces.iter().map(|id| id.as_str().to_owned()).collect(),
        };
        let limit = 5_001 + proposal.overlay.len();
        let rows = self
            .store
            .list(
                super::ListFilter {
                    kinds: &[pb::EntityKind::CommandSlice],
                    namespaces: &namespaces,
                    ..Default::default()
                },
                Some(limit),
                branch,
            )
            .await
            .map_err(store_err)?;
        let complete = rows.len() < limit;
        let mut overlay = Vec::new();
        for (key, entity) in &proposal.overlay {
            if key.kind != pb::EntityKind::CommandSlice {
                continue;
            }
            let next = if let Some(entity) = entity {
                let previous = match self.store.get(key.kind, &key.id(), branch).await {
                    Ok(stored) => Some(stored.entity),
                    Err(trogon_atlas_store::error::StoreError::NotFound) => None,
                    Err(error) => return Err(store_err(error)),
                };
                Some(StoredEntity {
                    entity: prepare_proposal_entity(
                        proposal,
                        key.kind,
                        &key.id(),
                        previous.as_ref(),
                        entity,
                    )?,
                    etag: String::new(),
                })
            } else {
                None
            };
            overlay.push((key.clone(), next));
        }
        let rows = super::data_product_events::merge_rows(rows, overlay)
            .into_iter()
            .filter(|stored| {
                super::entity_id(&stored.entity).is_ok_and(|id| lens.admits(&id.namespace))
            })
            .collect();
        Ok((rows, complete))
    }

    pub(super) async fn data_product_event_diagnostics(
        &self,
        snapshot: &[StoredEntity],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<Vec<pb::ValidationIssue>, Status> {
        if !snapshot.iter().any(|stored| matches!(&stored.entity.kind, Some(pb::entity::Kind::DataProduct(product)) if super::data_product_events::inputs(product).next().is_some())) { return Ok(Vec::new()) }
        let proposal = ArchitectureProposal {
            overlay: HashMap::new(),
            landing: None,
        };
        let (rows, complete) = self
            .data_product_emission_rows(branch, vis, &proposal)
            .await?;
        let mut budget = super::data_product_events::EventConfirmation::default();
        let mut issues = Vec::new();
        for stored in snapshot {
            let Some(pb::entity::Kind::DataProduct(product)) = &stored.entity.kind else {
                continue;
            };
            for (index, id) in super::data_product_events::inputs(product) {
                let code = match budget.confirm(id, &rows, complete) {
                    Ok(witnesses) if !witnesses.is_empty() => continue,
                    Ok(_) => "DATA_PRODUCT_REFERENCE_UNRESOLVED",
                    Err(status) if status.code() == Code::ResourceExhausted => {
                        "DATA_PRODUCT_CONFIRMATION_INCOMPLETE"
                    }
                    Err(status) => return Err(status),
                };
                issues.push(validation::issue_error(
                    code,
                    "Authorized local Event confirmation is absent or incomplete".into(),
                    crate::conv::entity_to_ref(&stored.entity).ok(),
                    &format!("inputs[{index}].data"),
                ));
            }
        }
        Ok(issues)
    }

    pub(super) async fn validate_architecture_write(
        &self,
        entity: &pb::Entity,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        if !matches!(
            entity.kind,
            Some(
                pb::entity::Kind::Component(_)
                    | pb::entity::Kind::ExternalSystem(_)
                    | pb::entity::Kind::Channel(_)
                    | pb::entity::Kind::DataStore(_)
                    | pb::entity::Kind::Specification(_)
                    | pb::entity::Kind::DataProduct(_)
                    | pb::entity::Kind::Team(_)
                    | pb::entity::Kind::Person(_)
                    | pb::entity::Kind::Decision(_)
                    | pb::entity::Kind::Document(_)
            )
        ) {
            return Ok(());
        }
        self.validate_architecture_writes(
            &[OpMetaLike {
                kind: entity_kind(entity)?,
                id: entity_id(entity)?.clone(),
                is_put: true,
                entity: Some(entity.clone()),
            }],
            branch,
            vis,
        )
        .await
    }
}

fn prepare_proposal_entity(
    proposal: &ArchitectureProposal<'_>,
    kind: pb::EntityKind,
    id: &pb::Id,
    previous: Option<&pb::Entity>,
    entity: &pb::Entity,
) -> Result<pb::Entity, Status> {
    match proposal.landing.and_then(|ops| {
        ops.iter().find_map(|op| match op {
            BranchLandOp::Put {
                kind: op_kind,
                ours,
                base,
                ..
            } if *op_kind == kind && entity_id(ours).ok() == Some(id) => Some(base.as_deref()),
            _ => None,
        })
    }) {
        Some(base) => {
            trogon_atlas_core::connection_persistence::prepare_landing(base, previous, entity)
        }
        None => trogon_atlas_core::connection_persistence::prepare_entity(previous, entity),
    }
    .map_err(|error| Status::invalid_argument(error.to_string()))
}

fn observation_endpoints(
    resources: &[&pb::Entity],
    world: &HashMap<pb::EntityKey, StoredEntity>,
) -> Vec<(pb::EntityKind, pb::Id)> {
    let mut endpoints = Vec::new();
    for resource in resources {
        let Some(pb::entity::Kind::Channel(channel)) = resource.kind.as_ref() else {
            continue;
        };
        for binding in &channel.bindings {
            let Some(observation) = &binding.observation else {
                continue;
            };
            let Some(owner) = &observation.owner else {
                continue;
            };
            let Some(id) = &owner.id else {
                continue;
            };
            if owner.kind != pb::EntityKind::AutomationSlice as i32 {
                continue;
            }
            let Some(stored) = world.get(&pb::EntityKey::new(pb::EntityKind::AutomationSlice, id))
            else {
                continue;
            };
            let Some(pb::entity::Kind::AutomationSlice(slice)) = stored.entity.kind.as_ref() else {
                continue;
            };
            if let Some(id) = slice
                .source_read_models
                .iter()
                .find(|edge| edge.connection_id.is_some() && edge.connection_id == observation.id)
                .and_then(|edge| edge.read_model.as_ref())
                .and_then(|reference| reference.id.as_ref())
            {
                endpoints.push((pb::EntityKind::ReadModel, id.clone()));
            }
            if let Some(id) = slice
                .processor
                .as_ref()
                .and_then(|edge| edge.processor.as_ref())
                .and_then(|reference| reference.id.as_ref())
            {
                endpoints.push((pb::EntityKind::Processor, id.clone()));
            }
        }
    }
    endpoints
}

fn responsibility_endpoints(
    resources: &[&pb::Entity],
    world: &HashMap<pb::EntityKey, StoredEntity>,
) -> Vec<(pb::EntityKind, pb::Id)> {
    resources
        .iter()
        .flat_map(|entity| crate::responsibility_resources::assignments(entity))
        .filter_map(|assignment| assignment.subject.as_ref())
        .flat_map(|subject| {
            crate::responsibility_resources::subject_owner(subject)
                .and_then(|reference| {
                    Some(pb::EntityKey::new(
                        pb::EntityKind::try_from(reference.kind).ok()?,
                        reference.id.as_ref()?,
                    ))
                })
                .and_then(|key| world.get(&key))
                .map(|stored| {
                    crate::responsibility_resources::selected_endpoints(subject, &stored.entity)
                })
                .unwrap_or_default()
        })
        .collect()
}

fn catalog_endpoints(
    resources: &[&pb::Entity],
    world: &HashMap<pb::EntityKey, StoredEntity>,
) -> Vec<(pb::EntityKind, pb::Id)> {
    resources
        .iter()
        .flat_map(|entity| crate::decision_document_resources::subjects(entity))
        .filter_map(|association| association.subject.as_ref())
        .flat_map(|subject| {
            let converted = crate::decision_document_resources::responsibility_subject(subject);
            crate::responsibility_resources::subject_owner(&converted)
                .and_then(|reference| {
                    Some(pb::EntityKey::new(
                        pb::EntityKind::try_from(reference.kind).ok()?,
                        reference.id.as_ref()?,
                    ))
                })
                .and_then(|key| world.get(&key))
                .map(|stored| {
                    crate::responsibility_resources::selected_endpoints(&converted, &stored.entity)
                })
                .unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod specification_validation_tests {
    use std::{sync::Arc, time::Duration};

    use tonic::{Code, Request};
    use tonic_types::StatusExt;
    use trogon_atlas_proto::event_model_service_server::EventModelService;
    use trogon_atlas_store::WriteContext;
    use trogon_atlas_types::CompileRunner;

    use super::*;
    use crate::{metrics::ServerMetrics, type_libraries::TypeLibraryGate};

    fn library_id(namespace: &str, version: u64) -> pb::Id {
        pb::Id {
            namespace: namespace.into(),
            slug: "orders".into(),
            version,
        }
    }

    async fn fixture() -> EventModelServiceImpl {
        let nats = trogon_atlas_testsupport::shared().await;
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap()
    }

    async fn library(
        service: &EventModelServiceImpl,
        id: pb::Id,
        source: &str,
        dependencies: Vec<pb::TypeLibraryRef>,
    ) {
        service
            .store
            .put(
                pb::EntityKind::TypeLibrary,
                &pb::Entity {
                    kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
                        id: Some(id),
                        title: "Orders".into(),
                        files: vec![pb::ProtoSourceFile {
                            path: "orders.proto".into(),
                            content: source.into(),
                        }],
                        dependencies,
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                false,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    const VALID: &str = "syntax = \"proto3\"; package orders.v1; message Order {}";

    async fn selection(service: &EventModelServiceImpl, slug: &str, id: pb::Id, message: &str) {
        service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(pb::Entity {
                    kind: Some(pb::entity::Kind::Specification(pb::Specification {
                        id: Some(pb::Id {
                            namespace: "contracts".into(),
                            slug: slug.into(),
                            version: 1,
                        }),
                        title: slug.into(),
                        messages: vec![pb::ContractMessage {
                            name: Some(pb::MemberName {
                                value: "Payload".into(),
                            }),
                            schemas: vec![pb::SchemaBinding {
                                role: pb::schema_binding::Role::Payload as i32,
                                r#type: Some(pb::schema_binding::Type::LibraryType(
                                    pb::TypeLibraryTypeRef {
                                        library: Some(pb::TypeLibraryRef { id: Some(id) }),
                                        message: Some(pb::QualifiedProtoName {
                                            value: message.into(),
                                        }),
                                    },
                                )),
                                ..Default::default()
                            }],
                            ..Default::default()
                        }],
                        ..Default::default()
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .await
            .unwrap();
    }

    async fn validate(
        service: &EventModelServiceImpl,
    ) -> Result<tonic::Response<pb::ValidateProjectResponse>, Status> {
        service
            .validate_project(Request::new(pb::ValidateProjectRequest {
                scope: Some(pb::validate_project_request::Scope::Namespace(
                    "contracts".into(),
                )),
            }))
            .await
    }

    fn issues(report: &pb::ValidateProjectResponse) -> Vec<&pb::ValidationIssue> {
        report
            .reports
            .iter()
            .flat_map(|r| &r.issues)
            .filter(|i| {
                i.code == "SPECIFICATION_REFERENCE_UNRESOLVED"
                    && i.subject_field == "messages.schemas.library_type"
            })
            .collect()
    }

    #[tokio::test]
    async fn validate_project_propagates_compile_timeout() {
        let mut service = fixture().await;
        library(&service, library_id("types", 1), VALID, vec![]).await;
        selection(&service, "typed", library_id("types", 1), "orders.v1.Order").await;
        service.type_libraries =
            TypeLibraryGate::with_compile_runner(CompileRunner::new(0, Duration::from_millis(1)));
        let error = validate(&service)
            .await
            .expect_err("compiler timeout must fail the RPC, not diagnose the model");
        assert_eq!(error.code(), Code::DeadlineExceeded);
        assert_eq!(
            error.get_error_details().error_info().unwrap().reason,
            crate::conv::reason::UNAVAILABLE
        );
        for private in ["types", "orders", "Order", "orders.proto"] {
            assert!(!format!("{error:?}").contains(private));
        }
    }

    #[tokio::test]
    async fn validate_project_memoizes_exact_selections_only_within_each_call() {
        let service = fixture().await;
        for (namespace, version) in [("types", 1), ("types", 2), ("other-types", 1)] {
            library(&service, library_id(namespace, version), VALID, vec![]).await;
        }
        for (slug, namespace, version) in [
            ("first", "types", 1),
            ("duplicate", "types", 1),
            ("version", "types", 2),
            ("namespace", "other-types", 1),
        ] {
            selection(
                &service,
                slug,
                library_id(namespace, version),
                "orders.v1.Order",
            )
            .await;
        }
        let before = service.type_libraries.contract_compile_calls();
        let report = validate(&service).await.unwrap().into_inner();
        assert!(issues(&report).is_empty(), "{report:?}");
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 3);
        library(
            &service,
            library_id("types", 1),
            "syntax = \"proto3\"; package orders.v1; message Replacement {}",
            vec![],
        )
        .await;
        let report = validate(&service).await.unwrap().into_inner();
        let failures = issues(&report);
        assert_eq!(failures.len(), 2, "{report:?}");
        for slug in ["first", "duplicate"] {
            assert!(
                failures.iter().any(|i| i
                    .subject
                    .as_ref()
                    .and_then(|r| r.id.as_ref())
                    .is_some_and(|id| id.slug == slug)),
                "{report:?}"
            );
        }
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 6);
    }

    #[tokio::test]
    async fn validate_project_distinguishes_message_selections_and_redacts_model_errors() {
        let service = fixture().await;
        library(
            &service,
            library_id("types", 1),
            "syntax = \"proto3\"; package orders.v1; message Order {} message Replacement {}",
            vec![],
        )
        .await;
        selection(
            &service,
            "valid",
            library_id("types", 1),
            "orders.v1.Replacement",
        )
        .await;
        selection(
            &service,
            "removed",
            library_id("types", 1),
            "orders.v1.Order",
        )
        .await;
        library(
            &service,
            library_id("types", 1),
            "syntax = \"proto3\"; package orders.v1; message Replacement {}",
            vec![],
        )
        .await;
        let report = validate(&service).await.unwrap().into_inner();
        assert_eq!(issues(&report).len(), 1, "{report:?}");
        assert_eq!(
            issues(&report)[0]
                .subject
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .slug,
            "removed"
        );
        for (source, dependencies) in [
            ("syntax = \"proto3\"; private compiler source", vec![]),
            (
                VALID,
                vec![pb::TypeLibraryRef {
                    id: Some(pb::Id {
                        namespace: "types".into(),
                        slug: "secret-dependency".into(),
                        version: 1,
                    }),
                }],
            ),
        ] {
            library(&service, library_id("types", 1), source, dependencies).await;
            let report = validate(&service).await.unwrap().into_inner();
            assert_eq!(issues(&report).len(), 2, "{report:?}");
            for issue in issues(&report) {
                for private in [
                    "orders.proto",
                    "secret-dependency",
                    "private compiler source",
                    "orders.v1",
                ] {
                    assert!(!format!("{issue:?}").contains(private), "{issue:?}");
                }
            }
        }
    }
    #[tokio::test]
    async fn data_product_compilation_memo_and_operational_errors_are_request_scoped() {
        let mut service = fixture().await;
        library(&service, library_id("types", 1), VALID, vec![]).await;
        let selection = pb::TypeLibraryTypeRef {
            library: Some(pb::TypeLibraryRef {
                id: Some(library_id("types", 1)),
            }),
            message: Some(pb::QualifiedProtoName {
                value: "orders.v1.Order".into(),
            }),
        };
        let product = pb::Entity {
            kind: Some(pb::entity::Kind::DataProduct(pb::DataProduct {
                id: Some(pb::Id {
                    namespace: "contracts".into(),
                    slug: "product".into(),
                    version: 1,
                }),
                title: "Product".into(),
                outputs: ["First", "Second"]
                    .into_iter()
                    .map(|name| pb::DataOutputPort {
                        name: Some(pb::PortName { value: name.into() }),
                        contract: Some(pb::OutputContract {
                            r#type: Some(pb::output_contract::Type::LibraryType(selection.clone())),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let specification = pb::Entity {
            kind: Some(pb::entity::Kind::Specification(pb::Specification {
                id: Some(pb::Id {
                    namespace: "contracts".into(),
                    slug: "specification".into(),
                    version: 1,
                }),
                title: "Specification".into(),
                messages: vec![pb::ContractMessage {
                    name: Some(pb::MemberName {
                        value: "Payload".into(),
                    }),
                    schemas: vec![pb::SchemaBinding {
                        role: pb::schema_binding::Role::Payload as i32,
                        r#type: Some(pb::schema_binding::Type::LibraryType(selection)),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let before = service.type_libraries.contract_compile_calls();
        service
            .batch_mutate(Request::new(pb::BatchMutateRequest {
                ops: [product, specification]
                    .into_iter()
                    .map(|entity| pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                            entity: Some(entity),
                            ..Default::default()
                        })),
                    })
                    .collect(),
                ..Default::default()
            }))
            .await
            .unwrap();
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 1);
        let request = || {
            Request::new(pb::InspectDataProductRequest {
                product: Some(pb::DataProductRef {
                    id: Some(pb::Id {
                        namespace: "contracts".into(),
                        slug: "product".into(),
                        version: 1,
                    }),
                }),
            })
        };
        service.inspect_data_product(request()).await.unwrap();
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 2);
        service.inspect_data_product(request()).await.unwrap();
        assert_eq!(service.type_libraries.contract_compile_calls() - before, 3);
        service.type_libraries =
            TypeLibraryGate::with_compile_runner(CompileRunner::new(0, Duration::from_millis(1)));
        let error = service.inspect_data_product(request()).await.unwrap_err();
        assert_eq!(error.code(), Code::DeadlineExceeded);
        for private in ["orders.proto", "orders.v1", "Order"] {
            assert!(!format!("{error:?}").contains(private));
        }
        service.type_libraries = TypeLibraryGate::default();
        library(
            &service,
            library_id("types", 1),
            "private compiler source",
            vec![],
        )
        .await;
        let mut edited = service
            .store
            .get(
                pb::EntityKind::DataProduct,
                &pb::Id {
                    namespace: "contracts".into(),
                    slug: "product".into(),
                    version: 1,
                },
                None,
            )
            .await
            .unwrap()
            .entity;
        let Some(pb::entity::Kind::DataProduct(product)) = &mut edited.kind else {
            panic!("DataProduct")
        };
        product.title = "Changed".into();
        let error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(edited),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument);
        for private in ["orders.proto", "private compiler source", "orders.v1"] {
            assert!(!format!("{error:?}").contains(private));
        }
    }
}
