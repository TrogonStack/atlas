//! Native flow and design topology: step identity, bounded validation and
//! intra-owner transition structure. Execution semantics are never implied.

use std::collections::{BTreeMap, HashMap, HashSet};

use prost::Message;
use trogon_atlas_proto::{self as pb, entity, DesignGraph, Entity, EntityKind};

use crate::{
    connections::{self, ConnectionError},
    decision_document::{
        self, exact, text, validate_catalog_text, validate_catalog_title, CatalogError,
        LocalValueId,
    },
    responsibility,
};

pub const MAX_STEPS: usize = 512;
pub const MAX_TRANSITIONS_PER_STEP: usize = 64;
pub const MAX_SUBJECTS: usize = 1_024;
pub const MAX_LOCAL_IDS: usize = 2_048;
pub const MAX_ENCODED_BYTES: usize = 1_048_576;
pub const MAX_CUSTOM_PROPERTIES: usize = 32;
pub const MAX_TRAVERSAL_DEPTH: u32 = 32;
pub const DEFAULT_TRAVERSAL_DEPTH: u32 = 8;

type Result<T> = std::result::Result<T, CatalogError>;

fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(CatalogError(message.into()))
    }
}
fn connection_error(error: CatalogError) -> ConnectionError {
    ConnectionError::Catalog(error.0)
}

#[must_use]
pub fn graph(entity: &Entity) -> Option<&DesignGraph> {
    match entity.kind.as_ref() {
        Some(entity::Kind::DesignGraph(value)) => Some(value),
        _ => None,
    }
}
fn graph_mut(entity: &mut Entity) -> Option<&mut DesignGraph> {
    match entity.kind.as_mut() {
        Some(entity::Kind::DesignGraph(value)) => Some(value),
        _ => None,
    }
}

/// One authored transition with its owning step, in authored order.
#[derive(Debug, Clone, PartialEq)]
pub struct TopologyEdge {
    pub step_index: usize,
    pub transition_index: usize,
    pub source_step_id: Option<pb::LocalValueId>,
    pub connection_id: Option<pb::ConnectionId>,
    pub target_step_id: Option<pb::LocalValueId>,
    pub label: Option<pb::CatalogTitle>,
}

#[must_use]
pub fn transitions(value: &DesignGraph) -> Vec<TopologyEdge> {
    value
        .steps
        .iter()
        .enumerate()
        .flat_map(|(step_index, step)| {
            step.transitions
                .iter()
                .enumerate()
                .map(move |(transition_index, transition)| TopologyEdge {
                    step_index,
                    transition_index,
                    source_step_id: step.step_id.clone(),
                    connection_id: transition.connection_id.clone(),
                    target_step_id: transition.target_step_id.clone(),
                    label: transition.label.clone(),
                })
        })
        .collect()
}

fn step_inventory(value: &DesignGraph, persisted: bool) -> Result<HashMap<LocalValueId, usize>> {
    require(value.steps.len() <= MAX_STEPS, "step limit exceeded")?;
    let mut out = HashMap::new();
    for (index, step) in value.steps.iter().enumerate() {
        match &step.step_id {
            Some(id) => {
                let id = id.value.parse::<LocalValueId>()?;
                require(out.insert(id, index).is_none(), "duplicate step ID")?;
            }
            None => require(!persisted, "persisted step ID missing")?,
        }
    }
    Ok(out)
}

pub fn validate_stored_steps(entity: &Entity) -> Result<()> {
    if let Some(value) = graph(entity) {
        step_inventory(value, true)?;
    }
    Ok(())
}

/// Fresh, unambiguous HTTPS locator without credentials.
fn https(value: &str) -> Result<()> {
    responsibility::validate_contact(&pb::ContactPoint {
        channel: Some(pb::contact_point::Channel::Web(pb::ContactUrl {
            value: value.to_owned(),
        })),
        label: None,
    })
    .map_err(|error| CatalogError(error.to_string()))
}

