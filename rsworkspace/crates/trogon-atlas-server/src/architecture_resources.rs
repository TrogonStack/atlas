use std::collections::{HashMap, HashSet};

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    refs::{entity_id, entity_kind},
    StoredEntity,
};

struct Checks {
    subject: Option<pb::EntityRef>,
    issues: Vec<pb::ValidationIssue>,
}

impl Checks {
    fn require(&mut self, valid: bool, code: &str, field: &str, message: &str) {
        if !valid {
            self.issues.push(super::issue_error(
                code,
                message.into(),
                self.subject.clone(),
                field,
            ));
        }
    }

    fn value(&mut self, valid: bool, field: &str) {
        self.require(
            valid,
            "ARCHITECTURE_VALUE_INVALID",
            field,
            "invalid architecture resource declaration",
        );
    }

    fn supersedes(
        &mut self,
        kind: pb::EntityKind,
        current: Option<&pb::Id>,
        prior: Option<&pb::Id>,
        context: &ArchitectureContext<'_>,
    ) {
        if let Some(prior) = prior {
            self.value(
                current.is_some_and(|current| {
                    valid_id(prior)
                        && prior.namespace == current.namespace
                        && prior.slug == current.slug
                        && prior.version < current.version
                }),
                "supersedes",
            );
            self.target(kind, Some(prior), context, "supersedes");
        }
    }

    fn target(
        &mut self,
        kind: pb::EntityKind,
        id: Option<&pb::Id>,
        context: &ArchitectureContext<'_>,
        field: &str,
    ) {
        self.require(
            id.is_some_and(|id| valid_id(id) && context.find(kind, id).is_some()),
            "ARCHITECTURE_REFERENCE_UNRESOLVED",
            field,
            "architecture resource target is absent or invalid",
        );
    }
}

fn valid_id(id: &pb::Id) -> bool {
    id.version > 0
        && trogon_atlas_core::validate_id_component(&id.namespace).is_ok()
        && trogon_atlas_core::validate_id_component(&id.slug).is_ok()
}

#[derive(Eq, Hash, PartialEq)]
struct EntityLookupKey<'a> {
    kind: pb::EntityKind,
    namespace: &'a str,
    slug: &'a str,
    version: u64,
}

impl<'a> EntityLookupKey<'a> {
    fn new(kind: pb::EntityKind, id: &'a pb::Id) -> Self {
        Self {
            kind,
            namespace: &id.namespace,
            slug: &id.slug,
            version: id.version,
        }
    }
}

pub(crate) struct ArchitectureContext<'a> {
    entities: HashMap<EntityLookupKey<'a>, &'a pb::Entity>,
}

impl<'a> ArchitectureContext<'a> {
    pub(crate) fn new(entities: impl IntoIterator<Item = &'a StoredEntity>) -> Self {
        Self {
            entities: entities
                .into_iter()
                .filter_map(|stored| {
                    let entity = &stored.entity;
                    Some((
                        EntityLookupKey::new(entity_kind(entity)?, entity_id(entity)?),
                        entity,
                    ))
                })
                .collect(),
        }
    }

    pub(crate) fn find(&self, kind: pb::EntityKind, id: &pb::Id) -> Option<&'a pb::Entity> {
        self.entities.get(&EntityLookupKey::new(kind, id)).copied()
    }
}

pub(crate) fn is_resource(entity: &pb::Entity) -> bool {
    matches!(
        entity.kind,
        Some(
            pb::entity::Kind::Component(_)
                | pb::entity::Kind::Domain(_)
                | pb::entity::Kind::ExternalSystem(_)
                | pb::entity::Kind::Channel(_)
                | pb::entity::Kind::DataStore(_)
                | pb::entity::Kind::Specification(_)
                | pb::entity::Kind::DataProduct(_)
                | pb::entity::Kind::Team(_)
                | pb::entity::Kind::Person(_)
                | pb::entity::Kind::Decision(_)
                | pb::entity::Kind::Term(_)
                | pb::entity::Kind::Document(_)
                | pb::entity::Kind::DesignGraph(_)
        )
    )
}

