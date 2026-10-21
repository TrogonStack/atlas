//! Native port identities and local product declaration validation.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};

use prost::Message;
use trogon_atlas_proto::{self as pb, entity, DataPortDirection as Direction, DataProduct, Entity};

use crate::connections::{self, ConnectionError};

pub const MAX_PORTS: usize = 1024;
pub const MAX_ASSOCIATIONS: usize = 4096;
pub const MAX_ENCODED_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DataProductError(pub String);
type Result<T> = std::result::Result<T, DataProductError>;
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(DataProductError(message.into()))
    }
}
fn connection_error(error: DataProductError) -> ConnectionError {
    ConnectionError::Port(error.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataPortId(uuid::Uuid);
impl DataPortId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}
impl Default for DataPortId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for DataPortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for DataPortId {
    type Err = DataProductError;
    fn from_str(value: &str) -> Result<Self> {
        let id = uuid::Uuid::parse_str(value)
            .map_err(|_| DataProductError("invalid port UUID".into()))?;
        require(
            !id.is_nil() && id.to_string() == value,
            "port ID must be a non-nil lowercase hyphenated UUID",
        )?;
        Ok(Self(id))
    }
}
impl From<DataPortId> for pb::DataPortId {
    fn from(id: DataPortId) -> Self {
        Self {
            value: id.to_string(),
        }
    }
}
fn port_id(id: Option<&pb::DataPortId>) -> Result<DataPortId> {
    id.ok_or_else(|| DataProductError("missing port ID".into()))?
        .value
        .parse()
}
fn product(entity: &Entity) -> Option<&DataProduct> {
    match entity.kind.as_ref() {
        Some(entity::Kind::DataProduct(value)) => Some(value),
        _ => None,
    }
}
fn product_mut(entity: &mut Entity) -> Option<&mut DataProduct> {
    match entity.kind.as_mut() {
        Some(entity::Kind::DataProduct(value)) => Some(value),
        _ => None,
    }
}
fn ports(value: &DataProduct) -> impl Iterator<Item = (Direction, Option<&pb::DataPortId>)> {
    value
        .inputs
        .iter()
        .map(|p| (Direction::Input, p.port_id.as_ref()))
        .chain(
            value
                .outputs
                .iter()
                .map(|p| (Direction::Output, p.port_id.as_ref())),
        )
}
fn each_port(
    value: &mut DataProduct,
    mut visit: impl FnMut(Direction, &mut Option<pb::DataPortId>) -> Result<()>,
) -> Result<()> {
    for port in &mut value.inputs {
        visit(Direction::Input, &mut port.port_id)?;
    }
    for port in &mut value.outputs {
        visit(Direction::Output, &mut port.port_id)?;
    }
    Ok(())
}
fn active(value: &DataProduct, persisted: bool) -> Result<HashMap<DataPortId, Direction>> {
    require(
        value.inputs.len() + value.outputs.len() <= MAX_PORTS,
        "DataProduct port limit exceeded",
    )?;
    let mut ids = HashMap::new();
    for (direction, id) in ports(value) {
        if persisted || id.is_some() {
            require(
                ids.insert(port_id(id)?, direction).is_none(),
                "duplicate port ID",
            )?;
        }
    }
    Ok(ids)
}

pub fn validate_stored_ports(entity: &Entity) -> Result<()> {
    if let Some(value) = product(entity) {
        active(value, true)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
pub enum ResolvedDataPort<'a> {
    Input(&'a pb::DataInputPort),
    Output(&'a pb::DataOutputPort),
}

pub fn resolve_port<'a>(
    entity: &'a Entity,
    reference: &pb::DataPortRef,
) -> Result<ResolvedDataPort<'a>> {
    let value =
        product(entity).ok_or_else(|| DataProductError("owner is not a DataProduct".into()))?;
    let active = active(value, true)?;
    require(
        reference
            .product
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .is_some_and(|id| Some(id) == value.id.as_ref())
            && !reference.owner_uid.is_empty()
            && entity
                .system
                .as_ref()
                .is_some_and(|s| s.uid == reference.owner_uid),
        "port reference does not identify exact owner incarnation",
    )?;
    let selected = port_id(reference.port_id.as_ref())?;
    let direction = Direction::try_from(reference.direction)
        .map_err(|_| DataProductError("unknown port direction".into()))?;
    require(
        direction != Direction::Unspecified && active.get(&selected) == Some(&direction),
        "port is missing or has wrong direction",
    )?;
    match direction {
        Direction::Input => value
            .inputs
            .iter()
            .find(|p| p.port_id == reference.port_id)
            .map(ResolvedDataPort::Input),
        Direction::Output => value
            .outputs
            .iter()
            .find(|p| p.port_id == reference.port_id)
            .map(ResolvedDataPort::Output),
        Direction::Unspecified => None,
    }
    .ok_or_else(|| DataProductError("port missing".into()))
}

pub(crate) fn prepare_ports(
    previous: Option<&Entity>,
    proposed: &mut Entity,
    trusted: bool,
) -> std::result::Result<(), ConnectionError> {
    let Some(value) = product_mut(proposed) else {
        return Ok(());
    };
    let prior = previous.and_then(product);
    let old = prior
        .map(|p| active(p, true))
        .transpose()
        .map_err(connection_error)?
        .unwrap_or_default();
    let supplied = active(value, false).map_err(connection_error)?;
    if !old.is_empty()
        && supplied.is_empty()
        && (!value.inputs.is_empty() || !value.outputs.is_empty())
    {
        return Err(ConnectionError::Port(
            "edited manifest cannot strip retained port identities".into(),
        ));
    }
    if let Some(prior) = prior {
        let input_stripped = value.inputs.iter().any(|p| {
            p.port_id.is_none()
                && prior.inputs.iter().any(|old| {
                    old.name == p.name && !value.inputs.iter().any(|p| p.port_id == old.port_id)
                })
        });
        let output_stripped = value.outputs.iter().any(|p| {
            p.port_id.is_none()
                && prior.outputs.iter().any(|old| {
                    old.name == p.name && !value.outputs.iter().any(|p| p.port_id == old.port_id)
                })
        });
        if input_stripped || output_stripped {
            return Err(ConnectionError::Port(
                "retained port identity was stripped".into(),
            ));
        }
    }
    each_port(value, |direction, id| {
        if let Some(id) = id {
            let parsed = port_id(Some(id))?;
            require(
                previous.is_none() || trusted || old.get(&parsed) == Some(&direction),
                "port is not active in this direction; omit IDs for additions",
            )?;
            require(
                old.get(&parsed).is_none_or(|old| *old == direction),
                "port direction cannot change",
            )?;
        } else {
            *id = Some(DataPortId::new().into());
        }
        Ok(())
    })
    .map_err(connection_error)
}

pub(crate) fn equivalent_manifest(
    previous: &Entity,
    proposed: &Entity,
) -> std::result::Result<bool, ConnectionError> {
    validate_stored_ports(previous).map_err(connection_error)?;
    let Some(desired) = product(proposed) else {
        return Ok(false);
    };
    connections::validate_stored_connection_ids(previous)?;
    let supplied = active(desired, false).map_err(connection_error)?;
    let mut comparison = previous.clone();
    if supplied.is_empty() {
        if let Some(value) = product_mut(&mut comparison) {
            each_port(value, |_, id| {
                *id = None;
                Ok(())
            })
            .map_err(connection_error)?;
        }
    }
    if crate::semantically_equal(&comparison, proposed) {
        return Ok(true);
    }
    connections::equivalent_with_unassigned_ids(&comparison, proposed)
}

pub(crate) fn removed_ports(
    base: Option<&Entity>,
    opposing: Option<&Entity>,
    selected: &Entity,
) -> std::result::Result<HashSet<DataPortId>, ConnectionError> {
    for entity in base
        .into_iter()
        .chain(opposing)
        .chain(std::iter::once(selected))
    {
        validate_stored_ports(entity).map_err(connection_error)?;
    }
    let same = |e: &&Entity| {
        e.system
            .as_ref()
            .zip(selected.system.as_ref())
            .is_some_and(|(a, b)| !a.uid.is_empty() && a.uid == b.uid)
    };
    let Some(base) = base.filter(same).and_then(product) else {
        return Ok(HashSet::new());
    };
    let Some(opposing) = opposing.filter(same).and_then(product) else {
        return Ok(HashSet::new());
    };
    let current = active(opposing, true).map_err(connection_error)?;
    Ok(active(base, true)
        .map_err(connection_error)?
        .into_keys()
        .filter(|id| !current.contains_key(id))
        .collect())
}
pub(crate) fn reject_removed(
    entity: &Entity,
    removed: &HashSet<DataPortId>,
) -> std::result::Result<(), ConnectionError> {
    if let Some(value) = product(entity) {
        if active(value, true)
            .map_err(connection_error)?
            .keys()
            .any(|id| removed.contains(id))
        {
            return Err(ConnectionError::Port(
                "port from branch base was removed on opposing side".into(),
            ));
        }
    }
    Ok(())
}
pub(crate) fn restore_ports(
    entity: &mut Entity,
    removed: &HashSet<DataPortId>,
) -> std::result::Result<(), ConnectionError> {
    let Some(value) = product_mut(entity) else {
        return Ok(());
    };
    let mut map = HashMap::new();
    each_port(value, |_, id| {
        let parsed = port_id(id.as_ref())?;
        if removed.contains(&parsed) {
            let fresh: pb::DataPortId = DataPortId::new().into();
            map.insert(parsed, fresh.clone());
            *id = Some(fresh);
        }
        Ok(())
    })
    .map_err(connection_error)?;
    for output in &mut value.outputs {
        if let Some(lineage) = &mut output.derived_from {
            for input in &mut lineage.inputs {
                let id = port_id(input.input_port_id.as_ref()).map_err(connection_error)?;
                if let Some(fresh) = map.get(&id) {
                    input.input_port_id = Some(fresh.clone());
                }
            }
        }
    }
    Ok(())
}

fn exact(id: Option<&pb::Id>) -> Result<()> {
    let id = id.ok_or_else(|| DataProductError("missing exact entity reference".into()))?;
    require(
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug),
        "invalid exact entity reference",
    )
}
fn connection(
    reference: Option<&pb::ConnectionRef>,
    kind: pb::EntityKind,
    owner: Option<&pb::Id>,
) -> Result<()> {
    let reference =
        reference.ok_or_else(|| DataProductError("missing connection selection".into()))?;
    let selected = reference
        .owner
        .as_ref()
        .ok_or_else(|| DataProductError("missing connection owner".into()))?;
    exact(selected.id.as_ref())?;
    require(
        selected.kind == kind as i32
            && owner.is_none_or(|id| selected.id.as_ref() == Some(id))
            && !reference.owner_uid.trim().is_empty(),
        "wrong connection owner kind, key or UID",
    )?;
    reference
        .id
        .as_ref()
        .ok_or_else(|| DataProductError("missing connection ID".into()))?
        .value
        .parse::<connections::ConnectionId>()
        .map_err(|e| DataProductError(e.to_string()))?;
    Ok(())
}
fn member(reference: Option<&pb::ContractMemberRef>, kind: pb::ContractMemberKind) -> Result<()> {
    let reference = reference.ok_or_else(|| DataProductError("missing member selection".into()))?;
    exact(reference.specification.as_ref().and_then(|r| r.id.as_ref()))?;
    require(
        reference.kind == kind as i32
            && !reference.owner_uid.trim().is_empty()
            && reference.expected_digest.is_none(),
        "wrong member kind, UID or unexpected member digest",
    )?;
    reference
        .member_id
        .as_ref()
        .ok_or_else(|| DataProductError("missing member ID".into()))?
        .value
        .parse::<crate::specification::MemberId>()
        .map_err(|e| DataProductError(e.to_string()))?;
    Ok(())
}
fn contract(value: &pb::OutputContract) -> Result<()> {
    match value.r#type.as_ref() {
        Some(pb::output_contract::Type::Schema(r)) => exact(r.id.as_ref()),
        Some(pb::output_contract::Type::LibraryType(r)) => {
            exact(r.library.as_ref().and_then(|r| r.id.as_ref()))?;
            require(
                r.message.as_ref().is_some_and(|m| {
                    !m.value.is_empty()
                        && m.value.split('.').all(|part| {
                            !part.is_empty()
                                && part.bytes().enumerate().all(|(i, b)| {
                                    b.is_ascii_alphabetic()
                                        || b == b'_'
                                        || (i > 0 && b.is_ascii_digit())
                                })
                        })
                }),
                "qualified protobuf message name required",
            )
        }
        Some(pb::output_contract::Type::ContractType(r)) => {
            member(r.schema.as_ref(), pb::ContractMemberKind::Schema)?;
            r.expected_artifact_id
                .as_ref()
                .ok_or_else(|| DataProductError("missing artifact ID pin".into()))?
                .value
                .parse::<crate::specification::MemberId>()
                .map_err(|e| DataProductError(e.to_string()))?;
            require(
                r.expected_digest.as_ref().is_some_and(|d| {
                    d.sha256.len() == 64
                        && d.sha256
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }),
                "SHA-256 digest pin required",
            )?;
            require(
                r.expected_selector
                    .as_ref()
                    .is_none_or(|s| !s.syntax.trim().is_empty() && !s.value.trim().is_empty()),
                "opaque selector requires syntax and value",
            )
        }
        None => Err(DataProductError("output contract type required".into())),
    }
}
fn access(value: &pb::OutputAccess) -> Result<()> {
    match value.selection.as_ref() {
        Some(pb::output_access::Selection::Store(r)) => {
            let id = r.store.as_ref().and_then(|r| r.id.as_ref());
            exact(id)?;
            connection(r.attachment.as_ref(), pb::EntityKind::DataStore, id)
        }
        Some(pb::output_access::Selection::Channel(r)) => {
            let id = r.channel.as_ref().and_then(|r| r.id.as_ref());
            exact(id)?;
            connection(r.binding.as_ref(), pb::EntityKind::Channel, id)?;
            connection(
                r.observation.as_ref(),
                pb::EntityKind::AutomationSlice,
                None,
            )
        }
        Some(pb::output_access::Selection::Operation(r)) => {
            member(r.operation.as_ref(), pb::ContractMemberKind::Operation)?;
            let owner = r
                .operation
                .as_ref()
                .and_then(|m| m.specification.as_ref())
                .and_then(|r| r.id.as_ref());
            connection(r.response.as_ref(), pb::EntityKind::Specification, owner)?;
            let uid = r.operation.as_ref().map(|m| m.owner_uid.as_str());
            require(
                r.response.as_ref().map(|r| r.owner_uid.as_str()) == uid,
                "response owner incarnation mismatch",
            )?;
            if let Some(provider) = &r.provider_exposure {
                connection(Some(provider), pb::EntityKind::Specification, owner)?;
                require(
                    Some(provider.owner_uid.as_str()) == uid,
                    "provider owner incarnation mismatch",
                )?;
            }
            Ok(())
        }
        None => Err(DataProductError("output access selection required".into())),
    }
}