fn entity_target(reference: &pb::EntityRef, kind: EntityKind, message: &'static str) -> Result<()> {
    exact(reference.id.as_ref())?;
    require(reference.kind == kind as i32, message)
}

fn validate_target(target: &pb::TopologyStepTarget, owner: Option<&pb::Id>) -> Result<()> {
    use pb::topology_step_target::Selection as S;
    match target.selection.as_ref().ok_or_else(|| {
        CatalogError("target selection required; omit target for a generic step".into())
    })? {
        S::CatalogMessage(subject) => {
            decision_document::validate_subject(subject)?;
            match subject.selection.as_ref() {
                Some(pb::catalog_subject::Selection::Member(member)) => {
                    require(
                        member.kind == pb::ContractMemberKind::Message as i32,
                        "catalog message member must be a MESSAGE",
                    )?;
                }
                Some(pb::catalog_subject::Selection::Entity(reference)) => {
                    require(
                        matches!(
                            EntityKind::try_from(reference.kind),
                            Ok(EntityKind::Event | EntityKind::Command)
                        ),
                        "catalog message entity must be an Event or Command",
                    )?;
                }
                _ => {
                    return Err(CatalogError(
                        "catalog message must select a MESSAGE member, Event or Command".into(),
                    ));
                }
            }
            Ok(())
        }
        S::Component(r) => entity_target(r, EntityKind::Component, "component target kind"),
        S::Channel(r) => entity_target(r, EntityKind::Channel, "channel target kind"),
        S::DataStore(r) => entity_target(r, EntityKind::DataStore, "data store target kind"),
        S::DataProduct(r) => entity_target(r, EntityKind::DataProduct, "data product target kind"),
        S::SystemGroup(r) => exact(r.id.as_ref()),
        S::Subflow(r) => {
            exact(r.id.as_ref())?;
            require(r.id.as_ref() != owner, "self subflow forbidden")
        }
        S::Actor(actor) => {
            validate_catalog_title(actor.name.as_ref())?;
            optional_text(actor.summary.as_ref())
        }
        S::Custom(custom) => {
            validate_catalog_title(custom.title.as_ref())?;
            optional_text(custom.summary.as_ref())?;
            require(
                custom.icon.is_empty()
                    || (custom.icon.len() <= 64
                        && custom.icon.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b)
                        })),
                "icon must be a lowercase token",
            )?;
            if !custom.url.is_empty() {
                https(&custom.url)?;
            }
            require(
                custom.properties.len() <= MAX_CUSTOM_PROPERTIES,
                "custom property limit exceeded",
            )?;
            for (key, value) in &custom.properties {
                text(key, 128)?;
                require(
                    value.len() <= 1_024 && !value.chars().any(char::is_control),
                    "custom property value outside bounds",
                )?;
            }
            Ok(())
        }
        S::ExternalSystem(system) => {
            validate_catalog_title(system.name.as_ref())?;
            optional_text(system.summary.as_ref())?;
            if !system.url.is_empty() {
                https(&system.url)?;
            }
            Ok(())
        }
    }
}
fn optional_text(value: Option<&pb::CatalogText>) -> Result<()> {
    value.map_or(Ok(()), |value| validate_catalog_text(Some(value)))
}

