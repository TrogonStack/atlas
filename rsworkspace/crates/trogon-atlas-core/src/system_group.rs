use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use prost::Message;
use trogon_atlas_proto::{self as pb, entity, Entity, EntityKind, EntityRef, Id};

use crate::{
    connections::{self, ConnectionId},
    entity_id, entity_kind,
    refs::OutboundRef,
};

pub const MAX_GROUP_MEMBERS: usize = 1_024;
pub const MAX_GROUP_RELATIONSHIPS: usize = 1_024;
pub const MAX_GROUP_ACTORS: usize = 1_024;
pub const MAX_DOMAIN_MEMBERS: usize = 1_024;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct SystemGroupError(pub String);

type Result<T> = std::result::Result<T, SystemGroupError>;

fn fail<T>(message: &str) -> Result<T> {
    Err(SystemGroupError(message.into()))
}

fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        fail(message)
    }
}

#[must_use]
pub fn design_graph_kind() -> Option<EntityKind> {
    Some(EntityKind::DesignGraph)
}

struct Slot<'a> {
    kind: Option<EntityKind>,
    arm: &'static str,
    id: Option<&'a Id>,
}

struct SlotMut<'a> {
    kind: Option<EntityKind>,
    arm: &'static str,
    id: &'a mut Option<Id>,
}

fn group_member_slot(member: &pb::SystemGroupMember) -> Option<Slot<'_>> {
    use pb::system_group_member::Target as T;
    Some(match member.target.as_ref()? {
        T::SystemGroup(v) => Slot {
            kind: Some(EntityKind::SystemGroup),
            arm: "system_group",
            id: v.id.as_ref(),
        },
        T::Component(v) => Slot {
            kind: Some(EntityKind::Component),
            arm: "component",
            id: v.id.as_ref(),
        },
        T::ExternalSystem(v) => Slot {
            kind: Some(EntityKind::ExternalSystem),
            arm: "external_system",
            id: v.id.as_ref(),
        },
        T::DataStore(v) => Slot {
            kind: Some(EntityKind::DataStore),
            arm: "data_store",
            id: v.id.as_ref(),
        },
        T::Schema(v) => Slot {
            kind: Some(EntityKind::Schema),
            arm: "schema",
            id: v.id.as_ref(),
        },
        T::DesignGraph(v) => Slot {
            kind: design_graph_kind(),
            arm: "design_graph",
            id: v.id.as_ref(),
        },
    })
}

fn group_member_slot_mut(member: &mut pb::SystemGroupMember) -> Option<SlotMut<'_>> {
    use pb::system_group_member::Target as T;
    Some(match member.target.as_mut()? {
        T::SystemGroup(v) => SlotMut {
            kind: Some(EntityKind::SystemGroup),
            arm: "system_group",
            id: &mut v.id,
        },
        T::Component(v) => SlotMut {
            kind: Some(EntityKind::Component),
            arm: "component",
            id: &mut v.id,
        },
        T::ExternalSystem(v) => SlotMut {
            kind: Some(EntityKind::ExternalSystem),
            arm: "external_system",
            id: &mut v.id,
        },
        T::DataStore(v) => SlotMut {
            kind: Some(EntityKind::DataStore),
            arm: "data_store",
            id: &mut v.id,
        },
        T::Schema(v) => SlotMut {
            kind: Some(EntityKind::Schema),
            arm: "schema",
            id: &mut v.id,
        },
        T::DesignGraph(v) => SlotMut {
            kind: design_graph_kind(),
            arm: "design_graph",
            id: &mut v.id,
        },
    })
}

fn domain_member_slot(member: &pb::DomainMember) -> Option<Slot<'_>> {
    use pb::domain_member::Target as T;
    Some(match member.target.as_ref()? {
        T::Domain(v) => Slot {
            kind: Some(EntityKind::Domain),
            arm: "domain",
            id: v.id.as_ref(),
        },
        T::SystemGroup(v) => Slot {
            kind: Some(EntityKind::SystemGroup),
            arm: "system_group",
            id: v.id.as_ref(),
        },
        T::Component(v) => Slot {
            kind: Some(EntityKind::Component),
            arm: "component",
            id: v.id.as_ref(),
        },
        T::DataProduct(v) => Slot {
            kind: Some(EntityKind::DataProduct),
            arm: "data_product",
            id: v.id.as_ref(),
        },
        T::Schema(v) => Slot {
            kind: Some(EntityKind::Schema),
            arm: "schema",
            id: v.id.as_ref(),
        },
        T::DesignGraph(v) => Slot {
            kind: design_graph_kind(),
            arm: "design_graph",
            id: v.id.as_ref(),
        },
        T::ExternalSystem(v) => Slot {
            kind: Some(EntityKind::ExternalSystem),
            arm: "external_system",
            id: v.id.as_ref(),
        },
    })
}

