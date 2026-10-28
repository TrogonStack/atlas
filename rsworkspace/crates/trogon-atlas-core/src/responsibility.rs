use std::collections::HashSet;

use prost::Message;
use trogon_atlas_proto::{
    self as pb, entity, responsibility_subject::Selection, Entity, EntityKind,
};

use crate::{connections, data_product, entity_id, entity_kind, specification};

pub const MAX_CONTACTS: usize = 16;
pub const MAX_MEMBERS: usize = 1_024;
pub const MAX_RESPONSIBILITIES: usize = 1_024;
pub const MAX_ASSOCIATIONS: usize = 2_048;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;
pub const MAX_DISCOVERY_ROWS: usize = 5_000;
pub const MAX_EXAMINED_OCCURRENCES: usize = 10_000;
pub const MAX_OUTPUT_RECORDS: usize = 4_096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ResponsibilityError(pub String);

type Result<T> = std::result::Result<T, ResponsibilityError>;

fn require(valid: bool, reason: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(ResponsibilityError(reason.into()))
    }
}
fn text(value: &str, limit: usize) -> Result<()> {
    require(
        !value.trim().is_empty() && value.len() <= limit,
        "directory text is blank or exceeds its byte limit",
    )
}
fn exact(id: Option<&pb::Id>) -> Result<()> {
    let id = id.ok_or_else(|| ResponsibilityError("exact entity ID required".into()))?;
    require(
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug),
        "invalid exact entity ID",
    )
}
fn provider(value: Option<&pb::DirectoryProvider>) -> Result<()> {
    let value = &value
        .ok_or_else(|| ResponsibilityError("directory provider required".into()))?
        .value;
    require(
        value.len() <= 64
            && value.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'),
        "invalid directory provider",
    )
}
fn key(value: Option<&pb::OpaqueDirectoryKey>) -> Result<()> {
    let value = &value
        .ok_or_else(|| ResponsibilityError("directory key required".into()))?
        .value;
    text(value, 512)?;
    require(
        !value.chars().any(char::is_control),
        "control character in directory key",
    )
}
fn uid(value: &str) -> Result<()> {
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|_| ResponsibilityError("invalid owner UID".into()))?;
    require(
        !parsed.is_nil() && parsed.to_string() == value,
        "invalid owner UID",
    )
}

pub fn validate_contact(value: &pb::ContactPoint) -> Result<()> {
    if let Some(label) = &value.label {
        text(label, 128)?;
    }
    match &value.channel {
        Some(pb::contact_point::Channel::Email(email)) => {
            let bytes = email.value.as_bytes();
            let parts: Vec<_> = email.value.split('@').collect();
            require(
                bytes.len() <= 254
                    && bytes.is_ascii()
                    && !bytes
                        .iter()
                        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
                    && parts.len() == 2
                    && parts.iter().all(|p| !p.is_empty()),
                "invalid catalog email",
            )
        }
        Some(pb::contact_point::Channel::Web(web)) => {
            require(
                web.value.len() <= 2_048
                    && !web
                        .value
                        .chars()
                        .any(|c| c.is_control() || c.is_whitespace()),
                "invalid contact URL",
            )?;
            let parsed = url::Url::parse(&web.value)
                .map_err(|_| ResponsibilityError("invalid contact URL".into()))?;
            let authority = web
                .value
                .split_once("://")
                .map(|(_, s)| s.split(['/', '?', '#']).next().unwrap_or_default());
            require(
                parsed.scheme() == "https"
                    && parsed.host_str().is_some()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && authority.is_some_and(|s| !s.contains('@')),
                "contact URL requires HTTPS host and forbids user information",
            )
        }
        Some(pb::contact_point::Channel::Chat(chat)) => {
            provider(chat.service.as_ref())?;
            key(chat.handle.as_ref())
        }
        None => Err(ResponsibilityError("contact channel required".into())),
    }
}

#[must_use]
pub fn connection_subject_owner(kind: EntityKind) -> bool {
    matches!(
        kind,
        EntityKind::CommandSlice
            | EntityKind::ReadModelSlice
            | EntityKind::AutomationSlice
            | EntityKind::UiSlice
            | EntityKind::Storyboard
            | EntityKind::Swimlane
            | EntityKind::Ui
            | EntityKind::Channel
            | EntityKind::DataStore
            | EntityKind::Specification
            | EntityKind::DataProduct
            | EntityKind::Component
            | EntityKind::ExternalSystem
            | EntityKind::SystemGroup
            | EntityKind::Domain
    )
}