/// Validate persisted structure. Step IDs may be absent before backend preparation;
/// supplied IDs, transition endpoints and entries must resolve inside the owner.
pub fn validate_values(entity: &Entity) -> Result<()> {
    let Some(value) = graph(entity) else {
        return Ok(());
    };
    exact(value.id.as_ref())?;
    validate_catalog_title(value.title.as_ref())?;
    if let Some(doc) = &value.doc {
        text(doc, 16_384)?;
    }
    require(
        matches!(
            pb::DesignGraphProfile::try_from(value.profile),
            Ok(pb::DesignGraphProfile::Flow | pb::DesignGraphProfile::Diagram)
        ),
        "design graph profile required",
    )?;
    require(!value.steps.is_empty(), "design graph requires steps")?;
    let ids = step_inventory(value, false)?;
    let mut transitions = 0usize;
    for step in &value.steps {
        validate_catalog_title(step.title.as_ref())?;
        optional_text(step.summary.as_ref())?;
        if let Some(target) = &step.target {
            validate_target(target, value.id.as_ref())?;
        }
        require(
            step.transitions.len() <= MAX_TRANSITIONS_PER_STEP,
            "transition limit exceeded",
        )?;
        transitions += step.transitions.len();
        for transition in &step.transitions {
            if transition.label.is_some() {
                validate_catalog_title(transition.label.as_ref())?;
            }
            let target = transition
                .target_step_id
                .as_ref()
                .ok_or_else(|| CatalogError("transition target step required".into()))?
                .value
                .parse::<LocalValueId>()?;
            require(
                ids.contains_key(&target),
                "transition target step missing from graph",
            )?;
        }
    }
    let mut entries = HashSet::new();
    for entry in &value.entry_steps {
        let id = entry.value.parse::<LocalValueId>()?;
        require(ids.contains_key(&id), "entry step missing from graph")?;
        require(entries.insert(id), "duplicate entry step")?;
    }
    if let Some(layout) = &value.layout_document {
        exact(layout.id.as_ref())?;
    }
    decision_document::validate_subjects(entity, false)?;
    require(
        value.steps.len() + transitions + value.subjects.len() <= MAX_LOCAL_IDS,
        "local identity limit exceeded",
    )?;
    require(
        entity.encoded_len() <= MAX_ENCODED_BYTES,
        "design graph byte limit exceeded",
    )?;
    connections::validate_connection_ids(entity).map_err(|e| CatalogError(e.to_string()))
}

#[derive(Eq, Hash, PartialEq)]
struct StepSignature {
    value: Vec<u8>,
    properties: BTreeMap<String, String>,
}

fn value_signature(step: &pb::TopologyStep) -> StepSignature {
    let mut value = step.clone();
    value.step_id = None;
    value.transitions.clear();
    // Protobuf map encoding order does not define value identity.
    let properties = match value
        .target
        .as_mut()
        .and_then(|target| target.selection.as_mut())
    {
        Some(pb::topology_step_target::Selection::Custom(custom)) => {
            std::mem::take(&mut custom.properties).into_iter().collect()
        }
        _ => BTreeMap::new(),
    };
    StepSignature {
        value: value.encode_to_vec(),
        properties,
    }
}

pub(crate) fn prepare_steps(
    previous: Option<&Entity>,
    proposed: &mut Entity,
    trusted: bool,
) -> std::result::Result<(), ConnectionError> {
    let prior = previous.and_then(graph);
    let old = prior
        .map(|value| step_inventory(value, true))
        .transpose()
        .map_err(connection_error)?
        .unwrap_or_default();
    let Some(value) = graph_mut(proposed) else {
        return Ok(());
    };
    let supplied = step_inventory(value, false).map_err(connection_error)?;
    if let Some(prior) = prior {
        let idless: HashSet<_> = value
            .steps
            .iter()
            .filter(|step| step.step_id.is_none())
            .map(value_signature)
            .collect();
        let stripped = prior.steps.iter().any(|step| {
            step.step_id
                .as_ref()
                .and_then(|id| id.value.parse::<LocalValueId>().ok())
                .is_some_and(|id| !supplied.contains_key(&id))
                && idless.contains(&value_signature(step))
        });
        require(
            !stripped,
            "removed step reintroduced without its ID; retain the ID or remove and add in separate writes",
        )
        .map_err(connection_error)?;
    }
    for step in &mut value.steps {
        match &step.step_id {
            Some(id) => {
                let id = id.value.parse::<LocalValueId>().map_err(connection_error)?;
                require(
                    previous.is_none() || trusted || old.contains_key(&id),
                    "step ID not active; omit IDs for additions",
                )
                .map_err(connection_error)?;
            }
            None => step.step_id = Some(LocalValueId::new().into()),
        }
    }
    Ok(())
}

