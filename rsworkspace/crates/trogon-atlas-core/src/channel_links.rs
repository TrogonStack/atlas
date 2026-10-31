use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind};

use crate::{connections, refs::OutboundRef};

pub const MAX_LINKS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ChannelLinkError(pub String);
type Result<T> = std::result::Result<T, ChannelLinkError>;

#[must_use]
pub fn links(entity: &Entity) -> &[pb::ChannelLink] {
    match &entity.kind {
        Some(entity::Kind::Channel(value)) => &value.links,
        _ => &[],
    }
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(ChannelLinkError(message.into()))
    }
}
fn exact(id: Option<&pb::Id>) -> Result<&pb::Id> {
    id.filter(|id| {
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug)
    })
    .ok_or_else(|| ChannelLinkError("invalid exact channel link target".into()))
}

pub fn validate_values(entity: &Entity) -> Result<()> {
    let Some(entity::Kind::Channel(channel)) = &entity.kind else {
        return Ok(());
    };
    require(
        entity.encoded_len() <= MAX_ENCODED_BYTES,
        "channel owner exceeds encoded byte limit",
    )?;
    require(
        channel.links.len() <= MAX_LINKS,
        "channel link limit exceeded",
    )?;
    for value in &channel.links {
        if let Some(id) = &value.connection_id {
            id.value
                .parse::<connections::ConnectionId>()
                .map_err(|e| ChannelLinkError(e.to_string()))?;
        }
        require(
            matches!(
                pb::ChannelLinkKind::try_from(value.kind),
                Ok(pb::ChannelLinkKind::Composes | pb::ChannelLinkKind::RoutesTo)
            ),
            "channel link kind required",
        )?;
        let target = exact(value.target.as_ref().and_then(|t| t.id.as_ref()))?;
        let owner = channel.id.as_ref();
        require(
            owner.is_none_or(|owner| owner != target),
            "channel link to the same channel forbidden",
        )?;
        if let Some(doc) = &value.doc {
            crate::decision_document::validate_catalog_text(Some(doc))
                .map_err(|e| ChannelLinkError(e.to_string()))?;
        }
    }
    connections::validate_connection_ids(entity).map_err(|e| ChannelLinkError(e.to_string()))
}

pub(crate) fn outbound(values: &[pb::ChannelLink], out: &mut Vec<OutboundRef>) {
    for (index, value) in values.iter().enumerate() {
        if let Some(id) = value.target.as_ref().and_then(|t| t.id.clone()) {
            out.push(OutboundRef {
                to_kind: Some(EntityKind::Channel),
                to_id: id,
                from_field: format!("links[{index}].target"),
            });
        }
    }
}

pub(crate) fn retarget(
    values: &mut [pb::ChannelLink],
    kind: EntityKind,
    from: &pb::Id,
    to: &pb::Id,
    rewritten: &mut Vec<String>,
) {
    if kind != EntityKind::Channel {
        return;
    }
    for (index, value) in values.iter_mut().enumerate() {
        if let Some(id) = value.target.as_mut().and_then(|t| t.id.as_mut()) {
            if id == from && from != to {
                *id = to.clone();
                value.connection_id = None;
                rewritten.push(format!("links[{index}].target"));
            }
        }
    }
}
