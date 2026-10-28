use trogon_atlas_proto::{self as pb, Entity, EntityKind, EntityRef, Id};

use crate::refs::OutboundRef;

pub const MAX_GROUP_MEMBERS: usize = 1_024;
pub const MAX_GROUP_RELATIONSHIPS: usize = 1_024;
pub const MAX_GROUP_ACTORS: usize = 1_024;
pub const MAX_DOMAIN_MEMBERS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct SystemGroupError(pub String);

type Result<T> = std::result::Result<T, SystemGroupError>;

#[must_use]
pub fn design_graph_kind() -> Option<EntityKind> {
    None
}

#[must_use]
pub fn group_member_ref(_member: &pb::SystemGroupMember) -> Option<EntityRef> {
    None
}

#[must_use]
pub fn domain_member_ref(_member: &pb::DomainMember) -> Option<EntityRef> {
    None
}

#[must_use]
pub fn actor_ref(_target: &pb::SystemActorTarget) -> Option<EntityRef> {
    None
}

pub const SYSTEM_GROUP_MEMBERSHIP_CYCLE: &str = "SYSTEM_GROUP_MEMBERSHIP_CYCLE";
pub const DOMAIN_MEMBERSHIP_CYCLE: &str = "DOMAIN_MEMBERSHIP_CYCLE";
pub const SYSTEM_GROUP_MEMBERSHIP_REPEAT: &str = "SYSTEM_GROUP_MEMBERSHIP_REPEAT";
pub const DOMAIN_MEMBERSHIP_REPEAT: &str = "DOMAIN_MEMBERSHIP_REPEAT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryHop {
    pub owner: EntityRef,
    pub owner_uid: String,
    pub connection_id: crate::connections::ConnectionId,
    pub target: EntityRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    pub rule: &'static str,
    pub path: Vec<AdvisoryHop>,
}

#[must_use]
pub fn advisories(_owners: &[Entity]) -> Vec<Advisory> {
    Vec::new()
}

pub fn validate_values(_entity: &Entity) -> Result<()> {
    Ok(())
}

pub fn resolve_connection_context(
    _owner: &Entity,
    _reference: &pb::ConnectionRef,
) -> Result<Option<EntityRef>> {
    Ok(None)
}

pub(crate) fn outbound_group(_group: &pb::SystemGroup, _out: &mut Vec<OutboundRef>) {}

pub(crate) fn outbound_domain(_members: &[pb::DomainMembership], _out: &mut Vec<OutboundRef>) {}

pub(crate) fn retarget_group(
    _group: &mut pb::SystemGroup,
    _kind: EntityKind,
    _from: &Id,
    _to: &Id,
    _rewritten: &mut Vec<String>,
) {
}

pub(crate) fn retarget_domain(
    _members: &mut [pb::DomainMembership],
    _kind: EntityKind,
    _from: &Id,
    _to: &Id,
    _rewritten: &mut Vec<String>,
) {
}