pub(crate) fn equivalent_manifest(
    previous: &Entity,
    proposed: &Entity,
) -> std::result::Result<bool, ConnectionError> {
    let Some(desired) = graph(proposed) else {
        return Ok(false);
    };
    validate_stored_steps(previous).map_err(connection_error)?;
    connections::validate_stored_connection_ids(previous)?;
    let supplied = step_inventory(desired, false).map_err(connection_error)?;
    let mut comparison = previous.clone();
    if supplied.is_empty() {
        if let Some(value) = graph_mut(&mut comparison) {
            for step in &mut value.steps {
                step.step_id = None;
            }
        }
    }
    if crate::semantically_equal(&comparison, proposed) {
        return Ok(true);
    }
    connections::equivalent_with_unassigned_ids(&comparison, proposed)
}

pub(crate) fn removed_steps(
    base: Option<&Entity>,
    opposing: Option<&Entity>,
    selected: &Entity,
) -> std::result::Result<HashSet<LocalValueId>, ConnectionError> {
    for owner in base
        .into_iter()
        .chain(opposing)
        .chain(std::iter::once(selected))
    {
        validate_stored_steps(owner).map_err(connection_error)?;
    }
    let same = |e: &&Entity| {
        e.system
            .as_ref()
            .zip(selected.system.as_ref())
            .is_some_and(|(a, b)| !a.uid.is_empty() && a.uid == b.uid)
    };
    let (Some(base), Some(opposing)) = (
        base.filter(same).and_then(graph),
        opposing.filter(same).and_then(graph),
    ) else {
        return Ok(HashSet::new());
    };
    let current = step_inventory(opposing, true).map_err(connection_error)?;
    Ok(step_inventory(base, true)
        .map_err(connection_error)?
        .into_keys()
        .filter(|id| !current.contains_key(id))
        .collect())
}

pub(crate) fn reject_removed(
    entity: &Entity,
    removed: &HashSet<LocalValueId>,
) -> std::result::Result<(), ConnectionError> {
    let Some(value) = graph(entity) else {
        return Ok(());
    };
    require(
        !step_inventory(value, true)
            .map_err(connection_error)?
            .keys()
            .any(|id| removed.contains(id)),
        "step ID from branch base removed on opposing side",
    )
    .map_err(connection_error)
}

pub(crate) fn restore_steps(
    entity: &mut Entity,
    removed: &HashSet<LocalValueId>,
) -> std::result::Result<(), ConnectionError> {
    let Some(value) = graph_mut(entity) else {
        return Ok(());
    };
    let mut map = HashMap::new();
    for step in &mut value.steps {
        let id = step
            .step_id
            .as_ref()
            .ok_or_else(|| ConnectionError::Catalog("step ID missing".into()))?
            .value
            .parse::<LocalValueId>()
            .map_err(connection_error)?;
        if removed.contains(&id) {
            let fresh: pb::LocalValueId = LocalValueId::new().into();
            map.insert(id, fresh.clone());
            step.step_id = Some(fresh);
        }
    }
    let rewrite = |id: &mut pb::LocalValueId| -> std::result::Result<(), ConnectionError> {
        let parsed = id.value.parse::<LocalValueId>().map_err(connection_error)?;
        if let Some(fresh) = map.get(&parsed) {
            *id = fresh.clone();
        }
        Ok(())
    };
    for entry in &mut value.entry_steps {
        rewrite(entry)?;
    }
    for step in &mut value.steps {
        for transition in &mut step.transitions {
            if let Some(target) = &mut transition.target_step_id {
                rewrite(target)?;
            }
        }
    }
    Ok(())
}
