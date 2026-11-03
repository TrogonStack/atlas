use std::collections::{HashMap, HashSet};

use trogon_atlas_store::error::StoreError;

use super::{
    entity_id, pb, store_err, EventModelServiceImpl, ListFilter, OpMetaLike, Status, StoredEntity,
    Visibility,
};

impl EventModelServiceImpl {
    pub(super) async fn validate_business_writes(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        self.admit_business_relationships(metas, metas, branch, vis)
            .await
    }

    /// Source batches set `preserve_matching` so a title or fields update can
    /// keep a dangling target property. The overlay still includes every op,
    /// including ones whose declarations match and are not rechecked.
    pub(super) async fn validate_batch_business_writes(
        &self,
        metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
        preserve_matching: bool,
    ) -> Result<(), Status> {
        if !preserve_matching {
            return self.validate_business_writes(metas, branch, vis).await;
        }
        let mut checked = Vec::new();
        for meta in metas {
            if !meta.is_put {
                continue;
            }
            let Some(entity) = meta.entity.as_ref() else {
                continue;
            };
            if !matches!(entity.kind, Some(pb::entity::Kind::Schema(_))) {
                continue;
            }
            match self.store.get(meta.kind, &meta.id, branch).await {
                Ok(stored)
                    if trogon_atlas_core::business_data::business_declarations_match(
                        &stored.entity,
                        entity,
                    ) => {}
                Ok(_) | Err(StoreError::NotFound) => checked.push(meta.clone()),
                Err(error) => return Err(store_err(error)),
            }
        }
        if checked.is_empty() {
            return Ok(());
        }
        self.admit_business_relationships(&checked, metas, branch, vis)
            .await
    }

    async fn admit_business_relationships(
        &self,
        checked: &[OpMetaLike],
        overlay_metas: &[OpMetaLike],
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        let overlay: HashMap<pb::EntityKey, Option<pb::Entity>> = overlay_metas
            .iter()
            .map(|meta| {
                (
                    pb::EntityKey::new(meta.kind, &meta.id),
                    meta.entity.clone().filter(|_| meta.is_put),
                )
            })
            .collect();
        for meta in checked.iter().filter(|meta| meta.is_put) {
            let Some(entity) = meta.entity.as_ref() else {
                continue;
            };
            let Some(pb::entity::Kind::Schema(schema)) = entity.kind.as_ref() else {
                continue;
            };
            for relationship in &schema.property_relationships {
                let target_id = relationship
                    .target_entity
                    .as_ref()
                    .and_then(|target| target.id.as_ref())
                    .ok_or_else(|| {
                        Status::invalid_argument("relationship target schema is required")
                    })?;
                let target = self
                    .business_target_schema(schema, target_id, &overlay, branch, vis)
                    .await?;
                if let Some(path) = relationship.target_property.as_ref() {
                    trogon_atlas_core::business_data::resolve_path(&target.fields, path)
                        .map_err(|error| Status::invalid_argument(error.to_string()))?;
                }
            }
        }
        Ok(())
    }