fn parameter_name(value: &str) -> bool {
    let mut chars = value.bytes();
    chars
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && chars.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn protocol_name(value: &str) -> bool {
    let mut chars = value.bytes();
    chars.next().is_some_and(|b| b.is_ascii_lowercase())
        && chars.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".+-".contains(&b))
}

fn placeholders(template: &str) -> Option<HashSet<&str>> {
    let mut rest = template;
    let mut names = HashSet::new();
    while let Some(index) = rest.find(['{', '}']) {
        if rest.as_bytes()[index] != b'{' {
            return None;
        }
        rest = &rest[index + 1..];
        let end = rest.find('}')?;
        let name = &rest[..end];
        if !parameter_name(name) {
            return None;
        }
        names.insert(name);
        rest = &rest[end + 1..];
    }
    Some(names)
}

fn address(channel: &pb::Channel, checks: &mut Checks) {
    let Some(address) = channel.address.as_ref() else {
        checks.value(
            channel
                .bindings
                .iter()
                .all(|b| b.parameter_values.is_empty()),
            "bindings.parameter_values",
        );
        return;
    };
    checks.value(!address.template.trim().is_empty(), "address.template");
    let placeholders = placeholders(&address.template);
    checks.value(placeholders.is_some(), "address.template");
    let mut parameters = HashMap::new();
    for parameter in &address.parameters {
        let name = parameter.name.as_ref().map_or("", |n| n.value.as_str());
        checks.value(
            parameter_name(name) && parameters.insert(name, parameter).is_none(),
            "address.parameters.name",
        );
        if let Some(allowed) = &parameter.allowed_values {
            checks.value(
                !allowed.values.is_empty()
                    && allowed.values.iter().collect::<HashSet<_>>().len() == allowed.values.len(),
                "address.parameters.allowed_values",
            );
            checks.value(
                parameter
                    .default
                    .as_ref()
                    .is_none_or(|value| allowed.values.contains(value))
                    && parameter
                        .examples
                        .iter()
                        .all(|value| allowed.values.contains(value)),
                "address.parameters.default",
            );
        }
        checks.value(
            parameter.examples.iter().collect::<HashSet<_>>().len() == parameter.examples.len(),
            "address.parameters.examples",
        );
    }
    if let Some(placeholders) = placeholders {
        checks.value(
            placeholders.len() == parameters.len()
                && placeholders
                    .iter()
                    .all(|name| parameters.contains_key(name)),
            "address.parameters",
        );
    }
    for binding in &channel.bindings {
        checks.value(
            binding
                .parameter_values
                .keys()
                .all(|name| parameters.contains_key(name.as_str())),
            "bindings.parameter_values",
        );
        for (name, parameter) in &parameters {
            let value = binding
                .parameter_values
                .get(*name)
                .or(parameter.default.as_ref());
            checks.value(
                value.is_some_and(|value| {
                    parameter
                        .allowed_values
                        .as_ref()
                        .is_none_or(|allowed| allowed.values.contains(value))
                }),
                "bindings.parameter_values",
            );
        }
    }
}

