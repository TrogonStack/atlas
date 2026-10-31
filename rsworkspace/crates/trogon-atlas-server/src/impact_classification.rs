//! Breaking versus bump-only classification of an entity's dependents.
//!
//! Only referrers are dependents. Each reference site is mapped to the root
//! fields it consumes, and the typed diff between the two exact versions is
//! intersected with that consumption. Whatever cannot be proven compatible is
//! [`Classification::Unknown`], never [`Classification::BumpOnly`].

use std::collections::{BTreeSet, HashMap, VecDeque};

use prost::Message as _;
use trogon_atlas_core::{refs, schema};
use trogon_atlas_proto as pb;
use trogon_atlas_store::StoredEntity;

use crate::graph::ReverseIndex;

/// Ordered by severity, so a dependent takes the maximum of its parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Classification {
    BumpOnly,
    Unknown,
    Breaking,
}

impl Classification {
    fn to_proto(self) -> pb::DependentClassification {
        match self {
            Self::BumpOnly => pb::DependentClassification::BumpOnly,
            Self::Unknown => pb::DependentClassification::Unknown,
            Self::Breaking => pb::DependentClassification::Breaking,
        }
    }

    /// What a dependent inherits from a dependent it references: a bump
    /// cascades as a bump, anything worse is no longer provable.
    fn through(self) -> Self {
        match self {
            Self::BumpOnly => Self::BumpOnly,
            Self::Unknown | Self::Breaking => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnknownReason {
    NoConsumptionModel,
    OpaquePayload,
    OpaqueRootSchema,
    ConsumerContextMissing,
    UncoveredChange,
    UnsupportedRootKind,
    TransitiveThroughAffected,
    TraversalIncomplete,
}

impl UnknownReason {
    fn to_proto(self) -> pb::ClassificationUnknownReason {
        use pb::ClassificationUnknownReason as R;
        match self {
            Self::NoConsumptionModel => R::NoConsumptionModel,
            Self::OpaquePayload => R::OpaquePayload,
            Self::OpaqueRootSchema => R::OpaqueRootSchema,
            Self::ConsumerContextMissing => R::ConsumerContextMissing,
            Self::UncoveredChange => R::UncoveredChange,
            Self::UnsupportedRootKind => R::UnsupportedRootKind,
            Self::TransitiveThroughAffected => R::TransitiveThroughAffected,
            Self::TraversalIncomplete => R::TraversalIncomplete,
        }
    }
}

/// Why the dependent set itself may be incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Incompleteness {
    SnapshotTruncated,
    DepthLimited,
    HiddenDependents,
    UnresolvedReferrers,
}

impl Incompleteness {
    fn to_proto(self) -> pb::ImpactIncompleteness {
        match self {
            Self::SnapshotTruncated => pb::ImpactIncompleteness::SnapshotTruncated,
            Self::DepthLimited => pb::ImpactIncompleteness::DepthLimited,
            Self::HiddenDependents => pb::ImpactIncompleteness::HiddenDependents,
            Self::UnresolvedReferrers => pb::ImpactIncompleteness::UnresolvedReferrers,
        }
    }
}

/// Every referrer reachable from a root through incoming references only.
#[derive(Debug, Default)]
pub struct Dependents {
    depths: HashMap<pb::EntityKey, u32>,
    depth_limited: bool,
    unresolved: bool,
}

impl Dependents {
    /// Breadth-first over referrers of `root`, up to `max_depth` hops.
    /// `excluded` (the candidate version) is neither a dependent nor a hop.
    #[must_use]
    pub fn walk(
        index: &ReverseIndex,
        root: &pb::EntityKey,
        excluded: &pb::EntityKey,
        max_depth: u32,
    ) -> Self {
        let mut walk = Self::default();
        let mut queue = VecDeque::from([(root.clone(), 0_u32)]);
        while let Some((target, depth)) = queue.pop_front() {
            for reference in index.incoming(target.kind, &target.id()) {
                let Some(from) = referrer_key(reference) else {
                    walk.unresolved = true;
                    continue;
                };
                if &from == root || &from == excluded || walk.depths.contains_key(&from) {
                    continue;
                }
                if depth >= max_depth {
                    walk.depth_limited = true;
                    continue;
                }
                walk.depths.insert(from.clone(), depth + 1);
                queue.push_back((from, depth + 1));
            }
        }
        walk
    }

    #[must_use]
    pub fn depth_limited(&self) -> bool {
        self.depth_limited
    }

    #[must_use]
    pub fn has_unresolved_referrers(&self) -> bool {
        self.unresolved
    }

    /// Whether every dependent here is also a dependent in `other`.
    #[must_use]
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.depths.keys().all(|key| other.depths.contains_key(key))
    }
}

fn referrer_key(reference: &pb::Reference) -> Option<pb::EntityKey> {
    let from = reference.from.as_ref()?;
    let kind = pb::EntityKind::try_from(from.kind).ok()?;
    Some(pb::EntityKey::new(kind, from.id.as_ref()?))
}

/// The root's field list in diff path vocabulary, or why it has none.
enum RootShape {
    Fields {
        base: &'static str,
        current: Vec<pb::FieldSpec>,
    },
    OpaqueSchema,
    Unsupported,
}

/// One diff op, interpreted against the root's fields.
enum Change<'a> {
    Cosmetic,
    /// A field was removed or its type or cardinality changed.
    Field {
        name: String,
        op: &'a pb::DiffOp,
    },
    /// A field's metadata changed; its meaning is not judged here.
    Annotation {
        name: String,
        op: &'a pb::DiffOp,
    },
    Added {
        field: &'a pb::FieldSpec,
        op: &'a pb::DiffOp,
    },
    Uncovered(&'a pb::DiffOp),
}

struct RootChange<'a> {
    shape: RootShape,
    changes: Vec<Change<'a>>,
    uncovered: Vec<String>,
}