    async fn business_target_schema(
        &self,
        schema: &pb::Schema,
        target_id: &pb::Id,
        overlay: &HashMap<pb::EntityKey, Option<pb::Entity>>,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<pb::Schema, Status> {
        if schema.id.as_ref() == Some(target_id) {
            return Ok(schema.clone());
        }
        let key = pb::EntityKey::new(pb::EntityKind::Schema, target_id);
        if let Some(slot) = overlay.get(&key) {
            self.require_visible(vis, &target_id.namespace).await?;
            return match slot {
                Some(entity) => schema_body(entity),
                None => Err(Status::not_found("entity not found")),
            };
        }
        self.require_visible(vis, &target_id.namespace).await?;
        match self
            .store
            .get(pb::EntityKind::Schema, target_id, branch)
            .await
        {
            Ok(stored) => schema_body(&stored.entity),
            Err(StoreError::NotFound) => Err(Status::not_found("entity not found")),
            Err(error) => Err(store_err(error)),
        }
    }

    /// Schemas share the general snapshot cap and the architecture resource
    /// scan does not restore them. Callers that asked for schema referrers
    /// cannot treat that incoming set as complete when the cap dropped schemas.
    pub(super) async fn refuse_omitted_schema_referrers(
        &self,
        snapshot: &[StoredEntity],
        branch: Option<&str>,
        vis: &Visibility,
        filter_kinds: &[pb::EntityKind],
        truncated: bool,
    ) -> Result<(), Status> {
        if !truncated || !schema_referrers_in_scope(filter_kinds) {
            return Ok(());
        }
        let lens = self.lens(vis).await?;
        let namespaces: Vec<String> = match &lens {
            super::Lens::Everything => Vec::new(),
            super::Lens::Only(namespaces) if namespaces.is_empty() => return Ok(()),
            super::Lens::Only(namespaces) => {
                namespaces.iter().map(|id| id.as_str().to_owned()).collect()
            }
        };
        let kept = snapshot
            .iter()
            .filter(|stored| matches!(stored.entity.kind, Some(pb::entity::Kind::Schema(_))))
            .filter(|stored| entity_id(&stored.entity).is_ok_and(|id| lens.admits(&id.namespace)))
            .count();
        let kinds = [pb::EntityKind::Schema];
        let probe = self
            .store
            .list(
                ListFilter {
                    kinds: &kinds,
                    namespaces: &namespaces,
                    ..Default::default()
                },
                Some(kept.saturating_add(1)),
                branch,
            )
            .await
            .map_err(store_err)?;
        if probe.len() > kept {
            return Err(Status::resource_exhausted(
                "business reference discovery is incomplete: the entity snapshot omitted schemas",
            ));
        }
        Ok(())
    }

    /// Every schema opened by the scan pays the shared row and occurrence
    /// budget once. A schema that is not a match was still examined.
    pub(super) fn account_examined_schemas<'a>(
        budget: &mut trogon_atlas_core::business_data::ReferenceBudget,
        entities: impl IntoIterator<Item = &'a pb::Entity>,
    ) -> Result<(), Status> {
        for entity in entities {
            let Some(pb::entity::Kind::Schema(schema)) = entity.kind.as_ref() else {
                continue;
            };
            let Ok(id) = entity_id(entity) else {
                continue;
            };
            budget
                .charge_row(&examined_schema_key(id))
                .map_err(|error| budget_status(&error))?;
            budget
                .charge_occurrences(schema.identifiers.len() + schema.property_relationships.len())
                .map_err(|error| budget_status(&error))?;
        }
        Ok(())
    }