fn domain_member_slot_mut(member: &mut pb::DomainMember) -> Option<SlotMut<'_>> {
    use pb::domain_member::Target as T;
    Some(match member.target.as_mut()? {
        T::Domain(v) => SlotMut {
            kind: Some(EntityKind::Domain),
            arm: "domain",
            id: &mut v.id,
        },
        T::SystemGroup(v) => SlotMut {
            kind: Some(EntityKind::SystemGroup),
            arm: "system_group",
            id: &mut v.id,
        },
        T::Component(v) => SlotMut {
            kind: Some(EntityKind::Component),
            arm: "component",
            id: &mut v.id,
        },
        T::DataProduct(v) => SlotMut {
            kind: Some(EntityKind::DataProduct),
            arm: "data_product",
            id: &mut v.id,
        },
        T::Schema(v) => SlotMut {
            kind: Some(EntityKind::Schema),
            arm: "schema",
            id: &mut v.id,
        },
        T::DesignGraph(v) => SlotMut {
            kind: design_graph_kind(),
            arm: "design_graph",
            id: &mut v.id,
        },
        T::ExternalSystem(v) => SlotMut {
            kind: Some(EntityKind::ExternalSystem),
            arm: "external_system",
            id: &mut v.id,
        },
    })
}

fn actor_slot(target: &pb::SystemActorTarget) -> Option<Slot<'_>> {
    use pb::system_actor_target::Target as T;
    Some(match target.target.as_ref()? {
        T::Persona(v) => Slot {
            kind: Some(EntityKind::Persona),
            arm: "persona",
            id: v.id.as_ref(),
        },
        T::Component(v) => Slot {
            kind: Some(EntityKind::Component),
            arm: "component",
            id: v.id.as_ref(),
        },
        T::ExternalSystem(v) => Slot {
            kind: Some(EntityKind::ExternalSystem),
            arm: "external_system",
            id: v.id.as_ref(),
        },
        T::SystemGroup(v) => Slot {
            kind: Some(EntityKind::SystemGroup),
            arm: "system_group",
            id: v.id.as_ref(),
        },
    })
}

fn actor_slot_mut(target: &mut pb::SystemActorTarget) -> Option<SlotMut<'_>> {
    use pb::system_actor_target::Target as T;
    Some(match target.target.as_mut()? {
        T::Persona(v) => SlotMut {
            kind: Some(EntityKind::Persona),
            arm: "persona",
            id: &mut v.id,
        },
        T::Component(v) => SlotMut {
            kind: Some(EntityKind::Component),
            arm: "component",
            id: &mut v.id,
        },
        T::ExternalSystem(v) => SlotMut {
            kind: Some(EntityKind::ExternalSystem),
            arm: "external_system",
            id: &mut v.id,
        },
        T::SystemGroup(v) => SlotMut {
            kind: Some(EntityKind::SystemGroup),
            arm: "system_group",
            id: &mut v.id,
        },
    })
}

fn entity_ref(kind: Option<EntityKind>, id: Option<&Id>) -> Option<EntityRef> {
    Some(EntityRef {
        kind: kind? as i32,
        id: id.cloned(),
    })
}

#[must_use]
pub fn group_member_ref(member: &pb::SystemGroupMember) -> Option<EntityRef> {
    let slot = group_member_slot(member)?;
    entity_ref(slot.kind, slot.id)
}

#[must_use]
pub fn domain_member_ref(member: &pb::DomainMember) -> Option<EntityRef> {
    let slot = domain_member_slot(member)?;
    entity_ref(slot.kind, slot.id)
}

#[must_use]
pub fn actor_ref(target: &pb::SystemActorTarget) -> Option<EntityRef> {
    let slot = actor_slot(target)?;
    entity_ref(slot.kind, slot.id)
}

