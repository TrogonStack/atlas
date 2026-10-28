//! Pure ownership and reconciliation decisions over complete native values.

use std::collections::BTreeSet;

use prost::Message;
use prost_reflect::{DynamicMessage, FieldDescriptor, ReflectMessage, Value};
use trogon_atlas_proto::{
    source_field_ownership::Policy, source_selection, Entity, SourceFieldOwnership,
    SourceOwnedField, SourceRetentionPolicy, SourceRunBudgets, SourceSelection,
};

use crate::{refs, transcode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("source ownership policy is required")]
    MissingOwnership,
    #[error("source ownership unit is unsupported for this native kind")]
    UnsupportedUnit,
    #[error("source ownership units overlap")]
    OverlappingUnits,
    #[error("source native identity differs")]
    Identity,
    #[error("source native value cannot be decoded")]
    NativeValue,
    #[error("source budget is exhausted or invalid")]
    Budget,
    #[error("source retention policy is invalid")]
    Retention,
    #[error("local source selection cannot claim native owner fields")]
    LocalOwnership,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipUnit {
    WholeOwner,
    Field(SourceOwnedField),
}
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedValue {
    unit: OwnershipUnit,
    value: DynamicMessage,
}

impl OwnedValue {
    pub fn message(&self) -> &DynamicMessage {
        &self.value
    }
}

