use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind, EntityRef};

use crate::{connections, entity_id, entity_kind, refs::OutboundRef};

pub const MAX_RELATIONSHIPS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ArchitecturalRelationshipError(pub String);

type Result<T> = std::result::Result<T, ArchitecturalRelationshipError>;

#[must_use]
pub fn relationships(entity: &Entity) -> &[pb::ArchitecturalRelationship] {
    match &entity.kind {
        Some(entity::Kind::Component(value)) => &value.relationships,
        Some(entity::Kind::ExternalSystem(value)) => &value.relationships,
        _ => &[],
    }
}

#[must_use]
pub fn endpoint_ref(endpoint: &pb::ArchitecturalEndpoint) -> Option<EntityRef> {
    use pb::architectural_endpoint::Target;
    let (kind, id) = match endpoint.target.as_ref()? {
        Target::Component(value) => (EntityKind::Component, value.id.clone()),
        Target::ExternalSystem(value) => (EntityKind::ExternalSystem, value.id.clone()),
    };
    Some(EntityRef {
        kind: kind as i32,
        id,
    })
}

fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(ArchitecturalRelationshipError(message.into()))
    }
}

pub fn validate_values(entity: &Entity) -> Result<()> {
    if !matches!(
        entity_kind(entity),
        Some(EntityKind::Component | EntityKind::ExternalSystem)
    ) {
        return Ok(());
    }
    require(
        entity.encoded_len() <= MAX_ENCODED_BYTES,
        "architectural owner exceeds encoded byte limit",
    )?;
    let owner = entity_id(entity)
        .ok_or_else(|| ArchitecturalRelationshipError("exact owner ID required".into()))?;
    exact(Some(owner))?;
    require(
        relationships(entity).len() <= MAX_RELATIONSHIPS,
        "architectural relationship limit exceeded",
    )?;
    for value in relationships(entity) {
        if let Some(id) = &value.connection_id {
            id.value
                .parse::<connections::ConnectionId>()
                .map_err(|e| ArchitecturalRelationshipError(e.to_string()))?;
        }
        let target = value
            .target
            .as_ref()
            .and_then(endpoint_ref)
            .ok_or_else(|| {
                ArchitecturalRelationshipError("architectural target required".into())
            })?;
        exact(target.id.as_ref())?;
        require(
            entity_kind(entity).map(|kind| kind as i32) != Some(target.kind)
                || target.id.as_ref() != Some(owner),
            "architectural self target forbidden",
        )?;
        crate::decision_document::validate_catalog_title(value.label.as_ref())
            .map_err(|e| ArchitecturalRelationshipError(e.to_string()))?;
        if let Some(doc) = &value.doc {
            crate::decision_document::validate_catalog_text(Some(doc))
                .map_err(|e| ArchitecturalRelationshipError(e.to_string()))?;
        }
    }
    connections::validate_connection_ids(entity)
        .map_err(|e| ArchitecturalRelationshipError(e.to_string()))
}

fn exact(id: Option<&pb::Id>) -> Result<()> {
    require(
        id.is_some_and(|id| {
            id.version > 0
                && crate::is_safe_id_component(&id.namespace)
                && crate::is_safe_id_component(&id.slug)
        }),
        "invalid exact architectural endpoint",
    )
}

pub fn resolve_connection_context(
    owner: &Entity,
    reference: &pb::ConnectionRef,
) -> Result<EntityRef> {
    require(
        matches!(
            entity_kind(owner),
            Some(EntityKind::Component | EntityKind::ExternalSystem)
        ),
        "unsupported architectural owner",
    )?;
    let occurrence = connections::resolve_connection(owner, reference)
        .map_err(|e| ArchitecturalRelationshipError(e.to_string()))?;
    if occurrence.path.starts_with("component.ui_hostings[") {
        return occurrence.target.ok_or_else(|| ArchitecturalRelationshipError("hosting UI required".into()));
    }
    let prefix = match entity_kind(owner) {
        Some(EntityKind::Component) => "component.relationships[",
        _ => "external_system.relationships[",
    };
    if occurrence.path.starts_with("component.communications[")
        || occurrence
            .path
            .starts_with("external_system.communications[")
    {
        return crate::communication_intent::resolve_connection_context(owner, reference)
            .map_err(|e| ArchitecturalRelationshipError(e.to_string()));
    }
    require(
        occurrence.path.starts_with(prefix),
        "unsupported architectural connection role",
    )?;
    let target = occurrence
        .target
        .ok_or_else(|| ArchitecturalRelationshipError("architectural target required".into()))?;
    exact(target.id.as_ref())?;
    Ok(target)
}

pub(crate) fn outbound(values: &[pb::ArchitecturalRelationship], out: &mut Vec<OutboundRef>) {
    for (index, value) in values.iter().enumerate() {
        if let Some(target) = value.target.as_ref().and_then(endpoint_ref) {
            if let (Ok(kind), Some(id)) = (EntityKind::try_from(target.kind), target.id) {
                let arm = if kind == EntityKind::Component {
                    "component"
                } else {
                    "external_system"
                };
                out.push(OutboundRef {
                    to_kind: Some(kind),
                    to_id: id,
                    from_field: format!("relationships[{index}].target.{arm}"),
                });
            }
        }
    }
}

pub(crate) fn retarget(
    values: &mut [pb::ArchitecturalRelationship],
    kind: EntityKind,
    from: &pb::Id,
    to: &pb::Id,
    rewritten: &mut Vec<String>,
) {
    use pb::architectural_endpoint::Target;
    for (index, value) in values.iter_mut().enumerate() {
        let Some(target) = value
            .target
            .as_mut()
            .and_then(|target| target.target.as_mut())
        else {
            continue;
        };
        let (target_kind, id, arm) = match target {
            Target::Component(value) => (EntityKind::Component, &mut value.id, "component"),
            Target::ExternalSystem(value) => {
                (EntityKind::ExternalSystem, &mut value.id, "external_system")
            }
        };
        if kind == target_kind && id.as_ref() == Some(from) && from != to {
            *id = Some(to.clone());
            value.connection_id = None;
            rewritten.push(format!("relationships[{index}].target.{arm}"));
        }
    }
}