impl<'a> RootChange<'a> {
    fn new(
        current: &pb::Entity,
        candidate: &'a pb::Entity,
        candidate_fields: &'a [pb::FieldSpec],
        ops: &'a [pb::DiffOp],
    ) -> Self {
        let shape = root_shape(current, candidate);
        let changes = match &shape {
            RootShape::Fields { base, current } => ops
                .iter()
                .map(|op| categorize(op, base, current, candidate_fields))
                .collect(),
            RootShape::OpaqueSchema | RootShape::Unsupported => ops
                .iter()
                .map(|op| {
                    if is_cosmetic_path(&op.path) {
                        Change::Cosmetic
                    } else {
                        Change::Uncovered(op)
                    }
                })
                .collect(),
        };
        let uncovered = match shape {
            RootShape::Fields { .. } => uncovered_paths(current, candidate),
            RootShape::OpaqueSchema | RootShape::Unsupported => Vec::new(),
        };
        Self {
            shape,
            changes,
            uncovered,
        }
    }

    /// True only when the versions provably differ in documentation alone.
    fn is_cosmetic(&self) -> bool {
        matches!(self.shape, RootShape::Fields { .. })
            && self.uncovered.is_empty()
            && self.changes.iter().all(|c| matches!(c, Change::Cosmetic))
    }

    fn non_cosmetic_ops(&self) -> Vec<pb::DiffOp> {
        self.changes
            .iter()
            .filter_map(|change| match change {
                Change::Cosmetic => None,
                Change::Field { op, .. }
                | Change::Annotation { op, .. }
                | Change::Added { op, .. }
                | Change::Uncovered(op) => Some((*op).clone()),
            })
            .collect()
    }
}

fn root_shape(current: &pb::Entity, candidate: &pb::Entity) -> RootShape {
    use pb::entity::Kind as K;
    match (current.kind.as_ref(), candidate.kind.as_ref()) {
        (Some(K::Schema(current)), Some(K::Schema(_))) => RootShape::Fields {
            base: "fields",
            current: current.fields.clone(),
        },
        (Some(a), Some(b)) => match (schema::kind_schema(a), schema::kind_schema(b)) {
            (None, None) if !has_schema_slot(a) => RootShape::Unsupported,
            (a_schema, b_schema) => {
                if [a_schema, b_schema]
                    .into_iter()
                    .flatten()
                    .any(|any| !schema::is_schema(any))
                {
                    RootShape::OpaqueSchema
                } else {
                    RootShape::Fields {
                        base: "schema.fields",
                        current: schema::schema_fields(a_schema),
                    }
                }
            }
        },
        _ => RootShape::Unsupported,
    }
}

fn has_schema_slot(kind: &pb::entity::Kind) -> bool {
    use pb::entity::Kind as K;
    matches!(kind, K::Event(_) | K::Command(_) | K::ReadModel(_))
}

fn candidate_fields(candidate: &pb::Entity) -> Vec<pb::FieldSpec> {
    match candidate.kind.as_ref() {
        Some(pb::entity::Kind::Schema(s)) => s.fields.clone(),
        Some(kind) => schema::schema_fields(schema::kind_schema(kind)),
        None => Vec::new(),
    }
}

