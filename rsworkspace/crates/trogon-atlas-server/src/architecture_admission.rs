use std::collections::{HashMap, HashSet};

use trogon_atlas_store::store::BranchLandOp;

use super::{
    entity_id, entity_kind, pb, store_err, validation, EventModelServiceImpl, OpMetaLike, Status,
    StoredEntity, Visibility,
};

struct ArchitectureProposal<'a> {
    overlay: HashMap<pb::EntityKey, Option<&'a pb::Entity>>,
    landing: Option<&'a [BranchLandOp]>,
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
        .map(|_| ())
    }

    pub(super) async fn validate_architecture_writes(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        self.validate_architecture_proposal(metas, branch, vis, None)
            .await
            .map(|_| ())
    }

    pub(super) async fn validate_architecture_landing(
        &self,
        metas: &[OpMetaLike],
        ops: &[BranchLandOp],
        vis: &Visibility,
    ) -> Result<Vec<StoredEntity>, Status> {
        self.validate_architecture_proposal(metas, None, vis, Some(ops))
            .await
    }

    async fn validate_architecture_proposal(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
        landing: Option<&[BranchLandOp]>,
    ) -> Result<Vec<StoredEntity>, Status> {
        let resources: Vec<_> = metas
            .iter()
            .filter(|meta| {
                meta.is_put
                    && matches!(
                        meta.kind,
                        pb::EntityKind::Channel | pb::EntityKind::DataStore
                    )
            })
            .filter_map(|meta| meta.entity.as_ref())
            .collect();
        if resources.is_empty() {
            return Ok(Vec::new());
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
            required.extend(
                trogon_atlas_core::refs::outbound_refs(entity)
                    .into_iter()
                    .filter_map(|reference| reference.to_kind.map(|kind| (kind, reference.to_id))),
            );
        }
        let mut loaded = HashSet::new();
        self.load_architecture_targets(&proposal, branch, vis, required, &mut loaded, &mut world)
            .await?;
        let mut endpoints = Vec::new();
        for resource in &resources {
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
                let Some(stored) =
                    world.get(&pb::EntityKey::new(pb::EntityKind::AutomationSlice, id))
                else {
                    continue;
                };
                let Some(pb::entity::Kind::AutomationSlice(slice)) = stored.entity.kind.as_ref()
                else {
                    continue;
                };
                if let Some(id) = slice
                    .source_read_models
                    .iter()
                    .find(|edge| {
                        edge.connection_id.is_some() && edge.connection_id == observation.id
                    })
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
        self.load_architecture_targets(&proposal, branch, vis, endpoints, &mut loaded, &mut world)
            .await?;
        let context: Vec<_> = world.into_values().collect();
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
            let issues = validation::architecture_resources::validate(&prepared, &context);
            if !issues.is_empty() {
                return Err(Status::invalid_argument(super::format_scenario_issues(
                    &issues,
                )));
            }
        }
        Ok(context)
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
                    kinds: &[pb::EntityKind::Channel, pb::EntityKind::DataStore],
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
        vis: &Visibility,
        targets: Vec<(pb::EntityKind, pb::Id)>,
        loaded: &mut HashSet<pb::EntityKey>,
        world: &mut HashMap<pb::EntityKey, StoredEntity>,
    ) -> Result<(), Status> {
        for (kind, id) in targets {
            self.require_visible(vis, &id.namespace).await?;
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
                world.insert(key, stored);
            }
        }
        Ok(())
    }

    pub(super) async fn validate_architecture_write(
        &self,
        entity: &pb::Entity,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        if !matches!(
            entity.kind,
            Some(pb::entity::Kind::Channel(_) | pb::entity::Kind::DataStore(_))
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