pub(crate) fn occurrence_target(full_name: &str, encoded: &[u8]) -> Option<EntityRef> {
    match full_name {
        "trogonatlas.eventmodel.v1alpha1.SystemGroupMembership" => {
            let value = pb::SystemGroupMembership::decode(encoded).ok()?;
            value.member.as_ref().and_then(group_member_ref)
        }
        "trogonatlas.eventmodel.v1alpha1.DomainMembership" => {
            let value = pb::DomainMembership::decode(encoded).ok()?;
            value.member.as_ref().and_then(domain_member_ref)
        }
        "trogonatlas.eventmodel.v1alpha1.SystemGroupRelationship" => {
            let value = pb::SystemGroupRelationship::decode(encoded).ok()?;
            value.target.as_ref().map(|target| EntityRef {
                kind: EntityKind::SystemGroup as i32,
                id: target.id.clone(),
            })
        }
        "trogonatlas.eventmodel.v1alpha1.SystemActorLink" => {
            let value = pb::SystemActorLink::decode(encoded).ok()?;
            value.target.as_ref().and_then(actor_ref)
        }
        _ => None,
    }
}

fn exact(id: Option<&Id>) -> Result<()> {
    require(
        id.is_some_and(|id| {
            id.version > 0
                && crate::is_safe_id_component(&id.namespace)
                && crate::is_safe_id_component(&id.slug)
        }),
        "invalid exact system structure endpoint",
    )
}

fn optional_title(value: Option<&pb::CatalogTitle>) -> Result<()> {
    if value.is_some() {
        crate::decision_document::validate_catalog_title(value)
            .map_err(|e| SystemGroupError(e.to_string()))?;
    }
    Ok(())
}

fn optional_text(value: Option<&pb::CatalogText>) -> Result<()> {
    if value.is_some() {
        crate::decision_document::validate_catalog_text(value)
            .map_err(|e| SystemGroupError(e.to_string()))?;
    }
    Ok(())
}

fn validate_member_slot(
    slot: Option<Slot<'_>>,
    owner_kind: EntityKind,
    owner_id: &Id,
) -> Result<()> {
    let slot = slot.ok_or_else(|| SystemGroupError("member target required".into()))?;
    exact(slot.id)?;
    require(
        slot.kind != Some(owner_kind) || slot.id != Some(owner_id),
        "self membership forbidden",
    )
}

fn parse_id(value: Option<&pb::ConnectionId>) -> Result<()> {
    if let Some(id) = value {
        id.value
            .parse::<ConnectionId>()
            .map_err(|e| SystemGroupError(e.to_string()))?;
    }
    Ok(())
}

pub fn validate_values(entity: &Entity) -> Result<()> {
    match entity.kind.as_ref() {
        Some(entity::Kind::SystemGroup(group)) => validate_group(entity, group),
        Some(entity::Kind::Domain(domain)) => validate_domain(entity, domain),
        _ => Ok(()),
    }
}

fn validate_group(owner: &Entity, group: &pb::SystemGroup) -> Result<()> {
    require(
        owner.encoded_len() <= MAX_ENCODED_BYTES,
        "system group exceeds encoded byte limit",
    )?;
    let owner_id = group
        .id
        .as_ref()
        .ok_or_else(|| SystemGroupError("exact owner ID required".into()))?;
    exact(group.id.as_ref())?;
    require(
        matches!(
            pb::SystemGroupScope::try_from(group.scope),
            Ok(pb::SystemGroupScope::Internal | pb::SystemGroupScope::External)
        ),
        "system group scope required",
    )?;
    require(
        group.members.len() <= MAX_GROUP_MEMBERS,
        "system group member limit exceeded",
    )?;
    require(
        group.relationships.len() <= MAX_GROUP_RELATIONSHIPS,
        "system group relationship limit exceeded",
    )?;
    require(
        group.actors.len() <= MAX_GROUP_ACTORS,
        "system group actor limit exceeded",
    )?;
    for value in &group.members {
        parse_id(value.connection_id.as_ref())?;
        validate_member_slot(
            value.member.as_ref().and_then(group_member_slot),
            EntityKind::SystemGroup,
            owner_id,
        )?;
        optional_text(value.doc.as_ref())?;
    }
    for value in &group.relationships {
        parse_id(value.connection_id.as_ref())?;
        let target = value
            .target
            .as_ref()
            .ok_or_else(|| SystemGroupError("relationship target required".into()))?;
        exact(target.id.as_ref())?;
        require(
            target.id.as_ref() != Some(owner_id),
            "self relationship forbidden",
        )?;
        optional_title(value.label.as_ref())?;
        optional_text(value.doc.as_ref())?;
    }
    for value in &group.actors {
        parse_id(value.connection_id.as_ref())?;
        let slot = value
            .target
            .as_ref()
            .and_then(actor_slot)
            .ok_or_else(|| SystemGroupError("actor target required".into()))?;
        exact(slot.id)?;
        require(
            matches!(
                pb::SystemActorDirection::try_from(value.direction),
                Ok(pb::SystemActorDirection::Inbound | pb::SystemActorDirection::Outbound)
            ),
            "actor direction required",
        )?;
        optional_title(value.label.as_ref())?;
        optional_text(value.doc.as_ref())?;
    }
    connections::validate_connection_ids(owner).map_err(|e| SystemGroupError(e.to_string()))
}