    pub(super) async fn decorate_business_property_status(
        &self,
        references: &mut [pb::Reference],
        outgoing_owner: Option<&pb::Entity>,
        branch: Option<&str>,
        vis: &Visibility,
        coherent_owners: Option<&HashMap<pb::EntityKey, pb::Entity>>,
        budget: &mut trogon_atlas_core::business_data::ReferenceBudget,
    ) -> Result<(), Status> {
        if !references.iter().any(|reference| {
            trogon_atlas_core::business_data::relationship_index(&reference.from_field).is_some()
        }) {
            return Ok(());
        }
        let lens = self.lens(vis).await?;
        let mut owners: HashMap<pb::EntityKey, pb::Entity> = HashMap::new();
        let mut targets: HashMap<pb::EntityKey, Option<pb::Schema>> = HashMap::new();
        let mut inventoried = HashSet::new();
        if let Some(owner) = outgoing_owner {
            if let Ok(id) = entity_id(owner) {
                if let Ok(kind) = super::entity_kind(owner) {
                    owners.insert(pb::EntityKey::new(kind, id), owner.clone());
                }
            }
        }
        for reference in references.iter_mut() {
            // The index only recognizes a business site. Identity is the
            // occurrence id: a reordered owner can put a sibling at that index.
            if trogon_atlas_core::business_data::relationship_index(&reference.from_field).is_none()
            {
                continue;
            }
            budget
                .charge_record()
                .map_err(|error| budget_status(&error))?;
            let Some(owner_ref) = reference.from.as_ref() else {
                reference.business_property_status = Some(unavailable());
                continue;
            };
            let Ok(kind) = pb::EntityKind::try_from(owner_ref.kind) else {
                reference.business_property_status = Some(unavailable());
                continue;
            };
            let Some(owner_id) = owner_ref.id.as_ref() else {
                reference.business_property_status = Some(unavailable());
                continue;
            };
            let owner_key = pb::EntityKey::new(kind, owner_id);
            if inventoried.insert(owner_key.clone()) && !owners.contains_key(&owner_key) {
                // A supplied snapshot is the body that stamped the reference.
                // Falling through to a later read can attach a sibling path.
                if let Some(coherent) = coherent_owners {
                    if let Some(entity) = coherent.get(&owner_key) {
                        owners.insert(owner_key.clone(), entity.clone());
                    } else {
                        reference.business_property_status = Some(unavailable());
                        continue;
                    }
                } else {
                    match self.store.get(kind, owner_id, branch).await {
                        Ok(stored) => {
                            owners.insert(owner_key.clone(), stored.entity);
                        }
                        Err(StoreError::NotFound) => {
                            reference.business_property_status = Some(unavailable());
                            continue;
                        }
                        Err(error) => return Err(store_err(error)),
                    }
                }
            }
            let relationship = owners.get(&owner_key).and_then(|entity| {
                occurrence_relationship(entity, reference.connection_id.as_ref()).cloned()
            });
            let Some(relationship) = relationship else {
                reference.business_property_status = Some(unavailable());
                continue;
            };
            let Some(path) = relationship.target_property.clone() else {
                reference.business_property_status =
                    Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32);
                continue;
            };
            budget
                .charge_occurrences(path.segments.len())
                .map_err(|error| budget_status(&error))?;
            let Some(target_id) = reference.to.as_ref().and_then(|to| to.id.clone()) else {
                reference.business_property_status = Some(unavailable());
                continue;
            };
            if !lens.admits(&target_id.namespace) {
                reference.business_property_status = Some(unavailable());
                continue;
            }
            let target_key = pb::EntityKey::new(pb::EntityKind::Schema, &target_id);
            if !targets.contains_key(&target_key) {
                budget
                    .charge_row(&examined_schema_key(&target_id))
                    .map_err(|error| budget_status(&error))?;
                let loaded = match self
                    .store
                    .get(pb::EntityKind::Schema, &target_id, branch)
                    .await
                {
                    Ok(stored) => match &stored.entity.kind {
                        Some(pb::entity::Kind::Schema(schema)) => Some(schema.clone()),
                        _ => None,
                    },
                    Err(StoreError::NotFound) => None,
                    Err(error) => return Err(store_err(error)),
                };
                targets.insert(target_key.clone(), loaded);
            }
            let status = match targets.get(&target_key).and_then(Option::as_ref) {
                Some(schema) => {
                    match trogon_atlas_core::business_data::resolve_path(&schema.fields, &path) {
                        Ok(_) => pb::BusinessPropertyReferenceStatus::PropertyResolved,
                        Err(_) => pb::BusinessPropertyReferenceStatus::PropertyUnresolved,
                    }
                }
                None => pb::BusinessPropertyReferenceStatus::Unavailable,
            };
            reference.business_property_status = Some(status as i32);
        }
        Ok(())
    }
}

fn schema_referrers_in_scope(filter_kinds: &[pb::EntityKind]) -> bool {
    filter_kinds.is_empty() || filter_kinds.contains(&pb::EntityKind::Schema)
}

fn examined_schema_key(id: &pb::Id) -> String {
    format!("schema:{}:{}:{}", id.namespace, id.slug, id.version)
}

fn occurrence_relationship<'a>(
    entity: &'a pb::Entity,
    connection_id: Option<&pb::ConnectionId>,
) -> Option<&'a pb::BusinessPropertyRelationship> {
    let connection_id = connection_id.filter(|id| !id.value.is_empty())?;
    let pb::entity::Kind::Schema(schema) = entity.kind.as_ref()? else {
        return None;
    };
    schema.property_relationships.iter().find(|relationship| {
        relationship
            .connection_id
            .as_ref()
            .is_some_and(|id| id.value == connection_id.value)
    })
}

fn schema_body(entity: &pb::Entity) -> Result<pb::Schema, Status> {
    match &entity.kind {
        Some(pb::entity::Kind::Schema(schema)) => Ok(schema.clone()),
        _ => Err(Status::not_found("entity not found")),
    }
}

fn unavailable() -> i32 {
    pb::BusinessPropertyReferenceStatus::Unavailable as i32
}

fn budget_status(error: &trogon_atlas_core::business_data::BusinessError) -> Status {
    Status::resource_exhausted(error.to_string())
}