fn observation(reference: Option<&pb::ConnectionRef>, context: &ArchitectureContext<'_>) -> bool {
    let Some(reference) = reference else {
        return false;
    };
    let Some(owner_ref) = reference.owner.as_ref() else {
        return false;
    };
    if owner_ref.kind != pb::EntityKind::AutomationSlice as i32 {
        return false;
    }
    let Some(owner_id) = owner_ref.id.as_ref().filter(|id| valid_id(id)) else {
        return false;
    };
    let Some(owner) = context.find(pb::EntityKind::AutomationSlice, owner_id) else {
        return false;
    };
    let Ok(occurrence) = trogon_atlas_core::connections::resolve_connection(owner, reference)
    else {
        return false;
    };
    let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_ref() else {
        return false;
    };
    let selected = slice
        .source_read_models
        .iter()
        .find(|edge| edge.connection_id.as_ref() == reference.id.as_ref());
    let Some(read_model) = selected
        .and_then(|edge| edge.read_model.as_ref())
        .and_then(|r| r.id.as_ref())
    else {
        return false;
    };
    let Some(processor) = slice
        .processor
        .as_ref()
        .and_then(|edge| edge.processor.as_ref())
        .and_then(|r| r.id.as_ref())
    else {
        return false;
    };
    occurrence
        .target
        .as_ref()
        .is_some_and(|target| target.kind == pb::EntityKind::ReadModel as i32)
        && valid_id(read_model)
        && valid_id(processor)
        && context
            .find(pb::EntityKind::ReadModel, read_model)
            .is_some()
        && context.find(pb::EntityKind::Processor, processor).is_some()
}

#[must_use]
pub fn validate(entity: &pb::Entity, all: &[StoredEntity]) -> Vec<pb::ValidationIssue> {
    validate_with_previous(entity, all, None)
}

pub fn validate_with_previous(
    entity: &pb::Entity,
    all: &[StoredEntity],
    previous: Option<&pb::Entity>,
) -> Vec<pb::ValidationIssue> {
    if !is_resource(entity) {
        return Vec::new();
    }
    validate_in_context_with_previous(entity, &ArchitectureContext::new(all), previous)
}

pub(crate) fn validate_admission_with_previous(
    entity: &pb::Entity,
    all: &[StoredEntity],
    previous: Option<&pb::Entity>,
) -> Vec<pb::ValidationIssue> {
    validate_shipped_in_context(entity, &ArchitectureContext::new(all), previous)
}

pub(super) fn validate_in_context(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
) -> Vec<pb::ValidationIssue> {
    validate_in_context_with_previous(entity, context, None)
}

fn validate_in_context_with_previous(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
    previous: Option<&pb::Entity>,
) -> Vec<pb::ValidationIssue> {
    let mut issues = validate_shipped_in_context(entity, context, previous);
    if trogon_atlas_core::message_claims::has_content(entity)
        && crate::message_claims_resources::validate(entity, context).is_err()
    {
        issues.push(super::issue_error(
            "MESSAGE_CLAIMS_REFERENCE_UNRESOLVED",
            "Message claim selection is absent, invalid or unavailable".into(),
            super::slice_ref(entity),
            "message_claims",
        ));
    }
    issues
}