pub fn subject_owner(subject: &pb::ResponsibilitySubject) -> Result<pb::EntityRef> {
    let owner = match subject.selection.as_ref() {
        Some(Selection::Entity(reference)) => reference.clone(),
        Some(Selection::Connection(reference)) => reference
            .owner
            .clone()
            .ok_or_else(|| ResponsibilityError("connection owner required".into()))?,
        Some(Selection::Member(reference)) => pb::EntityRef {
            kind: EntityKind::Specification as i32,
            id: reference.specification.as_ref().and_then(|r| r.id.clone()),
        },
        Some(Selection::Port(reference)) => pb::EntityRef {
            kind: EntityKind::DataProduct as i32,
            id: reference.product.as_ref().and_then(|r| r.id.clone()),
        },
        None => {
            return Err(ResponsibilityError(
                "responsibility subject required".into(),
            ));
        }
    };
    exact(owner.id.as_ref())?;
    require(
        EntityKind::try_from(owner.kind).is_ok_and(|k| k != EntityKind::Unspecified),
        "unknown responsibility entity kind",
    )?;
    Ok(owner)
}

pub fn validate_subject(subject: &pb::ResponsibilitySubject) -> Result<()> {
    let owner = subject_owner(subject)?;
    match subject.selection.as_ref() {
        Some(Selection::Entity(_)) => Ok(()),
        Some(Selection::Connection(reference)) => {
            require(
                EntityKind::try_from(owner.kind).is_ok_and(connection_subject_owner),
                "unsupported responsibility connection owner",
            )?;
            uid(&reference.owner_uid)?;
            reference
                .id
                .as_ref()
                .ok_or_else(|| ResponsibilityError("connection ID required".into()))?
                .value
                .parse::<connections::ConnectionId>()
                .map_err(|e| ResponsibilityError(e.to_string()))?;
            Ok(())
        }
        Some(Selection::Member(reference)) => {
            uid(&reference.owner_uid)?;
            require(
                matches!(
                    pb::ContractMemberKind::try_from(reference.kind),
                    Ok(pb::ContractMemberKind::Artifact
                        | pb::ContractMemberKind::Schema
                        | pb::ContractMemberKind::Message
                        | pb::ContractMemberKind::Operation)
                ),
                "invalid responsibility member kind",
            )?;
            reference
                .member_id
                .as_ref()
                .ok_or_else(|| ResponsibilityError("member ID required".into()))?
                .value
                .parse::<specification::MemberId>()
                .map_err(|e| ResponsibilityError(e.to_string()))?;
            if let Some(digest) = &reference.expected_digest {
                require(
                    reference.kind == pb::ContractMemberKind::Artifact as i32,
                    "only artifact may pin a digest",
                )?;
                require(
                    digest.sha256.len() == 64
                        && digest
                            .sha256
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "invalid artifact digest",
                )?;
            }
            Ok(())
        }
        Some(Selection::Port(reference)) => {
            uid(&reference.owner_uid)?;
            require(
                matches!(
                    pb::DataPortDirection::try_from(reference.direction),
                    Ok(pb::DataPortDirection::Input | pb::DataPortDirection::Output)
                ),
                "invalid responsibility port direction",
            )?;
            reference
                .port_id
                .as_ref()
                .ok_or_else(|| ResponsibilityError("port ID required".into()))?
                .value
                .parse::<data_product::DataPortId>()
                .map_err(|e| ResponsibilityError(e.to_string()))?;
            Ok(())
        }
        None => Err(ResponsibilityError(
            "responsibility subject required".into(),
        )),
    }
}