fn is_cosmetic_path(path: &str) -> bool {
    matches!(path, "title" | "doc" | "supersedes")
}

fn categorize<'a>(
    op: &'a pb::DiffOp,
    base: &str,
    current: &[pb::FieldSpec],
    candidate: &'a [pb::FieldSpec],
) -> Change<'a> {
    if is_cosmetic_path(&op.path) {
        return Change::Cosmetic;
    }
    let names = current
        .iter()
        .chain(candidate)
        .map(|field| field.name.as_str());
    for name in names {
        let field_path = format!("{base}[{name}]");
        if op.path == field_path {
            return if op.kind == pb::DiffOpKind::Removed as i32 {
                Change::Field {
                    name: name.to_owned(),
                    op,
                }
            } else if op.kind == pb::DiffOpKind::Added as i32 {
                candidate
                    .iter()
                    .find(|field| field.name == name)
                    .map_or(Change::Uncovered(op), |field| Change::Added { field, op })
            } else {
                Change::Uncovered(op)
            };
        }
        let Some(suffix) = op
            .path
            .strip_prefix(field_path.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
        else {
            continue;
        };
        let name = name.to_owned();
        return match suffix {
            "doc" => Change::Cosmetic,
            "kind" | "repeated" | "optional" => Change::Field { name, op },
            "metadata" => Change::Annotation { name, op },
            _ => Change::Uncovered(op),
        };
    }
    Change::Uncovered(op)
}

/// Parts of a field-bearing root that differ but that the typed diff does
/// not compare. Fields not listed are compared together, so a change the
/// list misses still surfaces.
fn uncovered_paths(current: &pb::Entity, candidate: &pb::Entity) -> Vec<String> {
    use pb::entity::Kind as K;
    let mut paths = Vec::new();
    let mut check = |path: &str, differs: bool| {
        if differs {
            paths.push(path.to_owned());
        }
    };
    match (current.kind.as_ref(), candidate.kind.as_ref()) {
        (Some(K::Event(a)), Some(K::Event(b))) => {
            check("swimlane", a.swimlane != b.swimlane);
            check("metadata", a.metadata != b.metadata);
            check(
                "schema.metadata",
                inline_schema_rest(a.schema.as_ref()) != inline_schema_rest(b.schema.as_ref()),
            );
            let pb::Event {
                id: _,
                title: _,
                doc: _,
                swimlane: _,
                metadata: _,
                supersedes: _,
                schema: _,
            } = a;
        }
        (Some(K::Command(a)), Some(K::Command(b))) => {
            check("swimlane", a.swimlane != b.swimlane);
            check("metadata", a.metadata != b.metadata);
            check(
                "schema.metadata",
                inline_schema_rest(a.schema.as_ref()) != inline_schema_rest(b.schema.as_ref()),
            );
            let pb::Command {
                id: _,
                title: _,
                doc: _,
                swimlane: _,
                metadata: _,
                supersedes: _,
                schema: _,
            } = a;
        }
        (Some(K::ReadModel(a)), Some(K::ReadModel(b))) => {
            check("source_events", a.source_events != b.source_events);
            check("external_source", a.external_source != b.external_source);
            check("metadata", a.metadata != b.metadata);
            check("key", a.key != b.key);
            check(
                "schema.metadata",
                inline_schema_rest(a.schema.as_ref()) != inline_schema_rest(b.schema.as_ref()),
            );
            let pb::ReadModel {
                id: _,
                title: _,
                doc: _,
                source_events: _,
                external_source: _,
                metadata: _,
                supersedes: _,
                schema: _,
                key: _,
            } = a;
        }
        (Some(K::Schema(a)), Some(K::Schema(b))) => {
            check("metadata", schema_rest(a) != schema_rest(b));
        }
        _ => {}
    }
    paths
}

/// An inline schema minus what the typed diff already compares.
fn inline_schema_rest(any: Option<&prost_types::Any>) -> Option<pb::Schema> {
    let any = any.filter(|any| schema::is_schema(any))?;
    pb::Schema::decode(any.value.as_slice())
        .ok()
        .map(|s| schema_rest(&s))
}

fn schema_rest(s: &pb::Schema) -> pb::Schema {
    pb::Schema {
        id: None,
        title: String::new(),
        doc: String::new(),
        fields: Vec::new(),
        supersedes: None,
        ..s.clone()
    }
}

/// Which root fields a reference site relies on.
enum Consumption {
    /// The site only names the root; its shape is irrelevant.
    Identity,
    Fields {
        names: BTreeSet<String>,
        additions: Additions,
    },
    /// The site embeds the root's whole shape (a typed field reference).
    WholeShape,
    Unknown(UnknownReason),
}