fn validate_domain(owner: &Entity, domain: &pb::Domain) -> Result<()> {
    if domain.members.is_empty() {
        return Ok(());
    }
    require(
        owner.encoded_len() <= MAX_ENCODED_BYTES,
        "domain exceeds encoded byte limit",
    )?;
    require(
        domain.members.len() <= MAX_DOMAIN_MEMBERS,
        "domain member limit exceeded",
    )?;
    let owner_id = domain
        .id
        .as_ref()
        .ok_or_else(|| SystemGroupError("exact owner ID required".into()))?;
    for value in &domain.members {
        parse_id(value.connection_id.as_ref())?;
        validate_member_slot(
            value.member.as_ref().and_then(domain_member_slot),
            EntityKind::Domain,
            owner_id,
        )?;
        optional_text(value.doc.as_ref())?;
    }
    connections::validate_connection_ids(owner).map_err(|e| SystemGroupError(e.to_string()))
}

pub fn resolve_connection_context(
    owner: &Entity,
    reference: &pb::ConnectionRef,
) -> Result<Option<EntityRef>> {
    let prefixes: &[&str] = match entity_kind(owner) {
        Some(EntityKind::SystemGroup) => &[
            "system_group.members[",
            "system_group.relationships[",
            "system_group.actors[",
        ],
        Some(EntityKind::Domain) => &["domain.members["],
        _ => return fail("unsupported system structure owner"),
    };
    let occurrence = connections::resolve_connection(owner, reference)
        .map_err(|e| SystemGroupError(e.to_string()))?;
    require(
        prefixes
            .iter()
            .any(|prefix| occurrence.path.starts_with(prefix)),
        "unsupported system structure connection role",
    )?;
    if let Some(target) = &occurrence.target {
        exact(target.id.as_ref())?;
    }
    Ok(occurrence.target)
}

fn push_outbound(slot: &Slot<'_>, field: &str, out: &mut Vec<OutboundRef>) {
    if let Some(id) = slot.id.cloned() {
        out.push(OutboundRef {
            to_kind: slot.kind,
            to_id: id,
            from_field: format!("{field}.{}", slot.arm),
        });
    }
}

pub(crate) fn outbound_group(group: &pb::SystemGroup, out: &mut Vec<OutboundRef>) {
    for (index, value) in group.members.iter().enumerate() {
        if let Some(slot) = value.member.as_ref().and_then(group_member_slot) {
            push_outbound(&slot, &format!("members[{index}]"), out);
        }
    }
    for (index, value) in group.relationships.iter().enumerate() {
        if let Some(id) = value.target.as_ref().and_then(|target| target.id.clone()) {
            out.push(OutboundRef {
                to_kind: Some(EntityKind::SystemGroup),
                to_id: id,
                from_field: format!("relationships[{index}].target"),
            });
        }
    }
    for (index, value) in group.actors.iter().enumerate() {
        if let Some(slot) = value.target.as_ref().and_then(actor_slot) {
            push_outbound(&slot, &format!("actors[{index}].target"), out);
        }
    }
}

pub(crate) fn outbound_domain(members: &[pb::DomainMembership], out: &mut Vec<OutboundRef>) {
    for (index, value) in members.iter().enumerate() {
        if let Some(slot) = value.member.as_ref().and_then(domain_member_slot) {
            push_outbound(&slot, &format!("members[{index}]"), out);
        }
    }
}

fn renew(
    slot: Option<SlotMut<'_>>,
    kind: EntityKind,
    from: &Id,
    to: &Id,
    path: &str,
) -> Option<(String, bool)> {
    let slot = slot?;
    if slot.kind == Some(kind) && slot.id.as_ref() == Some(from) && from != to {
        *slot.id = Some(to.clone());
        Some((format!("{path}.{}", slot.arm), true))
    } else {
        None
    }
}

