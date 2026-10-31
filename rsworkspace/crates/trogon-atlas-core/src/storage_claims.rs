use std::collections::HashSet;

use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind};

use crate::{connections, refs::OutboundRef};

pub const MAX_CLAIMS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct StorageClaimError(pub String);
type Result<T> = std::result::Result<T, StorageClaimError>;

#[must_use]
pub fn claims(entity: &Entity) -> &[pb::StorageAccessClaim] {
    match &entity.kind {
        Some(entity::Kind::Component(value)) => &value.storage_claims,
        Some(entity::Kind::ExternalSystem(value)) => &value.storage_claims,
        Some(entity::Kind::DataProduct(value)) => &value.storage_claims,
        _ => &[],
    }
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(StorageClaimError(message.into()))
    }
}
fn exact(id: Option<&pb::Id>) -> Result<&pb::Id> {
    id.filter(|id| {
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug)
    })
    .ok_or_else(|| StorageClaimError("invalid exact storage claim reference".into()))
}
fn validate_attachment(attachment: &pb::ConnectionRef, store: &pb::Id) -> Result<()> {
    let owner = attachment
        .owner
        .as_ref()
        .ok_or_else(|| StorageClaimError("storage attachment owner required".into()))?;
    require(
        owner.kind == EntityKind::DataStore as i32 && exact(owner.id.as_ref())? == store,
        "storage attachment must belong to the claimed store",
    )?;
    require(
        uuid::Uuid::parse_str(&attachment.owner_uid)
            .is_ok_and(|uid| !uid.is_nil() && uid.to_string() == attachment.owner_uid),
        "invalid storage attachment owner UID",
    )?;
    attachment
        .id
        .as_ref()
        .ok_or_else(|| StorageClaimError("storage attachment ID required".into()))?
        .value
        .parse::<connections::ConnectionId>()
        .map_err(|e| StorageClaimError(e.to_string()))?;
    Ok(())
}

/// Validate claims against the owner's local ports, which only a `DataProduct` has.
pub(crate) fn validate_claims(
    values: &[pb::StorageAccessClaim],
    ports: Option<&HashSet<String>>,
) -> Result<()> {
    require(values.len() <= MAX_CLAIMS, "storage claim limit exceeded")?;
    for value in values {
        if let Some(id) = &value.connection_id {
            id.value
                .parse::<connections::ConnectionId>()
                .map_err(|e| StorageClaimError(e.to_string()))?;
        }
        require(
            matches!(
                pb::StorageDirection::try_from(value.direction),
                Ok(pb::StorageDirection::Read | pb::StorageDirection::Write)
            ),
            "storage direction required",
        )?;
        let store = exact(value.store.as_ref().and_then(|s| s.id.as_ref()))?;
        if let Some(attachment) = &value.attachment {
            validate_attachment(attachment, store)?;
        }
        if let Some(port) = &value.port_id {
            require(
                ports.is_some_and(|ports| ports.contains(&port.value)),
                "storage claim port must be an existing local port",
            )?;
        }
        if let Some(doc) = &value.doc {
            crate::decision_document::validate_catalog_text(Some(doc))
                .map_err(|e| StorageClaimError(e.to_string()))?;
        }
    }
    Ok(())
}

pub fn validate_values(entity: &Entity) -> Result<()> {
    let ports = match &entity.kind {
        Some(entity::Kind::Component(_) | entity::Kind::ExternalSystem(_)) => None,
        Some(entity::Kind::DataProduct(product)) => Some(
            crate::data_product::port_id_strings(product, false)
                .map_err(|e| StorageClaimError(e.to_string()))?,
        ),
        _ => return Ok(()),
    };
    require(
        entity.encoded_len() <= MAX_ENCODED_BYTES,
        "storage claim owner exceeds encoded byte limit",
    )?;
    validate_claims(claims(entity), ports.as_ref())?;
    connections::validate_connection_ids(entity).map_err(|e| StorageClaimError(e.to_string()))
}

pub(crate) fn outbound(values: &[pb::StorageAccessClaim], out: &mut Vec<OutboundRef>) {
    for (index, value) in values.iter().enumerate() {
        if let Some(id) = value.store.as_ref().and_then(|s| s.id.clone()) {
            out.push(OutboundRef {
                to_kind: Some(EntityKind::DataStore),
                to_id: id,
                from_field: format!("storage_claims[{index}].store"),
            });
        }
        if let Some(id) = value
            .attachment
            .as_ref()
            .and_then(|a| a.owner.as_ref())
            .and_then(|o| o.id.clone())
        {
            out.push(OutboundRef {
                to_kind: Some(EntityKind::DataStore),
                to_id: id,
                from_field: format!("storage_claims[{index}].attachment.owner"),
            });
        }
    }
}

/// An attachment pins the store incarnation, so only unattached claims follow a store retarget.
pub(crate) fn retarget(
    values: &mut [pb::StorageAccessClaim],
    kind: EntityKind,
    from: &pb::Id,
    to: &pb::Id,
    rewritten: &mut Vec<String>,
) {
    if kind != EntityKind::DataStore {
        return;
    }
    for (index, value) in values.iter_mut().enumerate() {
        if value.attachment.is_some() {
            continue;
        }
        if let Some(id) = value.store.as_mut().and_then(|s| s.id.as_mut()) {
            if id == from && from != to {
                *id = to.clone();
                value.connection_id = None;
                rewritten.push(format!("storage_claims[{index}].store"));
            }
        }
    }
}
