use std::collections::{HashMap, HashSet};

use trogon_atlas_store::error::StoreError;

use super::{entity_id, pb, store_err, EventModelServiceImpl, OpMetaLike, Status, Visibility};

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

    pub(super) async fn decorate_business_property_status(
        &self,
        references: &mut [pb::Reference],
        outgoing_owner: Option<&pb::Entity>,
        branch: Option<&str>,
        vis: &Visibility,
    ) -> Result<(), Status> {
        if !references.iter().any(|reference| {
            trogon_atlas_core::business_data::relationship_index(&reference.from_field).is_some()
        }) {
            return Ok(());
        }
        let lens = self.lens(vis).await?;
        let mut budget = trogon_atlas_core::business_data::ReferenceBudget::default();
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
            let Some(index) =
                trogon_atlas_core::business_data::relationship_index(&reference.from_field)
            else {
                continue;
            };
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
            if inventoried.insert(owner_key.clone()) {
                budget
                    .charge_row(&format!(
                        "owner:{}:{}:{}:{}",
                        kind as i32, owner_id.namespace, owner_id.slug, owner_id.version
                    ))
                    .map_err(|error| budget_status(&error))?;
                if !owners.contains_key(&owner_key) {
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
                if let Some(pb::entity::Kind::Schema(schema)) = owners
                    .get(&owner_key)
                    .and_then(|entity| entity.kind.as_ref())
                {
                    budget
                        .charge_occurrences(
                            schema.identifiers.len() + schema.property_relationships.len(),
                        )
                        .map_err(|error| budget_status(&error))?;
                }
            }
            let relationship = owners
                .get(&owner_key)
                .and_then(|entity| match &entity.kind {
                    Some(pb::entity::Kind::Schema(schema)) => {
                        schema.property_relationships.get(index).cloned()
                    }
                    _ => None,
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
                    .charge_row(&format!(
                        "target:{}:{}:{}",
                        target_id.namespace, target_id.slug, target_id.version
                    ))
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