/// When a field added to the root forces the consumer to change.
enum Additions {
    Ignored,
    /// A required field breaks the consumer; an optional one does not.
    Required,
    /// Every added field must be handled, as a projection must project it.
    All,
    /// The consumer must source every root field: an addition breaks unless
    /// one of these names provides it or the field is declared derived.
    UnlessSourced(BTreeSet<String>),
}

impl Additions {
    fn breaks(&self, added: &pb::FieldSpec) -> bool {
        match self {
            Self::Ignored => false,
            Self::Required => !added.optional,
            Self::All => true,
            Self::UnlessSourced(names) => {
                !names.contains(&added.name) && !crate::validation::is_derived_field(added)
            }
        }
    }
}

/// Exact-version lookup of the entities a site's consumption depends on.
struct Model<'a> {
    by_key: HashMap<pb::EntityKey, &'a pb::Entity>,
}

impl<'a> Model<'a> {
    fn new(snapshot: &'a [StoredEntity]) -> Self {
        let by_key = snapshot
            .iter()
            .filter_map(|stored| {
                let kind = refs::entity_kind(&stored.entity)?;
                let id = refs::entity_id(&stored.entity)?;
                Some((pb::EntityKey::new(kind, id), &stored.entity))
            })
            .collect();
        Self { by_key }
    }