pub(crate) fn retarget_group(
    group: &mut pb::SystemGroup,
    kind: EntityKind,
    from: &Id,
    to: &Id,
    rewritten: &mut Vec<String>,
) {
    for (index, value) in group.members.iter_mut().enumerate() {
        let slot = value.member.as_mut().and_then(group_member_slot_mut);
        if let Some((path, _)) = renew(slot, kind, from, to, &format!("members[{index}]")) {
            value.connection_id = None;
            rewritten.push(path);
        }
    }
    if kind == EntityKind::SystemGroup && from != to {
        for (index, value) in group.relationships.iter_mut().enumerate() {
            if let Some(id) = value.target.as_mut().map(|target| &mut target.id) {
                if id.as_ref() == Some(from) {
                    *id = Some(to.clone());
                    value.connection_id = None;
                    rewritten.push(format!("relationships[{index}].target"));
                }
            }
        }
    }
    for (index, value) in group.actors.iter_mut().enumerate() {
        let slot = value.target.as_mut().and_then(actor_slot_mut);
        if let Some((path, _)) = renew(slot, kind, from, to, &format!("actors[{index}].target")) {
            value.connection_id = None;
            rewritten.push(path);
        }
    }
}

pub(crate) fn retarget_domain(
    members: &mut [pb::DomainMembership],
    kind: EntityKind,
    from: &Id,
    to: &Id,
    rewritten: &mut Vec<String>,
) {
    for (index, value) in members.iter_mut().enumerate() {
        let slot = value.member.as_mut().and_then(domain_member_slot_mut);
        if let Some((path, _)) = renew(slot, kind, from, to, &format!("members[{index}]")) {
            value.connection_id = None;
            rewritten.push(path);
        }
    }
}

pub const SYSTEM_GROUP_MEMBERSHIP_CYCLE: &str = "SYSTEM_GROUP_MEMBERSHIP_CYCLE";
pub const DOMAIN_MEMBERSHIP_CYCLE: &str = "DOMAIN_MEMBERSHIP_CYCLE";
pub const SYSTEM_GROUP_MEMBERSHIP_REPEAT: &str = "SYSTEM_GROUP_MEMBERSHIP_REPEAT";
pub const DOMAIN_MEMBERSHIP_REPEAT: &str = "DOMAIN_MEMBERSHIP_REPEAT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryHop {
    pub owner: EntityRef,
    pub owner_uid: String,
    pub connection_id: ConnectionId,
    pub target: EntityRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    pub rule: &'static str,
    pub path: Vec<AdvisoryHop>,
}

type NodeKey = (i32, String, String, u64);

fn node_key(kind: i32, id: &Id) -> NodeKey {
    (kind, id.namespace.clone(), id.slug.clone(), id.version)
}

struct Edge {
    hop: AdvisoryHop,
    to: NodeKey,
}

struct Owner {
    key: NodeKey,
    edges: Vec<Edge>,
}

fn collect(entity: &Entity) -> Option<(EntityKind, Owner)> {
    let kind = entity_kind(entity)?;
    let id = entity_id(entity)?;
    let uid = entity
        .system
        .as_ref()
        .map(|system| system.uid.clone())
        .unwrap_or_default();
    let owner = EntityRef {
        kind: kind as i32,
        id: Some(id.clone()),
    };
    let mut edges = Vec::new();
    let mut add = |connection: Option<&pb::ConnectionId>, target: Option<EntityRef>| {
        let (Some(connection), Some(target)) = (connection, target) else {
            return;
        };
        let (Ok(connection_id), Some(target_id)) =
            (connection.value.parse::<ConnectionId>(), target.id.as_ref())
        else {
            return;
        };
        edges.push(Edge {
            to: node_key(target.kind, target_id),
            hop: AdvisoryHop {
                owner: owner.clone(),
                owner_uid: uid.clone(),
                connection_id,
                target,
            },
        });
    };
    match entity.kind.as_ref()? {
        entity::Kind::SystemGroup(group) => {
            for value in &group.members {
                add(
                    value.connection_id.as_ref(),
                    value.member.as_ref().and_then(group_member_ref),
                );
            }
        }
        entity::Kind::Domain(domain) => {
            for value in &domain.members {
                add(
                    value.connection_id.as_ref(),
                    value.member.as_ref().and_then(domain_member_ref),
                );
            }
        }
        _ => return None,
    }
    Some((
        kind,
        Owner {
            key: node_key(kind as i32, id),
            edges,
        },
    ))
}

