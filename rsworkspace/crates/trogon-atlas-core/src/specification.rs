//! Native contract values, local graph validation and exact member selection.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};

use prost::Message;
use sha2::{Digest, Sha256};
use trogon_atlas_proto::{
    self as pb, contract_dependency, entity, operation_binding, schema_binding, schema_definition,
    ContractMemberKind as Kind, Entity, Specification,
};

use crate::connections::{self, ConnectionError};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SpecificationError(pub String);

type Result<T> = std::result::Result<T, SpecificationError>;
type MemberIdentity = Option<pb::ContractMemberId>;

fn require(ok: bool, detail: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(SpecificationError(detail.into()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemberId(uuid::Uuid);
impl MemberId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }
}
impl Default for MemberId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for MemberId {
    type Err = SpecificationError;
    fn from_str(value: &str) -> Result<Self> {
        let parsed = uuid::Uuid::parse_str(value)
            .map_err(|_| SpecificationError("invalid member UUID".into()))?;
        require(
            !parsed.is_nil() && parsed.to_string() == value,
            "member ID must be a non-nil lowercase hyphenated UUID",
        )?;
        Ok(Self(parsed))
    }
}
impl From<MemberId> for pb::ContractMemberId {
    fn from(value: MemberId) -> Self {
        Self {
            value: value.to_string(),
        }
    }
}

#[must_use]
pub fn content_digest(content: &str) -> pb::ContentDigest {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut sha256 = String::with_capacity(64);
    for byte in Sha256::digest(content.as_bytes()) {
        sha256.push(char::from(HEX[usize::from(byte >> 4)]));
        sha256.push(char::from(HEX[usize::from(byte & 15)]));
    }
    pb::ContentDigest { sha256 }
}

#[must_use]
pub fn is_api(value: &Specification) -> bool {
    value.exposures.iter().any(|exposure| {
        exposure.operation_id.is_some()
            && value
                .operations
                .iter()
                .any(|operation| operation.member_id == exposure.operation_id)
    })
}

fn member_id(id: Option<&pb::ContractMemberId>) -> Result<MemberId> {
    id.ok_or_else(|| SpecificationError("missing member ID".into()))?
        .value
        .parse()
}

macro_rules! members {
    ($value:expr, $callback:expr) => {{
        let value = &mut *$value;
        #[allow(unused_mut)]
        let mut callback = $callback;
        for member in &mut value.artifacts {
            callback(Kind::Artifact, &mut member.member_id)?;
        }
        for member in &mut value.schemas {
            callback(Kind::Schema, &mut member.member_id)?;
        }
        for member in &mut value.messages {
            callback(Kind::Message, &mut member.member_id)?;
        }
        for member in &mut value.operations {
            callback(Kind::Operation, &mut member.member_id)?;
        }
    }};
}

fn active(value: &Specification, persisted: bool) -> Result<HashMap<MemberId, Kind>> {
    let mut output = HashMap::new();
    members!(&mut value.clone(), |kind,
                                  id: &mut MemberIdentity|
     -> Result<()> {
        if id.is_some() || persisted {
            let id = member_id(id.as_ref())?;
            require(
                output.insert(id, kind).is_none(),
                format!("duplicate member ID {id}"),
            )?;
        }
        Ok(())
    });
    Ok(output)
}

fn spec(entity: &Entity) -> Option<&Specification> {
    match entity.kind.as_ref() {
        Some(entity::Kind::Specification(s)) => Some(s),
        _ => None,
    }
}
fn spec_mut(entity: &mut Entity) -> Option<&mut Specification> {
    match entity.kind.as_mut() {
        Some(entity::Kind::Specification(s)) => Some(s),
        _ => None,
    }
}
fn connection_error(error: SpecificationError) -> ConnectionError {
    ConnectionError::Member(error.0)
}

pub(crate) fn validate_stored_members(entity: &Entity) -> std::result::Result<(), ConnectionError> {
    if let Some(value) = spec(entity) {
        active(value, true).map_err(connection_error)?;
    }
    Ok(())
}

pub(crate) fn prepare_members(
    previous: Option<&Entity>,
    proposed: &mut Entity,
    trusted: bool,
) -> std::result::Result<(), ConnectionError> {
    let Some(value) = spec_mut(proposed) else {
        return Ok(());
    };
    let prior = previous.and_then(spec);
    let old = prior
        .map(|s| active(s, true))
        .transpose()
        .map_err(connection_error)?
        .unwrap_or_default();
    let supplied = active(value, false).map_err(connection_error)?;
    if !old.is_empty() && supplied.is_empty() {
        return Err(ConnectionError::Member("edited manifest cannot strip every retained member identity; preserve returned member IDs".into()));
    }
    if let Some(prior) = prior {
        macro_rules! retained {
            ($collection:ident, $same:expr) => {
                for proposed in &value.$collection {
                    if proposed.member_id.is_none() {
                        if prior.$collection.iter().any(|old| {
                            !value.$collection.iter().any(|m| m.member_id == old.member_id)
                                && old.name == proposed.name && ($same)(old, proposed)
                        }) {
                            return Err(ConnectionError::Member("retained member identity was stripped; preserve returned member IDs".into()));
                        }
                    }
                }
            };
        }
        retained!(
            artifacts,
            |a: &pb::ContractArtifact, b: &pb::ContractArtifact| a.content == b.content
                && a.format == b.format
                && a.format_version == b.format_version
                && a.media_type == b.media_type
        );
        retained!(schemas, |_: &pb::ContractSchema, _: &pb::ContractSchema| {
            true
        });
        retained!(
            messages,
            |_: &pb::ContractMessage, _: &pb::ContractMessage| true
        );
        retained!(
            operations,
            |_: &pb::ContractOperation, _: &pb::ContractOperation| true
        );
    }
    members!(value, |kind,
                     id: &mut MemberIdentity|
     -> std::result::Result<(), ConnectionError> {
        if let Some(id) = id {
            let parsed = id.value.parse::<MemberId>().map_err(connection_error)?;
            if previous.is_some() && !trusted && old.get(&parsed) != Some(&kind) {
                return Err(ConnectionError::Member(format!(
                    "member {parsed} is not active in this member kind; omit IDs for additions"
                )));
            }
            if old.get(&parsed).is_some_and(|old_kind| *old_kind != kind) {
                return Err(ConnectionError::Member("member kind cannot change".into()));
            }
        } else {
            *id = Some(MemberId::new().into());
        }
        Ok(())
    });
    if let Some(prior) = prior {
        for artifact in &value.artifacts {
            if let Some(old) = prior
                .artifacts
                .iter()
                .find(|a| a.member_id == artifact.member_id)
            {
                require(old.content == artifact.content && old.format == artifact.format && old.format_version == artifact.format_version && old.media_type == artifact.media_type && old.digest == artifact.digest,
                    "artifact bytes, format, media type and digest are immutable within member identity").map_err(connection_error)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn equivalent_manifest(
    previous: &Entity,
    proposed: &Entity,
) -> std::result::Result<bool, ConnectionError> {
    validate_stored_members(previous)?;
    let Some(desired) = spec(proposed) else {
        return Ok(false);
    };
    connections::validate_stored_connection_ids(previous)?;
    active(desired, false).map_err(connection_error)?;
    let mut comparison = previous.clone();
    if active(desired, false).map_err(connection_error)?.is_empty() {
        if let Some(value) = spec_mut(&mut comparison) {
            members!(
                value,
                |_, id: &mut MemberIdentity| -> std::result::Result<(), ConnectionError> {
                    *id = None;
                    Ok(())
                }
            );
        }
    }
    if crate::semantically_equal(&comparison, proposed) {
        return Ok(true);
    }
    connections::equivalent_with_unassigned_ids(&comparison, proposed)
}

pub(crate) fn removed_members(
    base: Option<&Entity>,
    opposing: Option<&Entity>,
    selected: &Entity,
) -> std::result::Result<HashSet<MemberId>, ConnectionError> {
    for entity in base
        .into_iter()
        .chain(opposing)
        .chain(std::iter::once(selected))
    {
        validate_stored_members(entity)?;
    }
    let same = |e: &&Entity| {
        e.system
            .as_ref()
            .zip(selected.system.as_ref())
            .is_some_and(|(a, b)| !a.uid.is_empty() && a.uid == b.uid)
    };
    let Some(base) = base.filter(same).and_then(spec) else {
        return Ok(HashSet::new());
    };
    let Some(opposing) = opposing.filter(same).and_then(spec) else {
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
    removed: &HashSet<MemberId>,
) -> std::result::Result<(), ConnectionError> {
    if let Some(value) = spec(entity) {
        if let Some(id) = active(value, true)
            .map_err(connection_error)?
            .keys()
            .find(|id| removed.contains(id))
        {
            return Err(ConnectionError::Member(format!(
                "member {id} from branch base was removed on opposing side"
            )));
        }
    }
    Ok(())
}

pub(crate) fn restore_members(
    entity: &mut Entity,
    removed: &HashSet<MemberId>,
) -> std::result::Result<(), ConnectionError> {
    let Some(value) = spec_mut(entity) else {
        return Ok(());
    };
    let mut replacements = HashMap::new();
    members!(value, |_,
                     id: &mut MemberIdentity|
     -> std::result::Result<(), ConnectionError> {
        let parsed = member_id(id.as_ref()).map_err(connection_error)?;
        if removed.contains(&parsed) {
            let fresh: pb::ContractMemberId = MemberId::new().into();
            replacements.insert(parsed.to_string(), fresh.clone());
            *id = Some(fresh);
        }
        Ok(())
    });
    let rewrite = |id: &mut MemberIdentity| {
        if let Some(fresh) = id.as_ref().and_then(|id| replacements.get(&id.value)) {
            *id = Some(fresh.clone());
        }
    };
    for schema in &mut value.schemas {
        if let Some(schema_definition::Target::Artifact(artifact)) =
            schema.definition.as_mut().and_then(|d| d.target.as_mut())
        {
            rewrite(&mut artifact.artifact_id);
        }
    }
    for message in &mut value.messages {
        for binding in &mut message.schemas {
            if let Some(schema_binding::Type::LocalSchema(id)) = binding.r#type.as_mut() {
                if let Some(fresh) = replacements.get(&id.value) {
                    *id = fresh.clone();
                }
            }
        }
    }
    for operation in &mut value.operations {
        for message in &mut operation.messages {
            rewrite(&mut message.message_id);
        }
    }
    for exposure in &mut value.exposures {
        rewrite(&mut exposure.operation_id);
    }
    for usage in &mut value.usages {
        rewrite(&mut usage.operation_id);
    }
    Ok(())
}

#[derive(Debug)]
pub enum ContractMember<'a> {
    Artifact(&'a pb::ContractArtifact),
    Schema(&'a pb::ContractSchema),
    Message(&'a pb::ContractMessage),
    Operation(&'a pb::ContractOperation),
}

pub fn resolve_member<'a>(
    entity: &'a Entity,
    reference: &pb::ContractMemberRef,
) -> Result<ContractMember<'a>> {
    let value =
        spec(entity).ok_or_else(|| SpecificationError("owner is not a Specification".into()))?;
    require(
        reference
            .specification
            .as_ref()
            .and_then(|r| r.id.as_ref())
            .is_some_and(|id| Some(id) == value.id.as_ref())
            && !reference.owner_uid.is_empty()
            && entity
                .system
                .as_ref()
                .is_some_and(|s| s.uid == reference.owner_uid),
        "member reference does not identify exact owner incarnation",
    )?;
    active(value, true)?;
    let id = member_id(reference.member_id.as_ref())?;
    let matches = |member: &Option<pb::ContractMemberId>| {
        member.as_ref().is_some_and(|m| m.value == id.to_string())
    };
    require(
        reference.kind == Kind::Artifact as i32 || reference.expected_digest.is_none(),
        "only artifact member references may pin a digest",
    )?;
    match Kind::try_from(reference.kind) {
        Ok(Kind::Artifact) => {
            let member = value
                .artifacts
                .iter()
                .find(|m| matches(&m.member_id))
                .ok_or_else(|| SpecificationError("artifact member missing".into()))?;
            if let Some(digest) = &reference.expected_digest {
                require(
                    member.digest.as_ref() == Some(digest)
                        && content_digest(&member.content) == *digest,
                    "artifact digest pin mismatch",
                )?;
            }
            Ok(ContractMember::Artifact(member))
        }
        Ok(Kind::Schema) => value
            .schemas
            .iter()
            .find(|m| matches(&m.member_id))
            .map(ContractMember::Schema)
            .ok_or_else(|| SpecificationError("schema member missing".into())),
        Ok(Kind::Message) => value
            .messages
            .iter()
            .find(|m| matches(&m.member_id))
            .map(ContractMember::Message)
            .ok_or_else(|| SpecificationError("message member missing".into())),
        Ok(Kind::Operation) => value
            .operations
            .iter()
            .find(|m| matches(&m.member_id))
            .map(ContractMember::Operation)
            .ok_or_else(|| SpecificationError("operation member missing".into())),
        _ => Err(SpecificationError("invalid member kind".into())),
    }
}

pub fn resolve_type<'a>(
    entity: &'a Entity,
    reference: &pb::ContractTypeRef,
) -> Result<&'a pb::ContractSchema> {
    let reference_schema = reference
        .schema
        .as_ref()
        .ok_or_else(|| SpecificationError("missing schema member reference".into()))?;
    let ContractMember::Schema(schema) = resolve_member(entity, reference_schema)? else {
        return Err(SpecificationError(
            "type reference must select schema member".into(),
        ));
    };
    let definition = schema
        .definition
        .as_ref()
        .ok_or_else(|| SpecificationError("missing schema definition".into()))?;
    let Some(schema_definition::Target::Artifact(selection)) = definition.target.as_ref() else {
        return Err(SpecificationError(
            "external contract type must select artifact-backed schema".into(),
        ));
    };
    member_id(reference.expected_artifact_id.as_ref())?;
    require(
        reference.expected_digest.is_some()
            && selection.artifact_id == reference.expected_artifact_id
            && selection.expected_digest == reference.expected_digest
            && definition.selector == reference.expected_selector,
        "schema artifact identity, digest or selector pin mismatch",
    )?;
    let value = spec(entity).ok_or_else(|| SpecificationError("missing Specification".into()))?;
    let artifact = value
        .artifacts
        .iter()
        .find(|a| a.member_id == selection.artifact_id)
        .ok_or_else(|| SpecificationError("selected artifact missing".into()))?;
    require(
        artifact.digest == reference.expected_digest
            && Some(content_digest(&artifact.content)) == artifact.digest,
        "selected artifact bytes mismatch",
    )?;
    Ok(schema)
}

fn exact(id: Option<&pb::Id>) -> Result<()> {
    let id = id.ok_or_else(|| SpecificationError("missing exact entity reference".into()))?;
    require(
        id.version > 0
            && crate::is_safe_id_component(&id.namespace)
            && crate::is_safe_id_component(&id.slug),
        "invalid exact entity reference",
    )
}
fn provider(target: Option<&pb::ProviderTarget>) -> Result<()> {
    match target.and_then(|p| p.target.as_ref()) {
        Some(pb::provider_target::Target::Component(r)) => exact(r.id.as_ref()),
        Some(pb::provider_target::Target::ExternalSystem(r)) => exact(r.id.as_ref()),
        None => Err(SpecificationError("missing provider target".into())),
    }
}
fn environment_name_ok(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.chars().any(char::is_control)
        && !name.trim().is_empty()
}

fn validate_schema_variants(value: &Specification) -> Result<()> {
    let mut groups: HashMap<(String, i32), (HashSet<String>, u32)> = HashMap::new();
    for message in &value.messages {
        let member = message
            .member_id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        for binding in &message.schemas {
            require(
                binding.environments.len() <= 32,
                "schema binding environment limit exceeded",
            )?;
            let mut seen = HashSet::new();
            for environment in &binding.environments {
                require(
                    environment_name_ok(&environment.value),
                    "environment name must be 1..=64 UTF-8 bytes, nonblank, and free of Unicode controls",
                )?;
                require(
                    seen.insert(environment.value.clone()),
                    "environment names must be unique within a schema binding",
                )?;
            }
            if binding.is_default == Some(true) {
                require(
                    binding.environments.is_empty(),
                    "default schema binding cannot name environments",
                )?;
            }
            let group = groups.entry((member.clone(), binding.role)).or_default();
            for environment in &binding.environments {
                require(
                    group.0.insert(environment.value.clone()),
                    "environment names must be unique per message and role",
                )?;
            }
            if binding.is_default == Some(true) {
                group.1 += 1;
                require(
                    group.1 == 1,
                    "at most one default schema binding per message and role",
                )?;
            }
        }
    }
    Ok(())
}

fn validate_message_channels(
    value: &Specification,
    ids: &HashMap<MemberId, Kind>,
) -> Result<()> {
    for declaration in &value.message_channels {
        let id = member_id(declaration.member_id.as_ref())?;
        require(
            ids.get(&id) == Some(&Kind::Message),
            format!("missing or wrong-kind local member {id}"),
        )?;
        exact(declaration.channel.as_ref().and_then(|channel| channel.id.as_ref()))?;
        if let Some(mode) = declaration.delivery_mode {
            require(
                matches!(
                    pb::ClaimDeliveryMode::try_from(mode),
                    Ok(pb::ClaimDeliveryMode::Push
                        | pb::ClaimDeliveryMode::Pull
                        | pb::ClaimDeliveryMode::PushPull)
                ),
                "delivery mode must be push, pull, or push_pull when supplied",
            )?;
        }
        if let Some(doc) = &declaration.doc {
            require(
                !doc.trim().is_empty(),
                "message channel doc must be nonblank when supplied",
            )?;
        }
        require(
            declaration.parameter_values.keys().all(|name| !name.is_empty()),
            "message channel parameter names must be nonblank",
        )?;
    }
    Ok(())
}

fn selector(value: Option<&pb::OpaqueTypeSelector>) -> Result<()> {
    if let Some(value) = value {
        require(
            !value.syntax.trim().is_empty() && !value.value.trim().is_empty(),
            "opaque selector requires syntax and value",
        )?;
    }
    Ok(())
}

/// Validate native structure and all same-owner selections. External targets require authorized backend resolution.
pub fn validate_specification(value: &Specification) -> Result<()> {
    exact(value.id.as_ref())?;
    require(
        !value.title.trim().is_empty(),
        "Specification title is required",
    )?;
    require(
        !value.artifacts.is_empty() || !value.schemas.is_empty() || !value.messages.is_empty(),
        "Specification requires artifact, schema or message",
    )?;
    require(
        value.artifacts.len() + value.schemas.len() + value.messages.len() + value.operations.len()
            <= 1024,
        "Specification member limit exceeded",
    )?;
    require(
        value.encoded_len() <= 1024 * 1024,
        "Specification encoded size limit exceeded",
    )?;
    let ids = active(value, true)?;
    let local = |id: Option<&pb::ContractMemberId>, kind| -> Result<()> {
        let id = member_id(id)?;
        require(
            ids.get(&id) == Some(&kind),
            format!("missing or wrong-kind local member {id}"),
        )
    };
    for names in [
        value
            .artifacts
            .iter()
            .map(|m| m.name.as_ref())
            .collect::<Vec<_>>(),
        value.schemas.iter().map(|m| m.name.as_ref()).collect(),
        value.messages.iter().map(|m| m.name.as_ref()).collect(),
        value.operations.iter().map(|m| m.name.as_ref()).collect(),
    ] {
        let mut seen = HashSet::new();
        for name in names {
            let name = name.ok_or_else(|| SpecificationError("member name is required".into()))?;
            require(
                !name.value.trim().is_empty() && seen.insert(&name.value),
                "member names must be nonblank and unique within kind",
            )?;
        }
    }
    let mut paths = HashSet::new();
    let mut bytes = 0;
    for artifact in &value.artifacts {
        let path = artifact
            .path
            .as_ref()
            .ok_or_else(|| SpecificationError("artifact path required".into()))?;
        require(
            !path.value.contains('\\')
                && path
                    .value
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != "..")
                && paths.insert(&path.value),
            "artifact path must be unique relative POSIX path",
        )?;
        require(
            !artifact.content.is_empty() && artifact.content.len() <= 256 * 1024,
            "artifact source text empty or exceeds limit",
        )?;
        bytes += artifact.content.len();
        require(
            artifact.digest.as_ref() == Some(&content_digest(&artifact.content)),
            "artifact digest mismatch",
        )?;
        require(
            artifact
                .media_type
                .as_ref()
                .is_some_and(|m| !m.value.trim().is_empty() && m.value.is_ascii()),
            "media type must be nonblank ASCII",
        )?;
        if let Some(version) = &artifact.format_version {
            require(
                !version.value.trim().is_empty(),
                "format version must be nonblank",
            )?;
        }
        match artifact.format.as_ref().and_then(|f| f.kind.as_ref()) {
            Some(pb::contract_format::Kind::Known(k)) => require(
                pb::contract_format::Known::try_from(*k).is_ok() && *k != 0,
                "invalid known contract format",
            )?,
            Some(pb::contract_format::Kind::Other(name)) => {
                require(!name.trim().is_empty(), "other format name required")?;
            }
            None => return Err(SpecificationError("contract format required".into())),
        }
    }
    require(bytes <= 512 * 1024, "total artifact byte limit exceeded")?;
    for schema in &value.schemas {
        let definition = schema
            .definition
            .as_ref()
            .ok_or_else(|| SpecificationError("schema definition required".into()))?;
        selector(definition.selector.as_ref())?;
        match definition.target.as_ref() {
            Some(schema_definition::Target::Artifact(selection)) => {
                local(selection.artifact_id.as_ref(), Kind::Artifact)?;
                let artifact = value
                    .artifacts
                    .iter()
                    .find(|a| a.member_id == selection.artifact_id)
                    .ok_or_else(|| SpecificationError("artifact missing".into()))?;
                require(
                    selection.expected_digest.is_some()
                        && selection.expected_digest == artifact.digest,
                    "schema definition artifact digest mismatch",
                )?;
            }
            Some(schema_definition::Target::Schema(r)) => {
                exact(r.id.as_ref())?;
                require(
                    definition.selector.is_none(),
                    "native Schema definition cannot have opaque selector",
                )?;
            }
            None => {
                return Err(SpecificationError(
                    "schema definition target required".into(),
                ));
            }
        }
    }
    for message in &value.messages {
        if let Some(mapping) = &message.mapping {
            exact(mapping.read_model.as_ref().and_then(|r| r.id.as_ref()))?;
        }
        for binding in &message.schemas {
            require(
                pb::schema_binding::Role::try_from(binding.role).is_ok() && binding.role != 0,
                "schema binding role required",
            )?;
            match binding.r#type.as_ref() {
                Some(schema_binding::Type::LocalSchema(id)) => local(Some(id), Kind::Schema)?,
                Some(schema_binding::Type::Schema(r)) => exact(r.id.as_ref())?,
                Some(schema_binding::Type::LibraryType(r)) => {
                    exact(r.library.as_ref().and_then(|r| r.id.as_ref()))?;
                    let name = r
                        .message
                        .as_ref()
                        .map(|n| n.value.as_str())
                        .unwrap_or_default();
                    require(
                        !name.is_empty()
                            && name.split('.').all(|s| {
                                !s.is_empty()
                                    && (s.as_bytes()[0].is_ascii_alphabetic() || s.starts_with('_'))
                                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                            }),
                        "qualified protobuf name required",
                    )?;
                }
                Some(schema_binding::Type::ContractType(r)) => {
                    let owner = r.schema.as_ref().ok_or_else(|| {
                        SpecificationError("contract type schema required".into())
                    })?;
                    exact(owner.specification.as_ref().and_then(|s| s.id.as_ref()))?;
                    require(
                        owner.specification.as_ref().and_then(|s| s.id.as_ref()) != value.id.as_ref(),
                        "same-owner schema selections must use LocalSchema instead of external contract type pins",
                    )?;
                    require(
                        owner.kind == Kind::Schema as i32
                            && !owner.owner_uid.trim().is_empty()
                            && owner.expected_digest.is_none(),
                        "contract type requires schema incarnation selection",
                    )?;
                    member_id(owner.member_id.as_ref())?;
                    member_id(r.expected_artifact_id.as_ref())?;
                    require(
                        r.expected_digest.as_ref().is_some_and(|d| {
                            d.sha256.len() == 64
                                && d.sha256
                                    .bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        }),
                        "contract type digest required",
                    )?;
                    selector(r.expected_selector.as_ref())?;
                }
                None => return Err(SpecificationError("schema binding type required".into())),
            }
        }
    }
    validate_schema_variants(value)?;
    validate_message_channels(value, &ids)?;
    for operation in &value.operations {
        require(
            !operation.messages.is_empty(),
            "operation requires message occurrence",
        )?;
        for occurrence in &operation.messages {
            local(occurrence.message_id.as_ref(), Kind::Message)?;
            require(
                pb::operation_message::Role::try_from(occurrence.role).is_ok()
                    && occurrence.role != 0,
                "operation message role required",
            )?;
        }
        if let Some(binding) = &operation.binding {
            match binding.kind.as_ref() {
                Some(operation_binding::Kind::Http(b)) => {
                    require(
                        !b.method.is_empty()
                            && b.method.bytes().all(|c| {
                                c.is_ascii_uppercase()
                                    || c.is_ascii_digit()
                                    || b"!#$%&'*+-.^_`|~".contains(&c)
                            })
                            && b.path.starts_with('/'),
                        "invalid HTTP method or path",
                    )?;
                    let mut seen = HashSet::new();
                    require(
                        b.response_status_codes
                            .iter()
                            .all(|s| (100..=599).contains(s) && seen.insert(s)),
                        "HTTP statuses must be distinct and in 100..599",
                    )?;
                }
                Some(operation_binding::Kind::Rpc(b)) => require(
                    !b.service.trim().is_empty() && !b.method.trim().is_empty(),
                    "RPC service and method required",
                )?,
                Some(operation_binding::Kind::Graphql(b)) => require(
                    pb::graphql_operation_binding::Kind::try_from(b.kind).is_ok()
                        && b.kind != 0
                        && !b.field.trim().is_empty(),
                    "GraphQL kind and field required",
                )?,
                Some(operation_binding::Kind::Channel(b)) => {
                    exact(b.channel.as_ref().and_then(|r| r.id.as_ref()))?;
                }
                Some(operation_binding::Kind::Other(b)) => {
                    let name = b
                        .protocol
                        .as_ref()
                        .map(|p| p.value.as_str())
                        .unwrap_or_default();
                    require(
                        !name.is_empty()
                            && name.as_bytes()[0].is_ascii_lowercase()
                            && name.bytes().all(|b| {
                                b.is_ascii_lowercase() || b.is_ascii_digit() || b".+-".contains(&b)
                            })
                            && !b.description.trim().is_empty(),
                        "other binding protocol and description required",
                    )?;
                }
                None => {
                    return Err(SpecificationError(
                        "operation binding alternative required".into(),
                    ));
                }
            }
        }
    }
    let mut providers = HashSet::new();
    for exposure in &value.exposures {
        provider(exposure.provider.as_ref())?;
        local(exposure.operation_id.as_ref(), Kind::Operation)?;
        require(
            providers.insert((
                exposure.provider.as_ref().map(Message::encode_to_vec),
                exposure.operation_id.as_ref().map(|id| &id.value),
            )),
            "duplicate provider-operation exposure",
        )?;
    }
    for usage in &value.usages {
        provider(usage.caller.as_ref())?;
        local(usage.operation_id.as_ref(), Kind::Operation)?;
        if let Some(id) = &usage.exposure_id {
            let exposure = value
                .exposures
                .iter()
                .find(|e| e.connection_id.as_ref() == Some(id))
                .ok_or_else(|| SpecificationError("usage exposure missing".into()))?;
            require(
                exposure.operation_id == usage.operation_id,
                "usage exposure selects different operation",
            )?;
        }
    }
    let mut dependencies = HashSet::new();
    for dependency in &value.dependencies {
        let (kind, id) = match dependency.target.as_ref() {
            Some(contract_dependency::Target::Specification(r)) => {
                (pb::EntityKind::Specification, r.id.as_ref())
            }
            Some(contract_dependency::Target::TypeLibrary(r)) => {
                (pb::EntityKind::TypeLibrary, r.id.as_ref())
            }
            None => return Err(SpecificationError("dependency target required".into())),
        };
        exact(id)?;
        require(
            kind != pb::EntityKind::Specification || id != value.id.as_ref(),
            "redundant self-dependency",
        )?;
        require(
            dependencies.insert((kind, id.map(|id| (&id.namespace, &id.slug, id.version)))),
            "duplicate dependency",
        )?;
    }
    let entity = Entity {
        kind: Some(entity::Kind::Specification(value.clone())),
        system: None,
    };
    connections::validate_stored_connection_ids(&entity)
        .map_err(|e| SpecificationError(e.to_string()))?;
    require(
        connections::connection_occurrences(&entity)
            .map_err(|e| SpecificationError(e.to_string()))?
            .len()
            <= 4096,
        "Specification association limit exceeded",
    )?;
    Ok(())
}

pub(crate) fn rewrite_exposure_selections(previous: &Entity, proposed: &mut Entity) {
    let Some(prior) = spec(previous) else {
        return;
    };
    let Some(value) = spec_mut(proposed) else {
        return;
    };
    let changed: HashMap<_, _> = prior
        .exposures
        .iter()
        .zip(&value.exposures)
        .filter_map(|(old, new)| {
            Some((
                old.connection_id.as_ref()?.value.clone(),
                new.connection_id.clone()?,
            ))
        })
        .collect();
    for usage in &mut value.usages {
        if let Some(replacement) = usage
            .exposure_id
            .as_ref()
            .and_then(|id| changed.get(&id.value))
        {
            usage.exposure_id = Some(replacement.clone());
        }
    }
}