    fn entity(&self, key: &pb::EntityKey) -> Option<&'a pb::Entity> {
        self.by_key.get(key).copied()
    }

    /// Field names of the pinned entity, or `None` when it is absent from
    /// the view or its schema is not inline.
    fn field_names(&self, kind: pb::EntityKind, id: Option<&pb::Id>) -> Option<BTreeSet<String>> {
        let entity = self.entity(&pb::EntityKey::new(kind, id?))?;
        let any = schema::kind_schema(entity.kind.as_ref()?);
        if any.is_some_and(|any| !schema::is_schema(any)) {
            return None;
        }
        let mut names: BTreeSet<String> = schema::schema_fields(any)
            .into_iter()
            .map(|field| field.name)
            .collect();
        if let Some(pb::entity::Kind::ReadModel(rm)) = entity.kind.as_ref() {
            names.extend(rm.key.iter().cloned());
        }
        Some(names)
    }

    /// Union of the field names of every pinned entity, failing when any is
    /// missing.
    fn union<'r>(
        &self,
        kind: pb::EntityKind,
        ids: impl IntoIterator<Item = Option<&'r pb::Id>>,
    ) -> Option<BTreeSet<String>> {
        let mut names = BTreeSet::new();
        for id in ids {
            names.extend(self.field_names(kind, id)?);
        }
        Some(names)
    }

    fn consumption(
        &self,
        root_kind: pb::EntityKind,
        root_names: &BTreeSet<String>,
        referrer: &pb::Entity,
        site: &str,
    ) -> Consumption {
        use pb::{entity::Kind as K, EntityKind as EK};
        if site == "supersedes" {
            return Consumption::Identity;
        }
        if is_typed_field_ref(site) {
            return Consumption::WholeShape;
        }
        if let Some(scenario) = scenario_example(referrer, site) {
            return example(scenario.payload);
        }
        let fields = |names: Option<BTreeSet<String>>,
                      additions: fn(BTreeSet<String>) -> Additions| {
            names.map_or(
                Consumption::Unknown(UnknownReason::ConsumerContextMissing),
                |names| {
                    let consumed = names.clone();
                    Consumption::Fields {
                        names: consumed,
                        additions: additions(names),
                    }
                },
            )
        };
        let Some(kind) = referrer.kind.as_ref() else {
            return Consumption::Unknown(UnknownReason::NoConsumptionModel);
        };
        match (root_kind, kind) {
            (
                _,
                K::EventModel(_)
                | K::Component(_)
                | K::Storyboard(_)
                | K::Tracker(_)
                | K::Term(_)
                | K::Ambiguity(_)
                | K::Swimlane(_)
                | K::Project(_)
                | K::Domain(_)
                | K::Subdomain(_)
                | K::BoundedContext(_)
                | K::Team(_)
                | K::Person(_)
                | K::Decision(_)
                | K::Document(_),
            ) => Consumption::Identity,
            (EK::Event, K::CommandSlice(slice)) if is_indexed(site, "emitted_events") => {
                let command = slice
                    .command
                    .as_ref()
                    .and_then(|edge| edge.command.as_ref())
                    .map(|command| command.id.as_ref());
                let sourced = match command {
                    Some(id) => self.field_names(EK::Command, id),
                    None => Some(BTreeSet::new()),
                };
                sourced.map_or(
                    Consumption::Unknown(UnknownReason::ConsumerContextMissing),
                    |sourced| Consumption::Fields {
                        names: root_names.clone(),
                        additions: Additions::UnlessSourced(sourced),
                    },
                )
            }
            (EK::Event, K::ReadModelSlice(slice)) if is_indexed(site, "source_events") => {
                let read_model = slice
                    .read_model
                    .as_ref()
                    .and_then(|edge| edge.read_model.as_ref());
                fields(
                    read_model.and_then(|rm| self.field_names(EK::ReadModel, rm.id.as_ref())),
                    |_| Additions::All,
                )
            }
            (EK::Event, K::ReadModel(rm)) if is_indexed(site, "source_events") => {
                let mut names: BTreeSet<String> = schema::schema_fields(rm.schema.as_ref())
                    .into_iter()
                    .map(|field| field.name)
                    .collect();
                names.extend(rm.key.iter().cloned());
                if rm
                    .schema
                    .as_ref()
                    .is_some_and(|any| !schema::is_schema(any))
                {
                    return Consumption::Unknown(UnknownReason::ConsumerContextMissing);
                }
                fields(Some(names), |_| Additions::All)
            }
            (EK::Command, K::CommandSlice(slice)) if site == "command" => fields(
                self.union(
                    EK::Event,
                    slice
                        .emitted_events
                        .iter()
                        .map(|edge| edge.event.as_ref().and_then(|e| e.id.as_ref())),
                ),
                |_| Additions::Ignored,
            ),
            (EK::Command, K::AutomationSlice(slice)) if site == "emitted_command" => self
                .union(
                    EK::ReadModel,
                    slice
                        .source_read_models
                        .iter()
                        .map(|edge| edge.read_model.as_ref().and_then(|rm| rm.id.as_ref())),
                )
                .map_or(
                    Consumption::Unknown(UnknownReason::ConsumerContextMissing),
                    |sourced| Consumption::Fields {
                        names: root_names.clone(),
                        additions: Additions::UnlessSourced(sourced),
                    },
                ),
            (EK::ReadModel, K::ReadModelSlice(slice)) if site == "read_model" => self
                .union(
                    EK::Event,
                    slice
                        .source_events
                        .iter()
                        .map(|edge| edge.event.as_ref().and_then(|e| e.id.as_ref())),
                )
                .map_or(
                    Consumption::Unknown(UnknownReason::ConsumerContextMissing),
                    |sourced| Consumption::Fields {
                        names: root_names.clone(),
                        additions: Additions::UnlessSourced(sourced),
                    },
                ),
            (EK::ReadModel, K::AutomationSlice(slice))
                if is_indexed(site, "source_read_models") =>
            {
                let command = slice
                    .emitted_command
                    .as_ref()
                    .and_then(|edge| edge.command.as_ref());
                match command {
                    Some(command) => {
                        fields(self.field_names(EK::Command, command.id.as_ref()), |_| {
                            Additions::Ignored
                        })
                    }
                    None => fields(Some(BTreeSet::new()), |_| Additions::Ignored),
                }
            }
            _ => Consumption::Unknown(UnknownReason::NoConsumptionModel),
        }
    }
}

fn is_indexed(site: &str, field: &str) -> bool {
    site.strip_prefix(field)
        .and_then(|rest| rest.strip_prefix('['))
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|index| index.parse::<usize>().is_ok())
}

fn is_typed_field_ref(site: &str) -> bool {
    (site.starts_with("fields[") || site.starts_with("schema.fields["))
        && site.strip_suffix(".ref").is_some()
}

/// A scenario example reference site and the payload it carries, if any.
struct ScenarioExample<'e> {
    payload: Option<&'e prost_types::Any>,
}