fn strongly_connected(nodes: &[NodeKey], adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let count = nodes.len();
    let mut index = vec![usize::MAX; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut next = 0;
    for root in 0..count {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        while let Some(&(node, cursor)) = work.last() {
            if cursor == 0 {
                index[node] = next;
                low[node] = next;
                next += 1;
                stack.push(node);
                on_stack[node] = true;
            }
            if let Some(&target) = adjacency[node].get(cursor) {
                if let Some(frame) = work.last_mut() {
                    frame.1 += 1;
                }
                if index[target] == usize::MAX {
                    work.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(index[target]);
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[node]);
                }
                if low[node] == index[node] {
                    let mut component = Vec::new();
                    while let Some(member) = stack.pop() {
                        on_stack[member] = false;
                        component.push(member);
                        if member == node {
                            break;
                        }
                    }
                    components.push(component);
                }
            }
        }
    }
    components
}

fn shortest_cycle(
    start: usize,
    members: &HashSet<usize>,
    edges: &[Vec<(usize, usize)>],
) -> Vec<(usize, usize)> {
    let mut previous: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    let mut seen = HashSet::from([start]);
    while let Some(node) = queue.pop_front() {
        for &(slot, target) in &edges[node] {
            if !members.contains(&target) {
                continue;
            }
            if target == start {
                let mut path = vec![(node, slot)];
                let mut cursor = node;
                while cursor != start {
                    let (before, via) = previous[&cursor];
                    path.push((before, via));
                    cursor = before;
                }
                path.reverse();
                return path;
            }
            if seen.insert(target) {
                previous.insert(target, (node, slot));
                queue.push_back(target);
            }
        }
    }
    Vec::new()
}

/// Cycles and exact repeats among the supplied owners. Findings are advisory,
/// inspect only supplied (readable) owners and never block a write.
#[must_use]
pub fn advisories(owners: &[Entity]) -> Vec<Advisory> {
    let mut by_kind: BTreeMap<i32, Vec<Owner>> = BTreeMap::new();
    for entity in owners {
        if let Some((kind, owner)) = collect(entity) {
            by_kind.entry(kind as i32).or_default().push(owner);
        }
    }
    let mut findings = Vec::new();
    for (kind, mut group) in by_kind {
        group.sort_by(|a, b| a.key.cmp(&b.key));
        let (cycle_rule, repeat_rule) = if kind == EntityKind::Domain as i32 {
            (DOMAIN_MEMBERSHIP_CYCLE, DOMAIN_MEMBERSHIP_REPEAT)
        } else {
            (
                SYSTEM_GROUP_MEMBERSHIP_CYCLE,
                SYSTEM_GROUP_MEMBERSHIP_REPEAT,
            )
        };
        let nodes: Vec<NodeKey> = group.iter().map(|owner| owner.key.clone()).collect();
        let position: HashMap<&NodeKey, usize> = nodes
            .iter()
            .enumerate()
            .map(|(index, key)| (key, index))
            .collect();
        let edges: Vec<Vec<(usize, usize)>> = group
            .iter()
            .map(|owner| {
                owner
                    .edges
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, edge)| Some((slot, *position.get(&edge.to)?)))
                    .collect()
            })
            .collect();
        let adjacency: Vec<Vec<usize>> = edges
            .iter()
            .map(|list| list.iter().map(|&(_, target)| target).collect())
            .collect();
        let mut cycles = Vec::new();
        for component in strongly_connected(&nodes, &adjacency) {
            let members: HashSet<usize> = component.iter().copied().collect();
            let Some(&start) = component.iter().min() else {
                continue;
            };
            if component.len() == 1 && !adjacency[start].contains(&start) {
                continue;
            }
            let path = shortest_cycle(start, &members, &edges);
            if path.is_empty() {
                continue;
            }
            cycles.push((
                start,
                Advisory {
                    rule: cycle_rule,
                    path: path
                        .into_iter()
                        .map(|(node, slot)| group[node].edges[slot].hop.clone())
                        .collect(),
                },
            ));
        }
        cycles.sort_by_key(|(start, _)| *start);
        findings.extend(cycles.into_iter().map(|(_, advisory)| advisory));
        for owner in &group {
            let mut repeats: BTreeMap<NodeKey, Vec<&Edge>> = BTreeMap::new();
            for edge in &owner.edges {
                repeats.entry(edge.to.clone()).or_default().push(edge);
            }
            findings.extend(
                repeats
                    .into_values()
                    .filter(|edges| edges.len() > 1)
                    .map(|edges| Advisory {
                        rule: repeat_rule,
                        path: edges.into_iter().map(|edge| edge.hop.clone()).collect(),
                    }),
            );
        }
    }
    findings
}
