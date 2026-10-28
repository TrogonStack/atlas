//! Stable identities for explicit connection occurrences within an entity.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
    sync::LazyLock,
};

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, ReflectMessage, Value};
use trogon_atlas_proto::{ConnectionRef, Entity, EntityRef};

use crate::{entity_id, entity_kind};

static POOL: LazyLock<DescriptorPool> = LazyLock::new(|| {
    DescriptorPool::decode(trogon_atlas_proto::FILE_DESCRIPTOR_SET)
        .unwrap_or_else(|error| panic!("invalid embedded descriptors: {error}"))
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(uuid::Uuid);

impl ConnectionId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ConnectionId {
    type Err = ConnectionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let id = uuid::Uuid::parse_str(value)
            .map_err(|_| ConnectionError::InvalidId(value.to_owned()))?;
        if id.is_nil() || id.to_string() != value {
            return Err(ConnectionError::InvalidId(value.to_owned()));
        }
        Ok(Self(id))
    }
}

impl From<ConnectionId> for trogon_atlas_proto::ConnectionId {
    fn from(id: ConnectionId) -> Self {
        Self {
            value: id.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionOccurrence {
    pub id: Option<ConnectionId>,
    pub path: String,
    pub target: Option<EntityRef>,
    signature: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConnectionError {
    #[error("connection ID must be a non-nil lowercase hyphenated UUID: {0:?}")]
    InvalidId(String),
    #[error("duplicate connection ID {id} at {first} and {second}")]
    DuplicateId {
        id: ConnectionId,
        first: String,
        second: String,
    },
    #[error("connection ID was removed from an existing occurrence at {0}")]
    StrippedId(String),
    #[error("stored connection at {0} has no connection ID")]
    MissingStoredId(String),
    #[error("connection {0} changed role or endpoint; use a new ID")]
    Retargeted(ConnectionId),
    #[error("connection reference does not identify this exact owner")]
    WrongOwner,
    #[error("connection reference has no ID")]
    MissingId,
    #[error("connection {0} is absent from this owner")]
    NotFound(ConnectionId),
    #[error("connection ID {0} is not active in this owner; omit IDs for new occurrences")]
    UnknownId(ConnectionId),
    #[error("connection ID {0} from the branch base was removed on the opposing side")]
    RemovedFromBranchBase(ConnectionId),
    #[error("specification identity error: {0}")]
    Member(String),
    #[error("catalog identity error: {0}")]
    Catalog(String),
    #[error("data port identity error: {0}")]
    Port(String),
    #[error("connection encoding error: {0}")]
    Encoding(String),
}

fn dynamic(entity: &Entity) -> Result<DynamicMessage, ConnectionError> {
    let descriptor = POOL
        .get_message_by_name("trogonatlas.eventmodel.v1alpha1.Entity")
        .ok_or_else(|| ConnectionError::Encoding("Entity descriptor missing".into()))?;
    DynamicMessage::decode(descriptor, entity.encode_to_vec().as_slice())
        .map_err(|error| ConnectionError::Encoding(error.to_string()))
}

fn wrapper_target(name: &str) -> Option<(&'static str, trogon_atlas_proto::EntityKind)> {
    use trogon_atlas_proto::EntityKind as K;
    match name {
        "trogonatlas.eventmodel.v1alpha1.EventEdge"
        | "trogonatlas.eventmodel.v1alpha1.Swimlane.Transition.Next" => Some(("event", K::Event)),
        "trogonatlas.eventmodel.v1alpha1.CommandEdge" => Some(("command", K::Command)),
        "trogonatlas.eventmodel.v1alpha1.ReadModelEdge"
        | "trogonatlas.eventmodel.v1alpha1.ReadModelAttachment" => {
            Some(("read_model", K::ReadModel))
        }
        "trogonatlas.eventmodel.v1alpha1.ProcessorEdge" => Some(("processor", K::Processor)),
        "trogonatlas.eventmodel.v1alpha1.UIEdge" => Some(("ui", K::Ui)),
        "trogonatlas.eventmodel.v1alpha1.PersonaEdge" => Some(("persona", K::Persona)),
        "trogonatlas.eventmodel.v1alpha1.UI.Transition" => Some(("to", K::Ui)),
        "trogonatlas.eventmodel.v1alpha1.ChannelBinding" => {
            Some(("observation", K::AutomationSlice))
        }
        _ => None,
    }
}

fn identity_field(message: &DynamicMessage) -> &'static str {
    if message.descriptor().full_name() == "trogonatlas.annotation.v1alpha1.ResourceGroup" { "group_id" } else { "connection_id" }
}

fn read_id(message: &DynamicMessage) -> Result<Option<ConnectionId>, ConnectionError> {
    if !message.has_field_by_name(identity_field(message)) {
        return Ok(None);
    }
    match message.get_field_by_name(identity_field(message)).as_deref() {
        Some(Value::Message(id)) => match id.get_field_by_name("value").as_deref() {
            Some(Value::String(value)) if value.is_empty() => Ok(None),
            Some(Value::String(value)) => value.parse().map(Some),
            _ => Err(ConnectionError::InvalidId(String::new())),
        },
        _ => Ok(None),
    }
}

fn anchor(message: &DynamicMessage, field: &str) -> Vec<u8> {
    let Some(value) = message.get_field_by_name(field) else {
        return Vec::new();
    };
    let Some(wrapper) = value.as_message() else {
        return Vec::new();
    };
    let Some((target, _)) = wrapper_target(wrapper.descriptor().full_name()) else {
        return Vec::new();
    };
    wrapper
        .get_field_by_name(target)
        .as_deref()
        .and_then(Value::as_message)
        .map(Message::encode_to_vec)
        .unwrap_or_default()
}

fn context_for(message: &DynamicMessage, field: &str) -> Vec<u8> {
    let descriptor = message.descriptor();
    let fields: &[&str] = match (descriptor.full_name(), field) {
        ("trogonatlas.eventmodel.v1alpha1.CommandSlice", "emitted_events") => &["command"],
        ("trogonatlas.eventmodel.v1alpha1.CommandSlice", "command" | "persona")
        | ("trogonatlas.eventmodel.v1alpha1.UiSlice", "persona" | "source_read_models")
        | ("trogonatlas.eventmodel.v1alpha1.StoryboardEntry.HumanObserver", "persona") => &["ui"],
        ("trogonatlas.eventmodel.v1alpha1.CommandSlice", "ui") => &["persona", "command"],
        ("trogonatlas.eventmodel.v1alpha1.ReadModelSlice", "source_events") => &["read_model"],
        (
            "trogonatlas.eventmodel.v1alpha1.AutomationSlice",
            "source_read_models" | "emitted_command",
        ) => &["processor"],
        (
            "trogonatlas.eventmodel.v1alpha1.UiSlice"
            | "trogonatlas.eventmodel.v1alpha1.StoryboardEntry.HumanObserver",
            "ui",
        ) => &["persona"],
        ("trogonatlas.eventmodel.v1alpha1.StoryboardEntry", "human" | "automation") => {
            &["read_model"]
        }
        _ => &[],
    };
    let mut bytes = Vec::new();
    for field in fields {
        let target = anchor(message, field);
        bytes.extend_from_slice(&(target.len() as u64).to_le_bytes());
        bytes.extend(target);
    }
    bytes
}

fn visit(
    message: &mut DynamicMessage,
    path: &str,
    role: &str,
    predecessor: &[u8],
    callback: &mut impl FnMut(&mut DynamicMessage, ConnectionOccurrence) -> Result<(), ConnectionError>,
) -> Result<(), ConnectionError> {
    let descriptor = message.descriptor();
    if descriptor.full_name() == "google.protobuf.Any" {
        let mut any = prost_types::Any::decode(message.encode_to_vec().as_slice())
            .map_err(|e| ConnectionError::Encoding(e.to_string()))?;
        if let Some(name) = crate::agent_metadata::known_type(&any.type_url) {
            let descriptor = POOL.get_message_by_name(name).ok_or_else(|| ConnectionError::Encoding("annotation descriptor missing".into()))?;
            let mut payload = DynamicMessage::decode(descriptor, any.value.as_slice()).map_err(|e| ConnectionError::Encoding(e.to_string()))?;
            if name == crate::agent_metadata::GROUPS {
                visit(&mut payload, path, role, predecessor, callback)?;
                any.value = payload.encode_to_vec();
                *message = DynamicMessage::decode(message.descriptor(), any.encode_to_vec().as_slice()).map_err(|e| ConnectionError::Encoding(e.to_string()))?;
            }
        }
        return Ok(());
    }
    if descriptor.full_name() == "trogonatlas.annotation.v1alpha1.ResourceGroup" {
        callback(message, ConnectionOccurrence { id: read_id(message)?, path: path.to_owned(), target: None, signature: b"resource-group".to_vec() })?;
    }
    if descriptor.get_field_by_name("connection_id").is_some()
        && wrapper_target(descriptor.full_name()).is_none()
    {
        let mut endpoint = message.clone();
        for name in ["connection_id", "doc", "label", "condition"] {
            if let Some(field) = descriptor.get_field_by_name(name) {
                endpoint.clear_field(&field);
            }
        }
        let mut signature = format!("{role}:{}:", descriptor.full_name()).into_bytes();
        signature.extend_from_slice(&(predecessor.len() as u64).to_le_bytes());
        signature.extend_from_slice(predecessor);
        signature.extend(endpoint.encode_to_vec());
        let occurrence = ConnectionOccurrence {
            id: read_id(message)?,
            path: path.to_owned(),
            target: if descriptor.full_name() == "trogonatlas.annotation.v1alpha1.ResourceGroupItem" {
                message.get_field_by_name("target").as_deref().and_then(Value::as_message).map(Message::encode_to_vec).map(|bytes| EntityRef::decode(bytes.as_slice())).transpose().map_err(|e| ConnectionError::Encoding(e.to_string()))?
            } else if descriptor.full_name()
                == "trogonatlas.eventmodel.v1alpha1.ArchitecturalRelationship"
            {
                message
                    .get_field_by_name("target")
                    .as_deref()
                    .and_then(Value::as_message)
                    .map(Message::encode_to_vec)
                    .and_then(|bytes| {
                        trogon_atlas_proto::ArchitecturalEndpoint::decode(bytes.as_slice()).ok()
                    })
                    .as_ref()
                    .and_then(crate::architectural_relationships::endpoint_ref)
            } else if descriptor.full_name()
                == "trogonatlas.eventmodel.v1alpha1.CommunicationDeclaration"
            {
                message
                    .get_field_by_name("peer")
                    .as_deref()
                    .and_then(Value::as_message)
                    .map(Message::encode_to_vec)
                    .and_then(|bytes| {
                        trogon_atlas_proto::ArchitecturalEndpoint::decode(bytes.as_slice()).ok()
                    })
                    .as_ref()
                    .and_then(crate::architectural_relationships::endpoint_ref)
            } else {
                None
            },
            signature,
        };
        callback(message, occurrence)?;
        return Ok(());
    }
    if let Some((field, kind)) = wrapper_target(descriptor.full_name()) {
        let endpoint = message.get_field_by_name(field);
        let target_message = endpoint.as_deref().and_then(Value::as_message);
        let reference_owner = target_message
            .filter(|target| {
                target.descriptor().full_name() == "trogonatlas.eventmodel.v1alpha1.ConnectionRef"
            })
            .and_then(|target| target.get_field_by_name("owner"))
            .and_then(|value| value.as_message().cloned());
        let target_id = reference_owner
            .as_ref()
            .or(target_message)
            .and_then(|target| target.get_field_by_name("id"))
            .and_then(|value| value.as_message().map(Message::encode_to_vec));
        let target = target_id
            .as_deref()
            .map(trogon_atlas_proto::Id::decode)
            .transpose()
            .map_err(|error| ConnectionError::Encoding(error.to_string()))?
            .map(|id| EntityRef {
                kind: kind as i32,
                id: Some(id),
            });
        let mut signature = format!("{role}:{}:", descriptor.full_name()).into_bytes();
        signature.extend_from_slice(&(predecessor.len() as u64).to_le_bytes());
        signature.extend_from_slice(predecessor);
        if let Some(target) = target_message {
            signature.extend(target.encode_to_vec());
        }
        let occurrence = ConnectionOccurrence {
            id: read_id(message)?,
            path: path.to_owned(),
            target,
            signature,
        };
        callback(message, occurrence)?;
        return Ok(());
    }
    let after = if descriptor.full_name() == "trogonatlas.eventmodel.v1alpha1.Swimlane.Transition" {
        message
            .get_field_by_name("after")
            .as_deref()
            .and_then(Value::as_message)
            .map(Message::encode_to_vec)
            .unwrap_or_default()
    } else {
        predecessor.to_vec()
    };
    let mut after = after;
    for identity_field in ["member_id", "port_id", "group_id"] {
        if let Some(value) = message.get_field_by_name(identity_field) {
            if let Some(id) = value.as_message() {
                let encoded = id.encode_to_vec();
                after.extend_from_slice(&(encoded.len() as u64).to_le_bytes());
                after.extend(encoded);
            }
        }
    }
    let fields: Vec<_> = descriptor
        .fields()
        .filter(|field| message.has_field(field))
        .collect();
    for field in fields {
        let next_path = if path.is_empty() {
            field.name().to_owned()
        } else {
            format!("{path}.{}", field.name())
        };
        let next_role = if role.is_empty() {
            field.name().to_owned()
        } else {
            format!("{role}.{}", field.name())
        };
        let context = context_for(message, field.name());
        let mut inherited = after.clone();
        inherited.extend(context);
        match message.get_field_mut(&field) {
            Value::Message(child) => visit(child, &next_path, &next_role, &inherited, callback)?,
            Value::List(values) => {
                for (index, value) in values.iter_mut().enumerate() {
                    if let Value::Message(child) = value {
                        visit(
                            child,
                            &format!("{next_path}[{index}]"),
                            &next_role,
                            &inherited,
                            callback,
                        )?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Request fresh identities for occurrences whose role or either declared endpoint changed.
pub fn renew_retargeted_connection_ids(
    previous: &Entity,
    proposed: &mut Entity,
) -> Result<usize, ConnectionError> {
    validate_stored_connection_ids(previous)?;
    validate_connection_ids(proposed)?;
    let old: HashMap<_, _> = connection_occurrences(previous)?
        .into_iter()
        .filter_map(|item| item.id.map(|id| (id, item.signature)))
        .collect();
    let mut normalized = dynamic(proposed)?;
    let mut renewed = 0;
    visit(&mut normalized, "", "", &[], &mut |message, occurrence| {
        if occurrence
            .id
            .and_then(|id| old.get(&id))
            .is_some_and(|signature| *signature != occurrence.signature)
        {
            let field = message
                .descriptor()
                .get_field_by_name(identity_field(message))
                .ok_or_else(|| {
                    ConnectionError::Encoding("connection ID descriptor missing".into())
                })?;
            message.clear_field(&field);
            renewed += 1;
        }
        Ok(())
    })?;
    *proposed = Entity::decode(normalized.encode_to_vec().as_slice())
        .map_err(|error| ConnectionError::Encoding(error.to_string()))?;
    Ok(renewed)
}

pub(crate) fn renew_connection_ids(
    entity: &mut Entity,
    selected: &HashSet<ConnectionId>,
) -> Result<usize, ConnectionError> {
    validate_stored_connection_ids(entity)?;
    let mut normalized = dynamic(entity)?;
    visit(&mut normalized, "", "", &[], &mut |message, occurrence| {
        if occurrence.id.is_some_and(|id| selected.contains(&id)) {
            let field = message
                .descriptor()
                .get_field_by_name(identity_field(message))
                .ok_or_else(|| {
                    ConnectionError::Encoding("connection ID descriptor missing".into())
                })?;
            message.clear_field(&field);
        }
        Ok(())
    })?;
    *entity = Entity::decode(normalized.encode_to_vec().as_slice())
        .map_err(|error| ConnectionError::Encoding(error.to_string()))?;
    assign_connection_ids(entity)
}

pub fn connection_occurrences(
    entity: &Entity,
) -> Result<Vec<ConnectionOccurrence>, ConnectionError> {
    let mut output = Vec::new();
    visit(&mut dynamic(entity)?, "", "", &[], &mut |_, occurrence| {
        output.push(occurrence);
        Ok(())
    })?;
    Ok(output)
}

/// Compare an id-less manifest without authorizing any identity-stripping write.
pub fn equivalent_with_unassigned_ids(
    current: &Entity,
    desired: &Entity,
) -> Result<bool, ConnectionError> {
    validate_stored_connection_ids(current)?;
    if connection_occurrences(desired)?
        .iter()
        .any(|item| item.id.is_some())
    {
        return Ok(false);
    }
    let mut comparison = dynamic(current)?;
    visit(&mut comparison, "", "", &[], &mut |message, _| {
        let field = message
            .descriptor()
            .get_field_by_name(identity_field(message))
            .ok_or_else(|| ConnectionError::Encoding("connection ID descriptor missing".into()))?;
        message.clear_field(&field);
        Ok(())
    })?;
    let comparison = Entity::decode(comparison.encode_to_vec().as_slice())
        .map_err(|error| ConnectionError::Encoding(error.to_string()))?;
    Ok(crate::semantically_equal(&comparison, desired))
}

pub fn validate_connection_ids(entity: &Entity) -> Result<(), ConnectionError> {
    let mut ids = HashMap::new();
    for occurrence in connection_occurrences(entity)? {
        if let Some(id) = occurrence.id {
            if let Some(first) = ids.insert(id, occurrence.path.clone()) {
                return Err(ConnectionError::DuplicateId {
                    id,
                    first,
                    second: occurrence.path,
                });
            }
        }
    }
    Ok(())
}

/// Require complete occurrence identities in an already persisted owner.
pub fn validate_stored_connection_ids(entity: &Entity) -> Result<(), ConnectionError> {
    validate_connection_ids(entity)?;
    for occurrence in connection_occurrences(entity)? {
        if occurrence.id.is_none() {
            return Err(ConnectionError::MissingStoredId(occurrence.path));
        }
    }
    Ok(())
}

/// Assign identities to freshly authored occurrences without changing existing IDs.
pub fn assign_connection_ids(entity: &mut Entity) -> Result<usize, ConnectionError> {
    validate_connection_ids(entity)?;
    let mut normalized = dynamic(entity)?;
    let mut assigned = 0;
    visit(&mut normalized, "", "", &[], &mut |message, occurrence| {
        if occurrence.id.is_none() {
            let field = message
                .descriptor()
                .get_field_by_name(identity_field(message))
                .ok_or_else(|| {
                    ConnectionError::Encoding("connection ID descriptor missing".into())
                })?;
            let prost_reflect::Kind::Message(descriptor) = field.kind() else {
                return Err(ConnectionError::Encoding(
                    "connection ID is not a message".into(),
                ));
            };
            let mut id = DynamicMessage::new(descriptor);
            id.set_field_by_name("value", Value::String(ConnectionId::new().to_string()));
            message.set_field(&field, Value::Message(id));
            assigned += 1;
        }
        Ok(())
    })?;
    *entity = Entity::decode(normalized.encode_to_vec().as_slice())
        .map_err(|error| ConnectionError::Encoding(error.to_string()))?;
    Ok(assigned)
}

/// Check identity continuity against the current owner before minting IDs for additions.
pub fn reconcile_connection_ids(
    previous: &Entity,
    proposed: &mut Entity,
) -> Result<usize, ConnectionError> {
    reconcile(previous, proposed, false)
}

pub(crate) fn reconcile_branch_connection_ids(
    previous: &Entity,
    proposed: &mut Entity,
) -> Result<usize, ConnectionError> {
    reconcile(previous, proposed, true)
}

fn reconcile(
    previous: &Entity,
    proposed: &mut Entity,
    trusted_additions: bool,
) -> Result<usize, ConnectionError> {
    validate_stored_connection_ids(previous)?;
    validate_connection_ids(proposed)?;
    let old = connection_occurrences(previous)?;
    let old_ids: HashMap<_, _> = old
        .iter()
        .filter_map(|item| item.id.map(|id| (id, &item.signature)))
        .collect();
    let proposed_occurrences = connection_occurrences(proposed)?;
    let retained_ids: HashSet<_> = proposed_occurrences
        .iter()
        .filter_map(|item| item.id)
        .collect();
    for occurrence in proposed_occurrences {
        match occurrence.id {
            Some(id) if !trusted_additions && !old_ids.contains_key(&id) => {
                return Err(ConnectionError::UnknownId(id));
            }
            Some(id)
                if old_ids
                    .get(&id)
                    .is_some_and(|signature| **signature != occurrence.signature) =>
            {
                return Err(ConnectionError::Retargeted(id));
            }
            None if old.iter().any(|item| {
                item.id.is_some_and(|id| !retained_ids.contains(&id))
                    && item.signature == occurrence.signature
            }) =>
            {
                return Err(ConnectionError::StrippedId(occurrence.path));
            }
            _ => {}
        }
    }
    assign_connection_ids(proposed)
}

pub fn resolve_connection(
    entity: &Entity,
    reference: &ConnectionRef,
) -> Result<ConnectionOccurrence, ConnectionError> {
    let owner = EntityRef {
        kind: entity_kind(entity).map_or(0, |kind| kind as i32),
        id: entity_id(entity).cloned(),
    };
    if owner.id.is_none() || reference.owner.as_ref() != Some(&owner) {
        return Err(ConnectionError::WrongOwner);
    }
    if reference.owner_uid.is_empty()
        || entity
            .system
            .as_ref()
            .is_none_or(|system| system.uid != reference.owner_uid)
    {
        return Err(ConnectionError::WrongOwner);
    }
    let id: ConnectionId = reference
        .id
        .as_ref()
        .ok_or(ConnectionError::MissingId)?
        .value
        .parse()?;
    validate_stored_connection_ids(entity)?;
    connection_occurrences(entity)?
        .into_iter()
        .find(|item| item.id == Some(id))
        .ok_or(ConnectionError::NotFound(id))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use trogon_atlas_proto::{
        entity, CommandRef, CommandSlice, EventEdge, EventRef, Id, SystemMeta,
    };

    use super::*;

    fn id(slug: &str) -> Id {
        Id {
            namespace: "example".into(),
            slug: slug.into(),
            version: 1,
        }
    }

    fn fixture() -> Entity {
        Entity {
            kind: Some(entity::Kind::CommandSlice(CommandSlice {
                id: Some(id("place-order")),
                command: Some(trogon_atlas_proto::CommandEdge {
                    command: Some(CommandRef {
                        id: Some(id("place-order")),
                    }),
                    ..Default::default()
                }),
                emitted_events: vec![
                    EventEdge {
                        event: Some(EventRef {
                            id: Some(id("order-placed"))
                        }),
                        ..Default::default()
                    };
                    2
                ],
                ..Default::default()
            })),
            system: Some(SystemMeta {
                uid: "owner-incarnation".into(),
                ..Default::default()
            }),
        }
    }

    fn slice(entity: &mut Entity) -> &mut CommandSlice {
        let Some(entity::Kind::CommandSlice(slice)) = entity.kind.as_mut() else {
            panic!("expected command slice")
        };
        slice
    }

    fn reference(entity: &Entity, occurrence: &ConnectionOccurrence) -> ConnectionRef {
        ConnectionRef {
            owner: Some(EntityRef {
                kind: entity_kind(entity).unwrap() as i32,
                id: entity_id(entity).cloned(),
            }),
            id: occurrence.id.map(Into::into),
            owner_uid: entity.system.as_ref().unwrap().uid.clone(),
        }
    }

    #[test]
    fn repeated_endpoints_remain_distinct_through_reorder_and_wire_roundtrip() {
        let mut entity = fixture();
        assert_eq!(assign_connection_ids(&mut entity).unwrap(), 3);
        let before = connection_occurrences(&entity).unwrap();
        assert_ne!(before[1].id, before[2].id);
        let old_ref = reference(&entity, &before[1]);
        let mut reordered = entity.clone();
        slice(&mut reordered).emitted_events.reverse();
        slice(&mut reordered).emitted_events[1].doc = "new annotation".into();
        assert_eq!(
            reconcile_connection_ids(&entity, &mut reordered).unwrap(),
            0
        );
        let decoded = Entity::decode(reordered.encode_to_vec().as_slice()).unwrap();
        let resolved = resolve_connection(&decoded, &old_ref).unwrap();
        assert_eq!(resolved.path, "command_slice.emitted_events[1]");
    }

    #[test]
    fn removal_and_wrong_owner_do_not_resolve() {
        let mut entity = fixture();
        assign_connection_ids(&mut entity).unwrap();
        let occurrence = connection_occurrences(&entity).unwrap().remove(1);
        let mut reference = reference(&entity, &occurrence);
        let mut removed = entity.clone();
        slice(&mut removed).emitted_events.remove(0);
        assert_eq!(
            resolve_connection(&removed, &reference),
            Err(ConnectionError::NotFound(occurrence.id.unwrap()))
        );
        reference
            .owner
            .as_mut()
            .unwrap()
            .id
            .as_mut()
            .unwrap()
            .version += 1;
        assert_eq!(
            resolve_connection(&entity, &reference),
            Err(ConnectionError::WrongOwner)
        );
    }

    #[test]
    fn recreated_owner_does_not_alias_old_reference() {
        let mut entity = fixture();
        assign_connection_ids(&mut entity).unwrap();
        let old_ref = reference(&entity, &connection_occurrences(&entity).unwrap()[0]);
        entity.system.as_mut().unwrap().uid = "different-incarnation".into();
        assert_eq!(
            resolve_connection(&entity, &old_ref),
            Err(ConnectionError::WrongOwner)
        );
    }

    #[test]
    fn retained_id_rejects_target_or_source_retarget_and_renewal_repairs_both() {
        let mut entity = fixture();
        assign_connection_ids(&mut entity).unwrap();
        let mut changed = entity.clone();
        slice(&mut changed).emitted_events[0]
            .event
            .as_mut()
            .unwrap()
            .id = Some(id("order-rejected"));
        assert!(matches!(
            reconcile_connection_ids(&entity, &mut changed),
            Err(ConnectionError::Retargeted(_))
        ));
        assert_eq!(
            renew_retargeted_connection_ids(&entity, &mut changed).unwrap(),
            1
        );
        assert!(slice(&mut changed).emitted_events[0]
            .connection_id
            .is_none());
        assert_eq!(
            slice(&mut changed).emitted_events[1].connection_id,
            slice(&mut entity).emitted_events[1].connection_id
        );
        reconcile_connection_ids(&entity, &mut changed).unwrap();
        assert_ne!(
            slice(&mut changed).emitted_events[0].connection_id,
            slice(&mut entity).emitted_events[0].connection_id
        );
        let mut changed_source = entity.clone();
        slice(&mut changed_source)
            .command
            .as_mut()
            .unwrap()
            .command
            .as_mut()
            .unwrap()
            .id = Some(id("cancel-order"));
        assert!(matches!(
            reconcile_connection_ids(&entity, &mut changed_source),
            Err(ConnectionError::Retargeted(_))
        ));
        assert_eq!(
            renew_retargeted_connection_ids(&entity, &mut changed_source).unwrap(),
            3
        );
        assert!(connection_occurrences(&changed_source)
            .unwrap()
            .iter()
            .all(|occurrence| occurrence.id.is_none()));
        reconcile_connection_ids(&entity, &mut changed_source).unwrap();
    }

    #[test]
    fn duplicate_invalid_and_stripped_ids_are_rejected_without_mutation() {
        let mut entity = fixture();
        assign_connection_ids(&mut entity).unwrap();
        let mut duplicate = entity.clone();
        slice(&mut duplicate).emitted_events[1].connection_id = slice(&mut duplicate)
            .emitted_events[0]
            .connection_id
            .clone();
        assert!(matches!(
            assign_connection_ids(&mut duplicate),
            Err(ConnectionError::DuplicateId { .. })
        ));
        let mut invalid = entity.clone();
        slice(&mut invalid).emitted_events[0]
            .connection_id
            .as_mut()
            .unwrap()
            .value = "invalid".into();
        assert!(matches!(
            assign_connection_ids(&mut invalid),
            Err(ConnectionError::InvalidId(_))
        ));
        let mut stripped = entity.clone();
        slice(&mut stripped).emitted_events[0].connection_id = None;
        let before = stripped.clone();
        assert!(matches!(
            reconcile_connection_ids(&entity, &mut stripped),
            Err(ConnectionError::StrippedId(_))
        ));
        assert_eq!(stripped, before);
    }

    #[test]
    fn fresh_assignment_is_idempotent() {
        let mut proposed = fixture();
        assert_eq!(assign_connection_ids(&mut proposed).unwrap(), 3);
        assert_eq!(assign_connection_ids(&mut proposed).unwrap(), 0);
        assert!(validate_stored_connection_ids(&proposed).is_ok());
        assert!("00000000-0000-0000-0000-000000000000"
            .parse::<ConnectionId>()
            .is_err());
        assert!("019a0590-a74e-7abc-8123-a12345678901"
            .to_uppercase()
            .parse::<ConnectionId>()
            .is_err());
    }

    #[test]
    fn idless_stored_connections_reject_update_and_manifest_comparison() {
        let unsupported = Entity::decode(fixture().encode_to_vec().as_slice()).unwrap();
        let mut proposed = unsupported.clone();
        let expected = ConnectionError::MissingStoredId("command_slice.command".into());
        assert_eq!(validate_stored_connection_ids(&unsupported), Err(expected));
        assert!(matches!(
            reconcile_connection_ids(&unsupported, &mut proposed),
            Err(ConnectionError::MissingStoredId(_))
        ));
        assert_eq!(proposed, unsupported);
        assert!(matches!(
            equivalent_with_unassigned_ids(&unsupported, &proposed),
            Err(ConnectionError::MissingStoredId(_))
        ));
        assert!(matches!(
            renew_retargeted_connection_ids(&unsupported, &mut proposed),
            Err(ConnectionError::MissingStoredId(_))
        ));
    }

    #[test]
    fn identified_connection_cannot_resolve_from_incomplete_stored_owner() {
        let mut stored = fixture();
        assign_connection_ids(&mut stored).unwrap();
        let occurrence = connection_occurrences(&stored).unwrap().remove(1);
        let reference = reference(&stored, &occurrence);
        slice(&mut stored).emitted_events[1].connection_id = None;
        assert_eq!(
            resolve_connection(&stored, &reference),
            Err(ConnectionError::MissingStoredId(
                "command_slice.emitted_events[1]".into()
            ))
        );
    }

    #[test]
    fn adding_repeated_endpoint_mints_an_independent_id() {
        let mut previous = fixture();
        assign_connection_ids(&mut previous).unwrap();
        let mut proposed = previous.clone();
        slice(&mut proposed).emitted_events.push(EventEdge {
            event: Some(EventRef {
                id: Some(id("order-placed")),
            }),
            ..Default::default()
        });
        assert_eq!(
            reconcile_connection_ids(&previous, &mut proposed).unwrap(),
            1
        );
        assert_eq!(connection_occurrences(&proposed).unwrap().len(), 4);
    }

    #[test]
    fn all_typed_wrappers_are_enumerated_with_consumer_context() {
        use trogon_atlas_proto::{
            AutomationSlice, PersonaEdge, PersonaRef, ProcessorEdge, ProcessorRef, ReadModelEdge,
            ReadModelRef, UiEdge, UiRef,
        };
        let mut human = fixture();
        slice(&mut human).persona = Some(PersonaEdge {
            persona: Some(PersonaRef {
                id: Some(id("customer")),
            }),
            ..Default::default()
        });
        slice(&mut human).ui = Some(UiEdge {
            ui: Some(UiRef {
                id: Some(id("checkout")),
            }),
            ..Default::default()
        });
        assert_eq!(assign_connection_ids(&mut human).unwrap(), 5);
        let mut automation = Entity {
            kind: Some(entity::Kind::AutomationSlice(AutomationSlice {
                source_read_models: vec![ReadModelEdge {
                    read_model: Some(ReadModelRef {
                        id: Some(id("pending-orders")),
                    }),
                    ..Default::default()
                }],
                processor: Some(ProcessorEdge {
                    processor: Some(ProcessorRef {
                        id: Some(id("dispatch")),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(assign_connection_ids(&mut automation).unwrap(), 2);
        let targets: Vec<_> = connection_occurrences(&human)
            .unwrap()
            .into_iter()
            .chain(connection_occurrences(&automation).unwrap())
            .map(|item| item.target.unwrap().kind)
            .collect();
        for kind in [
            trogon_atlas_proto::EntityKind::Event,
            trogon_atlas_proto::EntityKind::Command,
            trogon_atlas_proto::EntityKind::ReadModel,
            trogon_atlas_proto::EntityKind::Processor,
            trogon_atlas_proto::EntityKind::Ui,
            trogon_atlas_proto::EntityKind::Persona,
        ] {
            assert!(targets.contains(&(kind as i32)), "missing {kind:?}");
        }
        let mut changed = automation.clone();
        let Some(entity::Kind::AutomationSlice(slice)) = changed.kind.as_mut() else {
            panic!("automation")
        };
        slice
            .processor
            .as_mut()
            .unwrap()
            .processor
            .as_mut()
            .unwrap()
            .id = Some(id("other-dispatch"));
        assert_eq!(
            renew_retargeted_connection_ids(&automation, &mut changed).unwrap(),
            2
        );
        reconcile_connection_ids(&automation, &mut changed).unwrap();
    }

    #[test]
    fn lifecycle_source_and_navigation_destination_are_identity_endpoints() {
        use trogon_atlas_proto::{swimlane, ui, Swimlane, Ui, UiRef};
        let mut lifecycle = Entity {
            kind: Some(entity::Kind::Swimlane(Swimlane {
                transitions: vec![swimlane::Transition {
                    after: Some(EventRef {
                        id: Some(id("created")),
                    }),
                    next: vec![swimlane::transition::Next {
                        event: Some(EventRef {
                            id: Some(id("completed")),
                        }),
                        ..Default::default()
                    }],
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(assign_connection_ids(&mut lifecycle).unwrap(), 1);
        let mut changed = lifecycle.clone();
        let Some(entity::Kind::Swimlane(lane)) = changed.kind.as_mut() else {
            panic!("lane")
        };
        lane.transitions[0].after.as_mut().unwrap().id = Some(id("pending"));
        assert!(matches!(
            reconcile_connection_ids(&lifecycle, &mut changed),
            Err(ConnectionError::Retargeted(_))
        ));
        let mut navigation = Entity {
            kind: Some(entity::Kind::Ui(Ui {
                transitions: vec![ui::Transition {
                    to: Some(UiRef {
                        id: Some(id("details")),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        assert_eq!(assign_connection_ids(&mut navigation).unwrap(), 1);
        let mut changed = navigation.clone();
        let Some(entity::Kind::Ui(surface)) = changed.kind.as_mut() else {
            panic!("ui")
        };
        surface.transitions[0].to.as_mut().unwrap().id = Some(id("home"));
        assert!(matches!(
            reconcile_connection_ids(&navigation, &mut changed),
            Err(ConnectionError::Retargeted(_))
        ));
    }
}