/// `None` when the site is not a scenario example.
fn scenario_example<'e>(referrer: &'e pb::Entity, site: &str) -> Option<ScenarioExample<'e>> {
    use pb::entity::Kind as K;
    let mut sites: Vec<(String, Option<&'e prost_types::Any>)> = Vec::new();
    match referrer.kind.as_ref()? {
        K::CommandSlice(slice) => {
            for (s, scenario) in slice.scenarios.iter().enumerate() {
                for (g, given) in scenario.given.iter().enumerate() {
                    sites.push((
                        format!("scenarios[{s}].given[{g}].event"),
                        given.payload.as_ref(),
                    ));
                }
                if let Some(when) = &scenario.when {
                    sites.push((
                        format!("scenarios[{s}].when.command"),
                        when.payload.as_ref(),
                    ));
                }
                if let Some(pb::command_scenario::Then::Emit(emit)) = &scenario.then {
                    for (e, event) in emit.events.iter().enumerate() {
                        sites.push((
                            format!("scenarios[{s}].then.emit.events[{e}].event"),
                            event.payload.as_ref(),
                        ));
                    }
                }
            }
        }
        K::ReadModelSlice(slice) => {
            for (s, scenario) in slice.scenarios.iter().enumerate() {
                for (w, when) in scenario.when.iter().enumerate() {
                    sites.push((
                        format!("scenarios[{s}].when[{w}].event"),
                        when.payload.as_ref(),
                    ));
                }
                if let Some(then) = &scenario.then {
                    sites.push((
                        format!("scenarios[{s}].then.read_model"),
                        then.payload.as_ref(),
                    ));
                }
            }
        }
        K::AutomationSlice(slice) => {
            for (s, scenario) in slice.scenarios.iter().enumerate() {
                for (g, given) in scenario.given.iter().enumerate() {
                    sites.push((
                        format!("scenarios[{s}].given[{g}].read_model"),
                        given.payload.as_ref(),
                    ));
                }
                if let Some(then) = &scenario.then {
                    sites.push((
                        format!("scenarios[{s}].then.command"),
                        then.payload.as_ref(),
                    ));
                }
            }
        }
        _ => return None,
    }
    sites
        .into_iter()
        .find(|(path, _)| path == site)
        .map(|(_, payload)| ScenarioExample { payload })
}

fn example(payload: Option<&prost_types::Any>) -> Consumption {
    let Some(any) = payload else {
        return Consumption::Identity;
    };
    if schema::type_name(any) != "google.protobuf.Struct" {
        return Consumption::Unknown(UnknownReason::OpaquePayload);
    }
    prost_types::Struct::decode(any.value.as_slice()).map_or(
        Consumption::Unknown(UnknownReason::OpaquePayload),
        |example| Consumption::Fields {
            names: example.fields.into_keys().collect(),
            additions: Additions::Required,
        },
    )
}

/// The verdict for one reference from a dependent to the root.
struct Verdict {
    classification: Classification,
    consumed_paths: Vec<String>,
    causal_ops: Vec<pb::DiffOp>,
    reasons: BTreeSet<UnknownReason>,
}

impl Verdict {
    fn bump_only(consumed_paths: Vec<String>) -> Self {
        Self {
            classification: Classification::BumpOnly,
            consumed_paths,
            causal_ops: Vec::new(),
            reasons: BTreeSet::new(),
        }
    }

    fn unknown(
        reason: UnknownReason,
        consumed_paths: Vec<String>,
        causal_ops: Vec<pb::DiffOp>,
    ) -> Self {
        Self {
            classification: Classification::Unknown,
            consumed_paths,
            causal_ops,
            reasons: BTreeSet::from([reason]),
        }
    }
}

fn evaluate(consumption: &Consumption, root: &RootChange<'_>) -> Verdict {
    let (consumed, additions): (BTreeSet<&str>, &Additions) = match consumption {
        Consumption::Identity => return Verdict::bump_only(Vec::new()),
        Consumption::Unknown(reason) => {
            return if root.is_cosmetic() {
                Verdict::bump_only(Vec::new())
            } else {
                Verdict::unknown(*reason, Vec::new(), root.non_cosmetic_ops())
            };
        }
        Consumption::WholeShape | Consumption::Fields { .. } => {
            let RootShape::Fields { current, .. } = &root.shape else {
                let reason = match root.shape {
                    RootShape::OpaqueSchema => UnknownReason::OpaqueRootSchema,
                    _ => UnknownReason::UnsupportedRootKind,
                };
                return Verdict::unknown(reason, Vec::new(), root.non_cosmetic_ops());
            };
            let names = current.iter().map(|field| field.name.as_str());
            match consumption {
                Consumption::Fields {
                    names: wanted,
                    additions,
                } => (
                    names.filter(|name| wanted.contains(*name)).collect(),
                    additions,
                ),
                _ => (names.collect(), &Additions::Required),
            }
        }
    };
    let RootShape::Fields { base, .. } = &root.shape else {
        unreachable!("field consumption is resolved only against a field shape");
    };
    let consumed_paths: Vec<String> = consumed
        .iter()
        .map(|name| format!("{base}[{name}]"))
        .collect();
    let mut breaking = Vec::new();
    let mut unknown = Vec::new();
    for change in &root.changes {
        match change {
            Change::Field { name, op } if consumed.contains(name.as_str()) => {
                breaking.push((*op).clone());
            }
            Change::Added { field, op } if additions.breaks(field) => breaking.push((*op).clone()),
            Change::Annotation { name, op } if consumed.contains(name.as_str()) => {
                unknown.push((*op).clone());
            }
            Change::Uncovered(op) => unknown.push((*op).clone()),
            Change::Cosmetic
            | Change::Field { .. }
            | Change::Added { .. }
            | Change::Annotation { .. } => {}
        }
    }
    if !breaking.is_empty() {
        return Verdict {
            classification: Classification::Breaking,
            consumed_paths,
            causal_ops: breaking,
            reasons: BTreeSet::new(),
        };
    }
    if !unknown.is_empty() || !root.uncovered.is_empty() {
        return Verdict::unknown(UnknownReason::UncoveredChange, consumed_paths, unknown);
    }
    Verdict::bump_only(consumed_paths)
}