pub fn resolve_subject(subject: &pb::ResponsibilitySubject, owner: &Entity) -> Result<()> {
    validate_subject(subject)?;
    let selected = subject_owner(subject)?;
    require(
        entity_kind(owner).map(|k| k as i32) == Some(selected.kind)
            && entity_id(owner) == selected.id.as_ref(),
        "responsibility subject does not identify exact owner",
    )?;
    match subject.selection.as_ref() {
        Some(Selection::Entity(_)) => Ok(()),
        Some(Selection::Connection(reference)) => {
            if matches!(
                entity_kind(owner),
                Some(EntityKind::Component | EntityKind::ExternalSystem)
            ) {
                crate::architectural_relationships::resolve_connection_context(owner, reference)
                    .map(|_| ())
                    .map_err(|e| ResponsibilityError(e.to_string()))
            } else if matches!(
                entity_kind(owner),
                Some(EntityKind::SystemGroup | EntityKind::Domain)
            ) {
                crate::system_group::resolve_connection_context(owner, reference)
                    .map(|_| ())
                    .map_err(|e| ResponsibilityError(e.to_string()))
            } else {
                connections::resolve_connection(owner, reference)
                    .map(|_| ())
                    .map_err(|e| ResponsibilityError(e.to_string()))
            }
        }
        Some(Selection::Member(reference)) => specification::resolve_member(owner, reference)
            .map(|_| ())
            .map_err(|e| ResponsibilityError(e.to_string())),
        Some(Selection::Port(reference)) => data_product::resolve_port(owner, reference)
            .map(|_| ())
            .map_err(|e| ResponsibilityError(e.to_string())),
        None => Err(ResponsibilityError(
            "responsibility subject required".into(),
        )),
    }
}

#[must_use]
pub fn assignments(entity: &Entity) -> &[pb::Responsibility] {
    match &entity.kind {
        Some(entity::Kind::Team(v)) => &v.responsibilities,
        Some(entity::Kind::Person(v)) => &v.responsibilities,
        _ => &[],
    }
}

pub fn validate_values(entity: &Entity) -> Result<()> {
    let (title, contacts, members) = match &entity.kind {
        Some(entity::Kind::Team(v)) => {
            if let Some(purpose) = &v.purpose {
                require(!purpose.trim().is_empty(), "blank team purpose")?;
            }
            (v.title.as_ref(), &v.contacts, v.members.as_slice())
        }
        Some(entity::Kind::Person(v)) => {
            if let Some(role) = &v.role {
                text(&role.value, 512)?;
            }
            if let Some(pointer) = &v.directory_pointer {
                provider(pointer.provider.as_ref())?;
                key(pointer.key.as_ref())?;
            }
            (v.title.as_ref(), &v.contacts, &[][..])
        }
        _ => return Ok(()),
    };
    exact(entity_id(entity))?;
    text(
        &title
            .ok_or_else(|| ResponsibilityError("directory display name required".into()))?
            .value,
        256,
    )?;
    require(
        entity.encoded_len() <= MAX_ENCODED_BYTES,
        "directory entity encoded size limit exceeded",
    )?;
    require(contacts.len() <= MAX_CONTACTS, "contact limit exceeded")?;
    let mut seen = HashSet::new();
    for contact in contacts {
        validate_contact(contact)?;
        require(seen.insert(contact.encode_to_vec()), "duplicate contact")?;
    }
    require(members.len() <= MAX_MEMBERS, "membership limit exceeded")?;
    seen.clear();
    for member in members {
        let person = member
            .person
            .as_ref()
            .ok_or_else(|| ResponsibilityError("member Person required".into()))?;
        exact(person.id.as_ref())?;
        require(seen.insert(person.encode_to_vec()), "duplicate membership")?;
    }
    let assignments = assignments(entity);
    require(
        assignments.len() <= MAX_RESPONSIBILITIES
            && assignments.len() + members.len() <= MAX_ASSOCIATIONS,
        "responsibility association limit exceeded",
    )?;
    seen.clear();
    for assignment in assignments {
        require(
            matches!(
                pb::ResponsibilityKind::try_from(assignment.kind),
                Ok(pb::ResponsibilityKind::Owner
                    | pb::ResponsibilityKind::Maintainer
                    | pb::ResponsibilityKind::Steward
                    | pb::ResponsibilityKind::Contact)
            ),
            "invalid responsibility kind",
        )?;
        let subject = assignment
            .subject
            .as_ref()
            .ok_or_else(|| ResponsibilityError("responsibility subject required".into()))?;
        validate_subject(subject)?;
        if let Some(Selection::Entity(selected)) = subject.selection.as_ref() {
            require(
                !(entity_kind(entity).map(|k| k as i32) == Some(selected.kind)
                    && entity_id(entity) == selected.id.as_ref()),
                "redundant self responsibility",
            )?;
        }
        let mut fingerprint = assignment.kind.to_le_bytes().to_vec();
        fingerprint.extend(subject.encode_to_vec());
        require(seen.insert(fingerprint), "duplicate responsibility")?;
    }
    connections::validate_connection_ids(entity).map_err(|e| ResponsibilityError(e.to_string()))
}
