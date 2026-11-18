use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind, EntityRef};

use crate::{
    architectural_relationships::endpoint_ref, connections, entity_id, entity_kind,
    refs::OutboundRef,
};

pub const MAX_COMMUNICATIONS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct CommunicationIntentError(pub String);
type Result<T> = std::result::Result<T, CommunicationIntentError>;

#[must_use]
pub fn declarations(entity: &Entity) -> &[pb::CommunicationDeclaration] {
    match &entity.kind {
        Some(entity::Kind::Component(value)) => &value.communications,
        Some(entity::Kind::ExternalSystem(value)) => &value.communications,
        _ => &[],
    }
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(CommunicationIntentError(message.into()))
    }
}
fn exact(id: Option<&pb::Id>) -> Result<()> {
    require(
        id.is_some_and(|id| {
            id.version > 0
                && crate::is_safe_id_component(&id.namespace)
                && crate::is_safe_id_component(&id.slug)
        }),
        "invalid exact communication endpoint",
    )
}
fn validate_message(message: &pb::ContractMemberRef) -> Result<()> {
    exact(
        message
            .specification
            .as_ref()
            .and_then(|value| value.id.as_ref()),
    )?;
    require(
        message.kind == pb::ContractMemberKind::Message as i32 && message.expected_digest.is_none(),
        "communication requires MESSAGE without digest",
    )?;
    require(
        uuid::Uuid::parse_str(&message.owner_uid)
            .is_ok_and(|uid| !uid.is_nil() && uid.to_string() == message.owner_uid),
        "invalid message owner UID",
    )?;
    message
        .member_id
        .as_ref()
        .ok_or_else(|| CommunicationIntentError("message member ID required".into()))?
        .value
        .parse::<crate::specification::MemberId>()
        .map_err(|e| CommunicationIntentError(e.to_string()))?;
    Ok(())
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
        "communication owner exceeds encoded byte limit",
    )?;
    let owner = entity_id(entity)
        .ok_or_else(|| CommunicationIntentError("exact owner ID required".into()))?;
    exact(Some(owner))?;
    require(
        declarations(entity).len() <= MAX_COMMUNICATIONS,
        "communication declaration limit exceeded",
    )?;
    for value in declarations(entity) {
        if let Some(id) = &value.connection_id {
            id.value
                .parse::<connections::ConnectionId>()
                .map_err(|e| CommunicationIntentError(e.to_string()))?;
        }
        require(
            matches!(
                pb::CommunicationDirection::try_from(value.direction),
                Ok(pb::CommunicationDirection::Send | pb::CommunicationDirection::Receive)
            ),
            "communication direction required",
        )?;
        let peer = value
            .peer
            .as_ref()
            .and_then(endpoint_ref)
            .ok_or_else(|| CommunicationIntentError("communication peer required".into()))?;
        exact(peer.id.as_ref())?;
        require(
            entity_kind(entity).map(|kind| kind as i32) != Some(peer.kind)
                || peer.id.as_ref() != Some(owner),
            "communication self peer forbidden",
        )?;
        validate_message(
            value
                .message
                .as_ref()
                .ok_or_else(|| CommunicationIntentError("communication message required".into()))?,
        )?;
        crate::decision_document::validate_catalog_title(value.label.as_ref())
            .map_err(|e| CommunicationIntentError(e.to_string()))?;
        for text in [&value.condition, &value.doc].into_iter().flatten() {
            crate::decision_document::validate_catalog_text(Some(text))
                .map_err(|e| CommunicationIntentError(e.to_string()))?;
        }
    }
    connections::validate_connection_ids(entity)
        .map_err(|e| CommunicationIntentError(e.to_string()))
}
fn selected<'a>(
    owner: &'a Entity,
    reference: &pb::ConnectionRef,
) -> Result<&'a pb::CommunicationDeclaration> {
    let occurrence = connections::resolve_connection(owner, reference)
        .map_err(|e| CommunicationIntentError(e.to_string()))?;
    let prefix = match entity_kind(owner) {
        Some(EntityKind::Component) => "component.communications[",
        Some(EntityKind::ExternalSystem) => "external_system.communications[",
        _ => {
            return Err(CommunicationIntentError(
                "unsupported communication owner".into(),
            ))
        }
    };
    require(
        occurrence.path.starts_with(prefix),
        "unsupported communication connection role",
    )?;
    declarations(owner)
        .iter()
        .find(|value| value.connection_id == reference.id)
        .ok_or_else(|| CommunicationIntentError("communication absent".into()))
}
pub fn resolve_connection_context(
    owner: &Entity,
    reference: &pb::ConnectionRef,
) -> Result<EntityRef> {
    let value = selected(owner, reference)?;
    require(
        matches!(
            pb::CommunicationDirection::try_from(value.direction),
            Ok(pb::CommunicationDirection::Send | pb::CommunicationDirection::Receive)
        ),
        "communication direction required",
    )?;
    validate_message(
        value
            .message
            .as_ref()
            .ok_or_else(|| CommunicationIntentError("communication message required".into()))?,
    )?;
    let peer = value
        .peer
        .as_ref()
        .and_then(endpoint_ref)
        .ok_or_else(|| CommunicationIntentError("communication peer required".into()))?;
    exact(peer.id.as_ref())?;
    Ok(peer)
}
pub fn message_selection(
    owner: &Entity,
    reference: &pb::ConnectionRef,
) -> Result<pb::ContractMemberRef> {
    let message = selected(owner, reference)?
        .message
        .as_ref()
        .ok_or_else(|| CommunicationIntentError("communication message required".into()))?;
    validate_message(message)?;
    Ok(message.clone())
}
pub(crate) fn outbound(values: &[pb::CommunicationDeclaration], out: &mut Vec<OutboundRef>) {
    for (index, value) in values.iter().enumerate() {
        if let Some(peer) = value.peer.as_ref().and_then(endpoint_ref) {
            if let (Ok(kind), Some(id)) = (EntityKind::try_from(peer.kind), peer.id) {
                let arm = if kind == EntityKind::Component {
                    "component"
                } else {
                    "external_system"
                };
                out.push(OutboundRef {
                    to_kind: Some(kind),
                    to_id: id,
                    from_field: format!("communications[{index}].peer.{arm}"),
                });
            }
        }
        if let Some(id) = value
            .message
            .as_ref()
            .and_then(|value| value.specification.as_ref())
            .and_then(|value| value.id.clone())
        {
            out.push(OutboundRef {
                to_kind: Some(EntityKind::Specification),
                to_id: id,
                from_field: format!("communications[{index}].message.specification"),
            });
        }
    }
}
pub(crate) fn retarget(
    values: &mut [pb::CommunicationDeclaration],
    kind: EntityKind,
    from: &pb::Id,
    to: &pb::Id,
    rewritten: &mut Vec<String>,
) {
    use pb::architectural_endpoint::Target;
    for (index, value) in values.iter_mut().enumerate() {
        let Some(target) = value.peer.as_mut().and_then(|peer| peer.target.as_mut()) else {
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
            rewritten.push(format!("communications[{index}].peer.{arm}"));
        }
    }
}