/// Validate persisted structure and local lineage. External graph resolution belongs to authorized backend admission.
pub fn validate_values(value: &DataProduct) -> Result<()> {
    exact(value.id.as_ref())?;
    require(!value.title.trim().is_empty(), "DataProduct title required")?;
    require(
        value.purpose.as_ref().is_none_or(|p| !p.trim().is_empty()),
        "supplied purpose must be nonblank",
    )?;
    require(
        !value.outputs.is_empty(),
        "DataProduct requires output ports",
    )?;
    require(
        value.encoded_len() <= MAX_ENCODED_BYTES,
        "DataProduct encoded size limit exceeded",
    )?;
    let ids = active(value, true)?;
    for names in [
        value
            .inputs
            .iter()
            .map(|p| p.name.as_ref())
            .collect::<Vec<_>>(),
        value.outputs.iter().map(|p| p.name.as_ref()).collect(),
    ] {
        let mut seen = HashSet::new();
        for name in names {
            require(
                name.is_some_and(|name| !name.value.trim().is_empty() && seen.insert(&name.value)),
                "port names must be nonblank and unique within direction",
            )?;
        }
    }
    for input in &value.inputs {
        if let Some(data) = &input.data {
            match data.target.as_ref() {
                Some(pb::input_data_selection::Target::ReadModel(r)) => exact(r.id.as_ref())?,
                Some(pb::input_data_selection::Target::Event(r)) => exact(r.id.as_ref())?,
                None => return Err(DataProductError("input data target required".into())),
            }
        }
    }
    for output in &value.outputs {
        if let Some(data) = &output.data {
            exact(data.read_model.as_ref().and_then(|r| r.id.as_ref()))?;
        }
        require(
            output.data.is_some() || output.access.is_empty(),
            "undeclared output cannot declare access",
        )?;
        if let Some(value) = &output.contract {
            contract(value)?;
        }
        for value in &output.access {
            access(value)?;
        }
        if let Some(lineage) = &output.derived_from {
            let mut selected = HashSet::new();
            for input in &lineage.inputs {
                let id = port_id(input.input_port_id.as_ref())?;
                require(
                    ids.get(&id) == Some(&Direction::Input) && selected.insert(id),
                    "lineage must select distinct local input ports",
                )?;
            }
        }
    }
    let entity = Entity {
        kind: Some(entity::Kind::DataProduct(value.clone())),
        system: None,
    };
    let occurrences = connections::connection_occurrences(&entity)
        .map_err(|e| DataProductError(e.to_string()))?;
    require(
        occurrences.len() <= MAX_ASSOCIATIONS,
        "DataProduct association limit exceeded",
    )?;
    connections::validate_stored_connection_ids(&entity)
        .map_err(|e| DataProductError(e.to_string()))
}