fn validate_shipped_in_context(
    entity: &pb::Entity,
    context: &ArchitectureContext<'_>,
    previous: Option<&pb::Entity>,
) -> Vec<pb::ValidationIssue> {
    if matches!(
        entity.kind,
        Some(pb::entity::Kind::Decision(_) | pb::entity::Kind::Document(_))
    ) {
        return crate::decision_document_resources::validate_in_context(entity, context);
    }
    if matches!(entity.kind, Some(pb::entity::Kind::DesignGraph(_))) {
        return crate::design_topology_resources::validate_in_context(entity, context);
    }
    if matches!(entity.kind, Some(pb::entity::Kind::DataProduct(_))) {
        return crate::data_product_resources::validate_in_context_with_previous(
            entity, context, previous,
        );
    }
    if matches!(entity.kind, Some(pb::entity::Kind::Specification(_))) {
        return crate::specification_resources::validate_in_context_with_previous(
            entity, context, previous,
        );
    }
    if matches!(
        entity.kind,
        Some(pb::entity::Kind::Team(_) | pb::entity::Kind::Person(_))
    ) {
        return crate::responsibility_resources::validate_in_context(entity, context);
    }
    let mut checks = Checks {
        subject: super::slice_ref(entity),
        issues: Vec::new(),
    };
    match entity.kind.as_ref() {
        Some(pb::entity::Kind::Component(_) | pb::entity::Kind::ExternalSystem(_)) => {
            checks.value(
                trogon_atlas_core::architectural_relationships::validate_values(entity).is_ok(),
                "relationships",
            );
            checks.value(
                trogon_atlas_core::communication_intent::validate_values(entity).is_ok(),
                "communications",
            );
            for (index, declaration) in
                trogon_atlas_core::communication_intent::declarations(entity)
                    .iter()
                    .enumerate()
            {
                if let Some(target) = declaration
                    .peer
                    .as_ref()
                    .and_then(trogon_atlas_core::architectural_relationships::endpoint_ref)
                {
                    if let Ok(kind) = pb::EntityKind::try_from(target.kind) {
                        checks.target(
                            kind,
                            target.id.as_ref(),
                            context,
                            &format!("communications[{index}].peer"),
                        );
                    }
                }
                checks.require(
                    declaration.message.as_ref().is_some_and(|selection| {
                        selection
                            .specification
                            .as_ref()
                            .and_then(|reference| reference.id.as_ref())
                            .and_then(|id| context.find(pb::EntityKind::Specification, id))
                            .is_some_and(|specification| {
                                trogon_atlas_core::specification::resolve_member(
                                    specification,
                                    selection,
                                )
                                .is_ok()
                            })
                    }),
                    "ARCHITECTURE_REFERENCE_UNRESOLVED",
                    &format!("communications[{index}].message"),
                    "communication message selection is absent or invalid",
                );
            }
            for (index, relationship) in
                trogon_atlas_core::architectural_relationships::relationships(entity)
                    .iter()
                    .enumerate()
            {
                if let Some(target) = relationship
                    .target
                    .as_ref()
                    .and_then(trogon_atlas_core::architectural_relationships::endpoint_ref)
                {
                    if let Ok(kind) = pb::EntityKind::try_from(target.kind) {
                        checks.target(
                            kind,
                            target.id.as_ref(),
                            context,
                            &format!("relationships[{index}].target"),
                        );
                    }
                }
            }
        }
        Some(pb::entity::Kind::Channel(channel)) => {
            checks.supersedes(
                pb::EntityKind::Channel,
                channel.id.as_ref(),
                channel.supersedes.as_ref(),
                context,
            );
            checks.value(channel.id.as_ref().is_some_and(valid_id), "id");
            checks.value(!channel.title.trim().is_empty(), "title");
            checks.value(
                channel.protocols.iter().all(|p| protocol_name(&p.value))
                    && channel
                        .protocols
                        .iter()
                        .map(|p| &p.value)
                        .collect::<HashSet<_>>()
                        .len()
                        == channel.protocols.len(),
                "protocols",
            );
            checks.value(
                channel.delivery_guarantee.is_none_or(|value| {
                    pb::channel::DeliveryGuarantee::try_from(value)
                        .is_ok_and(|value| value != pb::channel::DeliveryGuarantee::Unspecified)
                }),
                "delivery_guarantee",
            );
            address(channel, &mut checks);
            checks.value(!channel.bindings.is_empty(), "bindings");
            let mut seen = HashSet::new();
            for binding in &channel.bindings {
                let key = binding.observation.as_ref().map(|reference| {
                    let owner = reference.owner.as_ref();
                    (
                        owner.map(|o| o.kind),
                        owner.and_then(|o| o.id.as_ref()).map(pb::IdKey::new),
                        &reference.owner_uid,
                        reference.id.as_ref().map(|id| &id.value),
                    )
                });
                checks.value(seen.insert(key), "bindings.observation");
                checks.require(observation(binding.observation.as_ref(), context), "CHANNEL_OBSERVATION_UNRESOLVED", "bindings.observation", "binding must select a persisted AutomationSlice source ReadModel observation with resolvable endpoints");
            }
            match channel.broker.as_ref() {
                Some(pb::channel::Broker::Component(reference)) => checks.target(
                    pb::EntityKind::Component,
                    reference.id.as_ref(),
                    context,
                    "component",
                ),
                Some(pb::channel::Broker::ExternalSystem(reference)) => checks.target(
                    pb::EntityKind::ExternalSystem,
                    reference.id.as_ref(),
                    context,
                    "external_system",
                ),
                None => {}
            }
        }
        Some(pb::entity::Kind::DataStore(store)) => {
            checks.supersedes(
                pb::EntityKind::DataStore,
                store.id.as_ref(),
                store.supersedes.as_ref(),
                context,
            );
            checks.value(store.id.as_ref().is_some_and(valid_id), "id");
            checks.value(!store.title.trim().is_empty(), "title");
            checks.value(
                match store
                    .store_class
                    .as_ref()
                    .and_then(|class| class.kind.as_ref())
                {
                    Some(pb::store_class::Kind::Known(value)) => {
                        pb::store_class::Known::try_from(*value)
                            .is_ok_and(|value| value != pb::store_class::Known::Unspecified)
                    }
                    Some(pb::store_class::Kind::Other(name)) => !name.trim().is_empty(),
                    None => false,
                },
                "store_class",
            );
            checks.value(
                store
                    .technology
                    .as_ref()
                    .is_none_or(|technology| !technology.value.trim().is_empty()),
                "technology",
            );
            checks.value(
                store
                    .purpose
                    .as_ref()
                    .is_none_or(|purpose| !purpose.trim().is_empty()),
                "purpose",
            );
            checks.value(
                store.access_mode.is_none_or(|value| {
                    pb::data_store::AccessMode::try_from(value)
                        .is_ok_and(|value| value != pb::data_store::AccessMode::Unspecified)
                }),
                "access_mode",
            );
            checks.value(
                store.classification.is_none_or(|value| {
                    pb::data_store::Classification::try_from(value)
                        .is_ok_and(|value| value != pb::data_store::Classification::Unspecified)
                }),
                "classification",
            );
            if let Some(residency) = &store.residency {
                checks.value(
                    residency
                        .policy
                        .as_ref()
                        .is_some_and(|policy| !policy.value.trim().is_empty()),
                    "residency.policy",
                );
                if let Some(regions) = &residency.regions {
                    checks.value(
                        !regions.values.is_empty()
                            && regions
                                .values
                                .iter()
                                .all(|region| !region.value.trim().is_empty())
                            && regions
                                .values
                                .iter()
                                .map(|region| &region.value)
                                .collect::<HashSet<_>>()
                                .len()
                                == regions.values.len(),
                        "residency.regions",
                    );
                }
            }
            if let Some(retention) = &store.retention {
                checks.value(
                    retention
                        .policy
                        .as_ref()
                        .is_some_and(|policy| !policy.value.trim().is_empty()),
                    "retention.policy",
                );
                checks.value(
                    retention
                        .duration
                        .as_ref()
                        .is_none_or(|duration| duration.seconds > 0),
                    "retention.duration",
                );
            }
            checks.value(!store.attachments.is_empty(), "attachments");
            let mut seen = HashSet::new();
            for attachment in &store.attachments {
                let id = attachment
                    .read_model
                    .as_ref()
                    .and_then(|reference| reference.id.as_ref());
                checks.value(
                    seen.insert(id.map(pb::IdKey::new)),
                    "attachments.read_model",
                );
                checks.target(
                    pb::EntityKind::ReadModel,
                    id,
                    context,
                    "attachments.read_model",
                );
            }
        }
        Some(pb::entity::Kind::Term(term)) => {
            if let Some(reference) = &term.owning_domain {
                checks.target(
                    pb::EntityKind::Domain,
                    reference.id.as_ref(),
                    context,
                    "owning_domain",
                );
            }
        }
        _ => {}
    }
    super::enrich_issues_with_catalog(&mut checks.issues);
    checks.issues
}