/// What [`classify`] needs about one comparison.
pub struct Comparison<'a> {
    pub current: &'a pb::Entity,
    pub candidate: &'a pb::Entity,
    pub ops: &'a [pb::DiffOp],
    pub snapshot: &'a [StoredEntity],
    pub index: &'a ReverseIndex,
    pub dependents: &'a Dependents,
    pub incompleteness: BTreeSet<Incompleteness>,
}

pub struct Report {
    pub verdict: Classification,
    pub incompleteness: Vec<pb::ImpactIncompleteness>,
    pub uncovered_changes: Vec<String>,
    pub counts: pb::ClassificationCounts,
    /// Every dependent, ordered by depth, kind, then canonical id.
    pub dependents: Vec<pb::DependentImpact>,
}

struct Node {
    key: pb::EntityKey,
    depth: u32,
    occurrences: Vec<(pb::Reference, Verdict)>,
    links: Vec<(pb::Reference, pb::EntityKey)>,
    classification: Classification,
    reasons: BTreeSet<UnknownReason>,
}

#[must_use]
pub fn classify(comparison: &Comparison<'_>) -> Report {
    let candidate_fields = candidate_fields(comparison.candidate);
    let root = RootChange::new(
        comparison.current,
        comparison.candidate,
        &candidate_fields,
        comparison.ops,
    );
    let root_key = match (
        refs::entity_kind(comparison.current),
        refs::entity_id(comparison.current),
    ) {
        (Some(kind), Some(id)) => pb::EntityKey::new(kind, id),
        _ => {
            return Report {
                verdict: Classification::Unknown,
                incompleteness: Vec::new(),
                uncovered_changes: Vec::new(),
                counts: pb::ClassificationCounts::default(),
                dependents: Vec::new(),
            };
        }
    };
    let root_names: BTreeSet<String> = match &root.shape {
        RootShape::Fields { current, .. } => {
            current.iter().map(|field| field.name.clone()).collect()
        }
        RootShape::OpaqueSchema | RootShape::Unsupported => BTreeSet::new(),
    };
    let model = Model::new(comparison.snapshot);

    let mut nodes: Vec<Node> = comparison
        .dependents
        .depths
        .iter()
        .map(|(key, depth)| Node {
            key: key.clone(),
            depth: *depth,
            occurrences: Vec::new(),
            links: Vec::new(),
            classification: Classification::BumpOnly,
            reasons: BTreeSet::new(),
        })
        .collect();
    nodes.sort_by_cached_key(|node| sort_key(&node.key, node.depth));
    let position: HashMap<pb::EntityKey, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (node.key.clone(), i))
        .collect();

    for reference in comparison.index.incoming(root_key.kind, &root_key.id()) {
        let Some(&at) = referrer_key(reference).and_then(|from| position.get(&from)) else {
            continue;
        };
        let consumption = model.entity(&nodes[at].key).map_or(
            Consumption::Unknown(UnknownReason::ConsumerContextMissing),
            |referrer| {
                model.consumption(root_key.kind, &root_names, referrer, &reference.from_field)
            },
        );
        let verdict = evaluate(&consumption, &root);
        nodes[at].occurrences.push((reference.clone(), verdict));
    }
    let mut links: Vec<(usize, pb::Reference, pb::EntityKey)> = Vec::new();
    for target in &nodes {
        for reference in comparison.index.incoming(target.key.kind, &target.key.id()) {
            if let Some(&at) = referrer_key(reference).and_then(|from| position.get(&from)) {
                if nodes[at].key != target.key {
                    links.push((at, reference.clone(), target.key.clone()));
                }
            }
        }
    }
    for (at, reference, target) in links {
        nodes[at].links.push((reference, target));
    }

    for node in &mut nodes {
        for (_, verdict) in &node.occurrences {
            node.classification = node.classification.max(verdict.classification);
            node.reasons.extend(verdict.reasons.iter().copied());
        }
    }
    loop {
        let mut changed = false;
        for at in 0..nodes.len() {
            let inherited = nodes[at]
                .links
                .iter()
                .map(|(_, target)| nodes[position[target]].classification.through())
                .max()
                .unwrap_or(Classification::BumpOnly);
            if inherited > nodes[at].classification {
                nodes[at].classification = inherited;
                nodes[at]
                    .reasons
                    .insert(UnknownReason::TransitiveThroughAffected);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for at in 0..nodes.len() {
        let affected = nodes[at]
            .links
            .iter()
            .any(|(_, target)| nodes[position[target]].classification != Classification::BumpOnly);
        if affected {
            nodes[at]
                .reasons
                .insert(UnknownReason::TransitiveThroughAffected);
        }
    }
    let incomplete = !comparison.incompleteness.is_empty();
    if incomplete {
        for node in &mut nodes {
            if node.classification == Classification::BumpOnly {
                node.classification = Classification::Unknown;
                node.reasons.insert(UnknownReason::TraversalIncomplete);
            }
        }
    }

    let mut counts = pb::ClassificationCounts::default();
    let mut verdict = if incomplete {
        Classification::Unknown
    } else {
        Classification::BumpOnly
    };
    for node in &nodes {
        verdict = verdict.max(node.classification);
        match node.classification {
            Classification::Breaking => counts.breaking += 1,
            Classification::BumpOnly => counts.bump_only += 1,
            Classification::Unknown => counts.unknown += 1,
        }
    }
    let final_classification: Vec<Classification> =
        nodes.iter().map(|node| node.classification).collect();
    let dependents = nodes
        .into_iter()
        .map(|node| pb::DependentImpact {
            dependent: Some(pb::EntityRef {
                kind: node.key.kind as i32,
                id: Some(node.key.id()),
            }),
            depth: node.depth,
            classification: node.classification.to_proto() as i32,
            occurrences: node
                .occurrences
                .into_iter()
                .map(|(reference, verdict)| pb::ConsumedOccurrence {
                    reference: Some(reference),
                    classification: verdict.classification.to_proto() as i32,
                    consumed_paths: verdict.consumed_paths,
                    causal_ops: verdict.causal_ops,
                    unknown_reasons: verdict
                        .reasons
                        .into_iter()
                        .map(|reason| reason.to_proto() as i32)
                        .collect(),
                })
                .collect(),
            through: node
                .links
                .into_iter()
                .map(|(reference, target)| pb::TransitiveLink {
                    reference: Some(reference),
                    through_classification: final_classification[position[&target]].to_proto()
                        as i32,
                })
                .collect(),
            unknown_reasons: if node.classification == Classification::Unknown {
                node.reasons
                    .into_iter()
                    .map(|reason| reason.to_proto() as i32)
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect();
    Report {
        verdict,
        incompleteness: comparison
            .incompleteness
            .iter()
            .map(|reason| reason.to_proto())
            .collect(),
        uncovered_changes: root.uncovered.clone(),
        counts,
        dependents,
    }
}

fn sort_key(key: &pb::EntityKey, depth: u32) -> (u32, i32, String, String, u64) {
    (
        depth,
        key.kind as i32,
        key.namespace.clone(),
        key.slug.clone(),
        key.version,
    )
}

impl Report {
    #[must_use]
    pub fn verdict_proto(&self) -> pb::DependentClassification {
        self.verdict.to_proto()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_referrer_without_a_known_kind_or_id_is_unresolved() {
        let id = pb::Id {
            namespace: "shop".into(),
            slug: "placed".into(),
            version: 1,
        };
        let reference = |kind: i32, id: Option<pb::Id>| pb::Reference {
            from: Some(pb::EntityRef { kind, id }),
            ..Default::default()
        };
        assert_eq!(referrer_key(&pb::Reference::default()), None);
        assert_eq!(referrer_key(&reference(i32::MAX, Some(id.clone()))), None);
        assert_eq!(
            referrer_key(&reference(pb::EntityKind::Event as i32, None)),
            None
        );
        assert_eq!(
            referrer_key(&reference(pb::EntityKind::Event as i32, Some(id.clone()))),
            Some(pb::EntityKey::new(pb::EntityKind::Event, &id))
        );
    }
}