fn native(
    entity: &Entity,
) -> Result<(DynamicMessage, FieldDescriptor, DynamicMessage), SourceError> {
    let descriptor = transcode::message_descriptor("trogonatlas.eventmodel.v1alpha1.Entity")
        .map_err(|_| SourceError::NativeValue)?;
    let wrapper = DynamicMessage::decode(descriptor, entity.encode_to_vec().as_slice())
        .map_err(|_| SourceError::NativeValue)?;
    let kind = wrapper
        .descriptor()
        .oneofs()
        .find(|o| o.name() == "kind")
        .and_then(|o| o.fields().find(|f| wrapper.has_field(f)))
        .ok_or(SourceError::NativeValue)?;
    let Value::Message(value) = wrapper.get_field(&kind).into_owned() else {
        return Err(SourceError::NativeValue);
    };
    Ok((wrapper, kind, value))
}
fn unit_fields(
    value: &DynamicMessage,
    unit: OwnershipUnit,
) -> Result<Vec<FieldDescriptor>, SourceError> {
    let descriptor = value.descriptor();
    if unit == OwnershipUnit::WholeOwner
        && descriptor.full_name() == "trogonatlas.eventmodel.v1alpha1.Decision"
        && ["authored_on", "decided_on", "decision_makers", "amends"]
            .iter()
            .any(|name| value.has_field_by_name(name))
    {
        return Err(SourceError::UnsupportedUnit);
    }
    if unit == OwnershipUnit::Field(SourceOwnedField::Metadata)
        && crate::agent_metadata::metadata_has_additions(value)
            .map_err(|_| SourceError::NativeValue)?
    {
        return Err(SourceError::UnsupportedUnit);
    }
    if matches!(
        descriptor.full_name(),
        "trogonatlas.eventmodel.v1alpha1.Component"
            | "trogonatlas.eventmodel.v1alpha1.ExternalSystem"
    ) {
        if unit == OwnershipUnit::Field(SourceOwnedField::Relationships) {
            return Err(SourceError::UnsupportedUnit);
        }
        if unit == OwnershipUnit::WholeOwner
            && ["relationships", "communications"].iter().any(|name| {
                value
                    .get_field_by_name(name)
                    .as_deref()
                    .and_then(Value::as_list)
                    .is_some_and(|values| !values.is_empty())
            })
        {
            return Err(SourceError::UnsupportedUnit);
        }
    }
    if unit == OwnershipUnit::WholeOwner
        && descriptor.full_name() == "trogonatlas.eventmodel.v1alpha1.Term"
        && value.get_field_by_name("owning_domain").is_some_and(|_| {
            descriptor
                .get_field_by_name("owning_domain")
                .is_some_and(|field| value.has_field(&field))
        })
    {
        return Err(SourceError::UnsupportedUnit);
    }
    match unit {
        OwnershipUnit::WholeOwner => Ok(descriptor.fields().filter(|f| f.name() != "id").collect()),
        OwnershipUnit::Field(field) => {
            if field == SourceOwnedField::Unspecified {
                return Err(SourceError::UnsupportedUnit);
            }
            let name = field
                .as_str_name()
                .strip_prefix("SOURCE_OWNED_FIELD_")
                .ok_or(SourceError::UnsupportedUnit)?
                .to_ascii_lowercase();
            if let Some(field) = descriptor.get_field_by_name(&name) {
                if field.name() == "id"
                    || field.containing_oneof().is_some_and(|o| !o.is_synthetic())
                {
                    return Err(SourceError::UnsupportedUnit);
                }
                return Ok(vec![field]);
            }
            descriptor
                .oneofs()
                .find(|o| o.name() == name && !o.is_synthetic())
                .map(|o| o.fields().collect())
                .ok_or(SourceError::UnsupportedUnit)
        }
    }
}
pub fn validate_ownership(
    entity: &Entity,
    ownership: &SourceFieldOwnership,
) -> Result<Vec<OwnershipUnit>, SourceError> {
    let units = match ownership
        .policy
        .as_ref()
        .ok_or(SourceError::MissingOwnership)?
    {
        Policy::None(_) => Vec::new(),
        Policy::WholeOwner(_) => vec![OwnershipUnit::WholeOwner],
        Policy::Fields(fields) => {
            if fields.fields.is_empty() {
                return Err(SourceError::MissingOwnership);
            }
            fields
                .fields
                .iter()
                .map(|f| {
                    SourceOwnedField::try_from(*f)
                        .map(OwnershipUnit::Field)
                        .map_err(|_| SourceError::UnsupportedUnit)
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    let (_, _, value) = native(entity)?;
    let mut occupied = BTreeSet::new();
    for unit in &units {
        for field in unit_fields(&value, *unit)? {
            if !occupied.insert(field.number()) {
                return Err(SourceError::OverlappingUnits);
            }
        }
    }
    Ok(units)
}

pub fn validate_create_ownership(
    entity: &Entity,
    ownership: &SourceFieldOwnership,
) -> Result<(), SourceError> {
    if let Some(trogon_atlas_proto::entity::Kind::Decision(value)) = &entity.kind {
        if value.authored_on.is_some()
            || value.decided_on.is_some()
            || !value.decision_makers.is_empty()
            || !value.amends.is_empty()
        {
            return Err(SourceError::UnsupportedUnit);
        }
    }
    validate_ownership(entity, ownership)?;
    Ok(())
}
pub fn validate_selection_ownership(
    selection: &SourceSelection,
    ownership: &SourceFieldOwnership,
) -> Result<(), SourceError> {
    if let Some(source_selection::Selection::Connection(reference)) = selection.selection.as_ref() {
        if reference.owner.as_ref().is_some_and(|owner| {
            matches!(
                trogon_atlas_proto::EntityKind::try_from(owner.kind),
                Ok(trogon_atlas_proto::EntityKind::Component
                    | trogon_atlas_proto::EntityKind::ExternalSystem)
            )
        }) {
            return Err(SourceError::UnsupportedUnit);
        }
    }
    match selection
        .selection
        .as_ref()
        .ok_or(SourceError::NativeValue)?
    {
        source_selection::Selection::Entity(_) => Ok(()),
        _ if matches!(ownership.policy, Some(Policy::None(_))) => Ok(()),
        _ => Err(SourceError::LocalOwnership),
    }
}
pub fn extract_unit(entity: &Entity, unit: OwnershipUnit) -> Result<OwnedValue, SourceError> {
    let (_, _, value) = native(entity)?;
    let mut selected = DynamicMessage::new(value.descriptor());
    for field in unit_fields(&value, unit)? {
        if value.has_field(&field) {
            selected.set_field(&field, value.get_field(&field).into_owned());
        }
    }
    Ok(OwnedValue {
        unit,
        value: selected,
    })
}
pub fn replace_unit(
    entity: &mut Entity,
    unit: OwnershipUnit,
    selected: &OwnedValue,
) -> Result<(), SourceError> {
    let (mut wrapper, kind, mut value) = native(entity)?;
    if selected.unit != unit || value.descriptor() != selected.value.descriptor() {
        return Err(SourceError::UnsupportedUnit);
    }
    let fields = unit_fields(&value, unit)?;
    for field in &fields {
        value.clear_field(field);
    }
    for field in fields {
        if selected.value.has_field(&field) {
            value.set_field(&field, selected.value.get_field(&field).into_owned());
        }
    }
    wrapper.set_field(&kind, Value::Message(value));
    *entity =
        Entity::decode(wrapper.encode_to_vec().as_slice()).map_err(|_| SourceError::NativeValue)?;
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeDisposition {
    Unchanged,
    Update,
    LocalDrift,
    Convergent,
    Conflict,
}
#[derive(Debug, Clone, PartialEq)]
pub struct UnitDecision {
    pub unit: OwnershipUnit,
    pub disposition: MergeDisposition,
}
#[derive(Debug, Clone, PartialEq)]
pub struct MergePlan {
    pub proposed: Entity,
    pub decisions: Vec<UnitDecision>,
}
impl MergePlan {
    #[must_use]
    pub fn has_conflicts(&self) -> bool {
        self.decisions
            .iter()
            .any(|d| d.disposition == MergeDisposition::Conflict)
    }
}
fn charge_value(value: &Value, budget: &mut RunBudget) -> Result<(), SourceError> {
    match value {
        Value::List(values) => {
            budget.charge(
                BudgetWork::Examinations,
                u64::try_from(values.len()).map_err(|_| SourceError::Budget)?,
            )?;
            for value in values {
                charge_value(value, budget)?;
            }
        }
        Value::Map(values) => {
            budget.charge(
                BudgetWork::Examinations,
                u64::try_from(values.len()).map_err(|_| SourceError::Budget)?,
            )?;
            for value in values.values() {
                charge_value(value, budget)?;
            }
        }
        Value::Message(message) => {
            for (_, value) in message.fields() {
                charge_value(value, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn plan_merge(
    base_source: &Entity,
    base_native: &Entity,
    incoming: &Entity,
    current: &Entity,
    ownership: &SourceFieldOwnership,
    budget: &mut RunBudget,
) -> Result<MergePlan, SourceError> {
    plan_merge_with_comparator(
        base_source,
        base_native,
        incoming,
        current,
        ownership,
        budget,
        |a, b| Ok::<_, SourceError>(a == b),
    )
}

pub fn plan_merge_with_comparator<E: From<SourceError>>(
    base_source: &Entity,
    base_native: &Entity,
    incoming: &Entity,
    current: &Entity,
    ownership: &SourceFieldOwnership,
    budget: &mut RunBudget,
    mut equivalent: impl FnMut(&OwnedValue, &OwnedValue) -> Result<bool, E>,
) -> Result<MergePlan, E> {
    for value in [base_source, base_native, incoming] {
        if refs::entity_kind(value) != refs::entity_kind(current)
            || refs::entity_id(value) != refs::entity_id(current)
        {
            return Err(SourceError::Identity.into());
        }
    }
    let units = validate_ownership(current, ownership)?;
    let mut plan = MergePlan {
        proposed: current.clone(),
        decisions: Vec::new(),
    };
    for unit in units {
        budget.charge(BudgetWork::Examinations, 1)?;
        let before_source = extract_unit(base_source, unit)?;
        let before_native = extract_unit(base_native, unit)?;
        let source = extract_unit(incoming, unit)?;
        let native = extract_unit(current, unit)?;
        for value in [&before_source, &before_native, &source, &native] {
            for (_, field) in value.value.fields() {
                charge_value(field, budget)?;
            }
        }
        let source_unchanged = equivalent(&source, &before_source)?;
        let native_unchanged = equivalent(&native, &before_native)?;
        let disposition = match (source_unchanged, native_unchanged) {
            (true, true) => MergeDisposition::Unchanged,
            (false, true) => {
                replace_unit(&mut plan.proposed, unit, &source)?;
                MergeDisposition::Update
            }
            (true, false) => MergeDisposition::LocalDrift,
            (false, false) if equivalent(&source, &native)? => MergeDisposition::Convergent,
            (false, false) => MergeDisposition::Conflict,
        };
        budget.charge(BudgetWork::ResultRecords, 1)?;
        plan.decisions.push(UnitDecision { unit, disposition });
    }
    Ok(plan)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetWork {
    SuppliedBytes,
    Files,
    Resources,
    Relations,
    Mappings,
    Owners,
    Examinations,
    NativeOperations,
    MappingUpdates,
    ControlTransitions,
    ResultRecords,
    ResultBytes,
}
#[derive(Debug, Clone)]
pub struct RunBudget {
    limits: SourceRunBudgets,
    used: [u64; 12],
}
impl RunBudget {
    pub fn new(limits: SourceRunBudgets) -> Result<Self, SourceError> {
        validate_budgets(&limits)?;
        Ok(Self {
            limits,
            used: [0; 12],
        })
    }
    pub fn charge(&mut self, work: BudgetWork, amount: u64) -> Result<(), SourceError> {
        let (index, ceiling) = match work {
            BudgetWork::SuppliedBytes => (0, self.limits.supplied_bytes),
            BudgetWork::Files => (1, u64::from(self.limits.files)),
            BudgetWork::Resources => (2, u64::from(self.limits.resources)),
            BudgetWork::Relations => (3, u64::from(self.limits.relations)),
            BudgetWork::Mappings => (4, u64::from(self.limits.mappings)),
            BudgetWork::Owners => (5, u64::from(self.limits.hydrated_owners)),
            BudgetWork::Examinations => (6, u64::from(self.limits.examined_occurrences)),
            BudgetWork::NativeOperations => (7, u64::from(self.limits.native_operations)),
            BudgetWork::MappingUpdates => (8, u64::from(self.limits.mapping_updates)),
            BudgetWork::ControlTransitions => (9, u64::from(self.limits.control_transitions)),
            BudgetWork::ResultRecords => (10, u64::from(self.limits.result_records)),
            BudgetWork::ResultBytes => (11, self.limits.result_bytes),
        };
        let used = self.used[index]
            .checked_add(amount)
            .filter(|n| *n <= ceiling)
            .ok_or(SourceError::Budget)?;
        self.used[index] = used;
        Ok(())
    }
    pub fn check_elapsed(&self, elapsed_ms: u64) -> Result<(), SourceError> {
        if elapsed_ms > self.limits.phase_timeout_ms {
            Err(SourceError::Budget)
        } else {
            Ok(())
        }
    }
}
#[must_use]
pub fn profile_budgets() -> SourceRunBudgets {
    SourceRunBudgets {
        supplied_bytes: 32 * 1024 * 1024,
        files: 256,
        resources: 1000,
        relations: 10_000,
        mappings: 4096,
        hydrated_owners: 5000,
        examined_occurrences: 10_000,
        native_operations: 1000,
        mapping_updates: 4096,
        control_transitions: 64,
        result_records: 4096,
        result_bytes: 8 * 1024 * 1024,
        phase_timeout_ms: 60_000,
        page_records: 500,
        page_bytes: 4 * 1024 * 1024,
    }
}
pub fn validate_budgets(budget: &SourceRunBudgets) -> Result<(), SourceError> {
    let max = profile_budgets();
    let values = [
        (budget.supplied_bytes, max.supplied_bytes),
        (u64::from(budget.files), u64::from(max.files)),
        (u64::from(budget.resources), u64::from(max.resources)),
        (u64::from(budget.relations), u64::from(max.relations)),
        (u64::from(budget.mappings), u64::from(max.mappings)),
        (
            u64::from(budget.hydrated_owners),
            u64::from(max.hydrated_owners),
        ),
        (
            u64::from(budget.examined_occurrences),
            u64::from(max.examined_occurrences),
        ),
        (
            u64::from(budget.native_operations),
            u64::from(max.native_operations),
        ),
        (
            u64::from(budget.mapping_updates),
            u64::from(max.mapping_updates),
        ),
        (
            u64::from(budget.control_transitions),
            u64::from(max.control_transitions),
        ),
        (
            u64::from(budget.result_records),
            u64::from(max.result_records),
        ),
        (budget.result_bytes, max.result_bytes),
        (budget.phase_timeout_ms, max.phase_timeout_ms),
        (u64::from(budget.page_records), u64::from(max.page_records)),
        (budget.page_bytes, max.page_bytes),
    ];
    if values.iter().any(|(v, c)| *v == 0 || v > c) {
        return Err(SourceError::Budget);
    }
    Ok(())
}
#[must_use]
pub fn profile_retention() -> SourceRetentionPolicy {
    let week = 7 * 24 * 60 * 60;
    let month = 30 * 24 * 60 * 60;
    SourceRetentionPolicy {
        registration_bytes: 128 * 1024 * 1024,
        unaccepted_snapshot_seconds: week,
        preview_seconds: week,
        accepted_snapshot_seconds: month,
        base_seconds: month,
        settled_outcome_seconds: month,
        marker_seconds: month,
        audit_seconds: month,
        raw_bytes_seconds: month,
        overlay_seconds: month,
    }
}
pub fn validate_retention(policy: &SourceRetentionPolicy) -> Result<(), SourceError> {
    let max = profile_retention();
    let values = [
        (policy.registration_bytes, max.registration_bytes),
        (
            policy.unaccepted_snapshot_seconds,
            max.unaccepted_snapshot_seconds,
        ),
        (policy.preview_seconds, max.preview_seconds),
        (
            policy.accepted_snapshot_seconds,
            max.accepted_snapshot_seconds,
        ),
        (policy.base_seconds, max.base_seconds),
        (policy.settled_outcome_seconds, max.settled_outcome_seconds),
        (policy.marker_seconds, max.marker_seconds),
        (policy.audit_seconds, max.audit_seconds),
        (policy.raw_bytes_seconds, max.raw_bytes_seconds),
        (policy.overlay_seconds, max.overlay_seconds),
    ];
    if values.iter().any(|(v, c)| *v == 0 || v > c) {
        return Err(SourceError::Retention);
    }
    Ok(())
}
