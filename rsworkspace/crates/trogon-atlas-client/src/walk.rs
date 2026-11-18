//! `walk_scenarios`: play example data through an event model.
//!
//! Validation checks a model's structure; it never runs the model. A model
//! can pass validation and still fail to carry the data a real timeline
//! needs: a command field no observed read model supplies, a read model
//! that ends in the wrong state, a command accepted where a reject scenario
//! should refuse it, a step legal on its own but not after the steps
//! before it.
//!
//! A *case* (a `Specification` document) is data, not code: initial values
//! (`given` events and read-model rows), ordered `steps` (a command, the
//! external `answer` it receives, and the expected emit or reject), and
//! expected read-model rows. The walker plays each step against the model
//! and checks, per step:
//!
//! 1. Lifecycle: every emitted event is a legal successor of the stream's
//!    previous lifecycle event (`Swimlane.transitions`).
//! 2. Command data: every command field gets a value from the read models
//!    the issuing UI or automation observes, or from the step's answer.
//!    Disagreeing sources are flagged.
//! 3. Event data: every event field gets a value from the command, an
//!    earlier event on the same stream, the swimlane's `state`, or a
//!    `DerivedFieldAnnotation`.
//! 4. Projection: each read-model slice projects the event, and the row
//!    carries every field its slice scenario's `then` payload names; the
//!    case's expected rows must match.
//! 5. Rejects: a command after a terminal lifecycle event must be refused
//!    by a reject scenario; an expected reject must be modeled.
//! 6. Coverage: storyboards and scenarios no case walked.
//!
//! Findings use the validation finding shape (severity, code, message,
//! subject, subject field, rule context) plus the case and step that
//! produced them. The walker reads each entity's proto JSON, so it runs
//! offline against manifest files or against a namespace fetched from the
//! live store.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    path::PathBuf,
};

use anyhow::{anyhow, Context as _, Result};
use pb::EntityKind as K;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    export, manifest,
    tenant_types::{self, TypeScope},
};

/// Manifest `kind` of a case document.
pub const SPECIFICATION_KIND: &str = "Specification";

/// `ValidationIssue.rule_category` every walk finding carries.
pub const RULE_CATEGORY: &str = "walk";

const STRUCT_TYPE_SUFFIX: &str = "google.protobuf.Struct";
const DERIVED_TYPE_SUFFIX: &str = "trogonatlas.eventmodel.v1alpha1.DerivedFieldAnnotation";

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// Severity, rendered the way `ValidationIssue.Severity` renders in proto
/// JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Severity {
    #[serde(rename = "SEVERITY_INFO")]
    Info,
    #[serde(rename = "SEVERITY_WARNING")]
    Warning,
    #[serde(rename = "SEVERITY_ERROR")]
    Error,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        }
    }
}

/// One entry of the walk rule catalog, shaped like `ValidationRule`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkRule {
    pub code: &'static str,
    pub default_severity: Severity,
    pub title: &'static str,
    pub doc: &'static str,
}

macro_rules! rules {
    ($($ident:ident = $code:literal, $sev:ident, $title:literal, $doc:literal;)*) => {
        $(const $ident: &WalkRule = &WalkRule { code: $code, default_severity: Severity::$sev, title: $title, doc: $doc };)*
        /// Every code the walker can emit.
        pub const RULES: &[WalkRule] = &[
            $(*$ident,)*
        ];
    };
}

rules! {
    REF_UNRESOLVED = "WALK_REF_UNRESOLVED", Error, "Case names an entity the model lacks",
        "A step's command, a `via` slice, or an expected read model does not resolve in the walked model. Refs are `slug`, `namespace/slug` or `namespace/slug@version`; a bare slug resolves in the case's namespace and an unpinned ref resolves to the latest version.";
    CASE_INVALID = "WALK_CASE_INVALID", Error, "Case is malformed",
        "A step expects both an emit and a reject; state one of them, or neither to let the model decide.";
    VIA_NOT_ISSUER = "WALK_VIA_NOT_ISSUER", Error, "`via` does not issue the command",
        "A step's `via` names an AutomationSlice or UiSlice that does not issue the step's command (an automation must emit it; a UI slice must show the command slice's `ui`).";
    SCHEMA_OPAQUE = "WALK_SCHEMA_OPAQUE", Info, "Schema not inspected",
        "The entity's schema is a type-library message the walker cannot see into, so its fields are not checked; observed and answered values pass through unchecked.";
    COMMAND_UNHANDLED = "WALK_COMMAND_UNHANDLED", Error, "No command slice handles the command",
        "A step issues a command that no CommandSlice names, so the model says nothing about what the command does.";
    COMMAND_FIELD_UNSUPPLIED = "WALK_COMMAND_FIELD_UNSUPPLIED", Error, "Command field gets no value",
        "At this step no read model observed by the UI or automation that issues the command holds a value for the field, and the step's external answer does not supply it. Project the field into an observed read model, observe a read model that carries it, mark it with a DerivedFieldAnnotation, or document it as an external answer.";
    COMMAND_FIELD_CONFLICT = "WALK_COMMAND_FIELD_CONFLICT", Error, "Command field sources disagree",
        "Two sources (observed read-model rows or the external answer) offer different values for one command field. The model does not say which one wins.";
    COMMAND_FIELD_AMBIGUOUS = "WALK_COMMAND_FIELD_AMBIGUOUS", Warning, "Observed rows disagree on a command field",
        "Several rows of an observed read model match the step and hold different values for the field. Put the read model's key fields in the step's answer to pick the row. Reported as an error when the field is required and nothing else supplies it.";
    EVENT_FIELD_UNSOURCED = "WALK_EVENT_FIELD_UNSOURCED", Error, "Event field gets no value",
        "At this step an emitted event field gets no value from the command, from an earlier event on the same stream, from the swimlane's `state`, or from a DerivedFieldAnnotation.";
    EVENT_NOT_EMITTED_BY_SLICE = "WALK_EVENT_NOT_EMITTED_BY_SLICE", Error, "Expected event is not one the slice emits",
        "The case expects an event the command's CommandSlice does not list in `emitted_events`.";
    LIFECYCLE_VIOLATION = "WALK_LIFECYCLE_VIOLATION", Error, "Event is not a legal successor",
        "The emitted event is not a `next` of the stream's previous lifecycle event in `Swimlane.transitions`. The step may be legal on its own but not after the steps before it.";
    ACCEPTED_AFTER_TERMINAL = "WALK_ACCEPTED_AFTER_TERMINAL", Error, "Command accepted after a terminal event",
        "The stream's previous lifecycle event has no successor, yet the command emitted an event because no reject scenario refuses it. Add a reject scenario for this history.";
    LIFECYCLE_START = "WALK_LIFECYCLE_START", Warning, "Stream starts mid-lifecycle",
        "The first lifecycle event on the stream is only ever reached as a successor of another event. Give the earlier event in the case's `given`, or model the event as a lifecycle start.";
    REJECT_UNMODELED = "WALK_REJECT_UNMODELED", Error, "Expected reject is not modeled",
        "The case expects the command to be refused, but no reject scenario on the command slice matches the stream's history, so the model accepts the command.";
    REJECT_REASON_MISMATCH = "WALK_REJECT_REASON_MISMATCH", Warning, "Reject reason differs",
        "A reject scenario refuses the command, but with a different reason code than the case expects.";
    UNEXPECTED_REJECT = "WALK_UNEXPECTED_REJECT", Error, "Model refuses an expected command",
        "The case expects events, but a reject scenario on the command slice matches the stream's history and refuses the command.";
    EMIT_MISMATCH = "WALK_EMIT_MISMATCH", Error, "Model emits something else",
        "A scenario on the command slice matches the stream's history and decides the step's events, but they differ from the events the case expects. The walk follows the model.";
    SCENARIO_MISSING = "WALK_SCENARIO_MISSING", Warning, "No scenario covers the step",
        "No scenario on the command slice has this command with a `given` matching the stream's history and the step's outcome. The walk continued, but the model does not document this timeline.";
    OUTCOME_UNDETERMINED = "WALK_OUTCOME_UNDETERMINED", Error, "Step outcome cannot be decided",
        "The step states no expected outcome, no scenario matches, and the command slice emits more than one event. State `expect.emit` or `expect.reject` on the step.";
    STREAM_UNRESOLVED = "WALK_STREAM_UNRESOLVED", Warning, "Stream id has an unfilled placeholder",
        "The swimlane's `stream_id` names a placeholder the payload does not carry, so the walk cannot tell stream instances apart.";
    PROJECTION_KEY_UNSOURCED = "WALK_PROJECTION_KEY_UNSOURCED", Error, "Projected event lacks a row key",
        "A read-model slice projects an event whose payload does not carry every `key` field of the target read model, so the projection cannot address a row.";
    PROJECTION_FIELD_MISSING = "WALK_PROJECTION_FIELD_MISSING", Error, "Projected row lacks a scenario field",
        "A read-model slice scenario says the row holds this field after these events, but the walked projection left it unset.";
    READ_MODEL_MISMATCH = "WALK_READ_MODEL_MISMATCH", Error, "Read model differs from the case",
        "After the step (or at the end of the case) the read model's rows differ from the rows the case expects.";
    STORYBOARD_UNWALKED = "WALK_STORYBOARD_UNWALKED", Info, "Storyboard not walked",
        "No case exercised any slice of this storyboard.";
    SCENARIO_UNWALKED = "WALK_SCENARIO_UNWALKED", Info, "Scenario not walked",
        "No case step matched this slice scenario.";
}

// ---------------------------------------------------------------------------
// Findings and report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Key {
    pub namespace: String,
    pub slug: String,
    /// Proto JSON renders uint64 as a decimal string; so does this, so a
    /// version above 2^53 survives a JSON consumer's float parse.
    #[serde(serialize_with = "version_as_string")]
    pub version: u64,
}

// serde's `serialize_with` hands the field by reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn version_as_string<S: serde::Serializer>(
    version: &u64,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&version.to_string())
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.version == 0 {
            // An unpinned event outside the walked model.
            return write!(f, "{}/{}", self.namespace, self.slug);
        }
        write!(f, "{}/{}@{}", self.namespace, self.slug, self.version)
    }
}

/// `ValidationIssue.subject`: an `EntityRef` in proto JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Subject {
    pub kind: String,
    pub id: Key,
}

/// One finding, in the `ValidationIssue` shape plus the case location.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkFinding {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Subject>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subject_field: String,
    pub rule_title: &'static str,
    pub rule_category: &'static str,
    pub rule_default_severity: Severity,
    /// `namespace/name` of the case, empty for coverage findings.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub case: String,
    /// 1-based step index; absent for `given` and end-of-case findings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
}

impl WalkFinding {
    fn new(r: &'static WalkRule, message: impl Into<String>) -> Self {
        Self {
            severity: r.default_severity,
            code: r.code,
            message: message.into(),
            subject: None,
            subject_field: String::new(),
            rule_title: r.title,
            rule_category: RULE_CATEGORY,
            rule_default_severity: r.default_severity,
            case: String::new(),
            step: None,
        }
    }

    fn subject(mut self, kind: K, key: &Key) -> Self {
        self.subject = Some(Subject {
            kind: kind.as_str_name().to_string(),
            id: key.clone(),
        });
        self
    }

    fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    fn schema_field(self, kind: K, key: &Key, name: &str) -> Self {
        self.subject(kind, key)
            .field(format!("schema.fields[{name}]"))
    }

    fn field(mut self, field: impl Into<String>) -> Self {
        self.subject_field = field.into();
        self
    }

    /// `ERROR WALK_X [case step 2] message (subject)`.
    pub fn render(&self) -> String {
        let mut out = format!("{:<7} {}", self.severity.label(), self.code);
        if !self.case.is_empty() {
            out.push_str(" [");
            out.push_str(&self.case);
            if let Some(step) = self.step {
                out.push_str(" step ");
                out.push_str(&step.to_string());
            }
            out.push(']');
        }
        out.push(' ');
        out.push_str(&self.message);
        out
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseSummary {
    pub case: String,
    pub source: String,
    pub steps: usize,
    pub errors: usize,
    pub warnings: usize,
    pub passed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub storyboards_walked: usize,
    pub storyboards_total: usize,
    pub scenarios_walked: usize,
    pub scenarios_total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkReport {
    pub cases: Vec<CaseSummary>,
    pub findings: Vec<WalkFinding>,
    pub coverage: Coverage,
}

impl WalkReport {
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Entity bodies (proto JSON) keyed by kind, then exact id. Refs resolve to
/// the exact version they pin; iteration over "live" entities sees only the
/// latest version of each slug, so a superseded slice never projects.
#[derive(Debug, Default, Clone)]
pub struct Model {
    docs: BTreeMap<K, BTreeMap<Key, Value>>,
    /// Versions the latest entities pin, by (namespace, slug).
    pins: BTreeMap<(String, String), BTreeSet<u64>>,
}

impl Model {
    /// Build from stored or parsed entities. Entities that cannot be
    /// rendered are returned by label, never dropped silently.
    pub fn from_entities(entities: &[pb::Entity], types: &TranscodePool) -> (Self, Vec<String>) {
        let mut model = Model::default();
        let mut skipped = Vec::new();
        for entity in entities {
            match entity_body(entity, types) {
                Ok((kind, key, body)) => {
                    model.docs.entry(kind).or_default().insert(key, body);
                }
                Err(err) => skipped.push(format!("{}: {err:#}", export::entity_label(entity))),
            }
        }
        let mut pins = BTreeMap::new();
        for kind in model.docs.keys() {
            for (_, body) in model.live(*kind) {
                collect_refs(body, &mut pins);
            }
        }
        model.pins = pins;
        (model, skipped)
    }

    /// Load manifest files or directories offline.
    pub fn load_paths(paths: &[PathBuf]) -> Result<Self> {
        let loaded = manifest::load_paths(paths)?;
        let entities: Vec<pb::Entity> = loaded.into_iter().map(|m| m.entity).collect();
        let types = TranscodePool::builtin()?;
        let (model, skipped) = Self::from_entities(&entities, &types);
        if let Some(first) = skipped.first() {
            crate::invalid_input!("cannot render manifest {first}");
        }
        Ok(model)
    }

    /// Fetch every entity version in `namespaces` from the live store; an
    /// empty list means the namespaces `cases` name.
    pub async fn fetch(
        client: &mut Client,
        namespaces: &[String],
        cases: &[Case],
    ) -> Result<(Self, Vec<String>)> {
        let namespaces = fetch_scope(namespaces, cases);
        let mut scope = TypeScope::default();
        let mut entities = Vec::new();
        for namespace in namespaces {
            trogon_atlas_core::validate_id_component(namespace).map_err(|e| {
                crate::failure::invalid_input(format!("namespace {namespace:?}: {e}"))
            })?;
            scope.add_namespace(namespace);
            entities.extend(export::fetch_namespace_entities(client, namespace).await?);
        }
        let types = tenant_types::transcode_pool(client, &scope).await?;
        Ok(Self::from_entities(&entities, &types))
    }

    fn get(&self, kind: K, key: &Key) -> Option<&Value> {
        self.docs.get(&kind)?.get(key)
    }

    fn latest_version(&self, kind: K, namespace: &str, slug: &str) -> Option<u64> {
        let bound = |version| Key {
            namespace: namespace.to_string(),
            slug: slug.to_string(),
            version,
        };
        self.docs
            .get(&kind)?
            .range(bound(0)..=bound(u64::MAX))
            .next_back()
            .map(|(key, _)| key.version)
    }

    /// For a bare case ref: the highest version of the entity that the
    /// model's latest entities pin, so a case follows the version slices use
    /// rather than a newer one nothing references yet.
    fn pinned_version(&self, kind: K, namespace: &str, slug: &str) -> Option<u64> {
        self.pins
            .get(&(namespace.to_string(), slug.to_string()))?
            .iter()
            .rev()
            .copied()
            .find(|v| {
                self.get(
                    kind,
                    &Key {
                        namespace: namespace.to_string(),
                        slug: slug.to_string(),
                        version: *v,
                    },
                )
                .is_some()
            })
    }

    /// Latest version of every entity of `kind`.
    fn live(&self, kind: K) -> impl Iterator<Item = (&Key, &Value)> {
        // Keys sort by (namespace, slug, version): the last of each
        // (namespace, slug) run is its latest version.
        let mut entries = self.docs.get(&kind).into_iter().flatten().peekable();
        std::iter::from_fn(move || loop {
            let entry = entries.next()?;
            let superseded = entries.peek().is_some_and(|(next, _)| {
                next.namespace == entry.0.namespace && next.slug == entry.0.slug
            });
            if !superseded {
                return Some(entry);
            }
        })
    }

    /// Resolve a case ref (`slug`, `ns/slug`, `ns/slug@v`) against `kinds`.
    fn resolve(&self, kinds: &[K], reference: &str, default_ns: &str) -> Option<(K, Key)> {
        let (namespace, slug, version) = split_ref(reference, default_ns);
        for kind in kinds {
            let version = match version {
                Some(v) => v,
                None => match self
                    .pinned_version(*kind, &namespace, &slug)
                    .or_else(|| self.latest_version(*kind, &namespace, &slug))
                {
                    Some(v) => v,
                    None => continue,
                },
            };
            let key = Key {
                namespace: namespace.clone(),
                slug: slug.clone(),
                version,
            };
            if self.get(*kind, &key).is_some() {
                return Some((*kind, key));
            }
        }
        None
    }

    /// Like `resolve`, but an event the model lacks (one owned by a
    /// namespace that is not loaded) still yields a key, so `given` can
    /// carry upstream events that read-model slices project.
    fn event_key(&self, reference: &str, default_ns: &str) -> Key {
        if let Some((_, key)) = self.resolve(&[K::Event], reference, default_ns) {
            return key;
        }
        let (namespace, slug, version) = split_ref(reference, default_ns);
        // Version 0 marks an unpinned event the model does not hold: it
        // matches whichever version a slice pins (see `same_event`).
        Key {
            namespace,
            slug,
            version: version.unwrap_or(0),
        }
    }
}

/// The namespaces a live walk loads: the cases' own plus `extra`, such as an
/// upstream namespace whose events feed local projections.
fn fetch_scope<'a>(extra: &'a [String], cases: &'a [Case]) -> BTreeSet<&'a str> {
    cases
        .iter()
        .map(|c| c.namespace.as_str())
        .chain(extra.iter().map(String::as_str))
        .collect()
}

/// Every `{namespace, slug, version}` ref in an entity body, below its top
/// level (the top-level `id` is the entity itself).
fn collect_refs(body: &Value, out: &mut BTreeMap<(String, String), BTreeSet<u64>>) {
    fn walk(v: &Value, out: &mut BTreeMap<(String, String), BTreeSet<u64>>) {
        match v {
            Value::Object(map) => {
                if let Some(key) = id_key(v).filter(|_| map.contains_key("version")) {
                    out.entry((key.namespace, key.slug))
                        .or_default()
                        .insert(key.version);
                }
                map.values().for_each(|c| walk(c, out));
            }
            Value::Array(items) => items.iter().for_each(|c| walk(c, out)),
            _ => {}
        }
    }
    if let Value::Object(map) = body {
        map.iter()
            .filter(|(k, _)| k.as_str() != "id")
            .for_each(|(_, v)| walk(v, out));
    }
}

/// An entity's kind, exact id and proto-JSON body (references unshortened).
fn entity_body(entity: &pb::Entity, types: &TranscodePool) -> Result<(K, Key, Value)> {
    let kind = trogon_atlas_core::refs::entity_kind(entity)
        .ok_or_else(|| anyhow!("entity has no kind"))?;
    let Value::Object(envelope) = types.entity_json_value(entity)? else {
        return Err(anyhow!("entity rendered as non-object JSON"));
    };
    let body = envelope
        .into_iter()
        .find_map(|(field, body)| (field != "system").then_some(body))
        .ok_or_else(|| anyhow!("entity JSON has no body"))?;
    let key = body
        .get("id")
        .and_then(id_key)
        .ok_or_else(|| anyhow!("entity body has no id"))?;
    Ok((kind, key, body))
}

/// Split a case ref (`slug`, `ns/slug`, `ns/slug@v`) into its parts
/// without validating them; `check_ref` validates, `split_ref` resolves.
fn ref_parts(reference: &str) -> (Option<&str>, &str, Option<&str>) {
    let (path, version) = match reference.rsplit_once('@') {
        Some((path, v)) => (path, Some(v)),
        None => (reference, None),
    };
    match path.split_once('/') {
        Some((ns, slug)) => (Some(ns), slug, version),
        None => (None, path, version),
    }
}

fn split_ref(reference: &str, default_ns: &str) -> (String, String, Option<u64>) {
    let (namespace, slug, version) = ref_parts(reference);
    (
        namespace.unwrap_or(default_ns).to_string(),
        slug.to_string(),
        version.and_then(|v| v.parse::<u64>().ok()),
    )
}

/// Whether `event` takes part in a swimlane lifecycle, as a source or a
/// successor of some transition.
fn in_lifecycle(transitions: &[(Key, Vec<Key>)], event: &Key) -> bool {
    transitions
        .iter()
        .any(|(from, next)| from == event || next.contains(event))
}

// --- JSON accessors over manifest specs ---

fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn id_key(id: &Value) -> Option<Key> {
    Some(Key {
        namespace: id.get("namespace")?.as_str()?.to_string(),
        slug: id.get("slug")?.as_str()?.to_string(),
        version: id.get("version").and_then(as_u64).unwrap_or(1),
    })
}

/// `{inner: {id: {...}}}` → key (an Edge or an Example's typed ref).
fn edge_key(edge: &Value, inner: &str) -> Option<Key> {
    id_key(edge.get(inner)?.get("id")?)
}

/// `spec[field]` as an edge → key.
fn edge_at(spec: &Value, field: &str, inner: &str) -> Option<Key> {
    spec.get(field).and_then(|e| edge_key(e, inner))
}

/// `spec[field]` as a list of edges → keys.
fn edge_keys(spec: &Value, field: &str, inner: &str) -> Vec<Key> {
    arr(spec, field)
        .iter()
        .filter_map(|e| edge_key(e, inner))
        .collect()
}

/// `{id: {...}}` → key (a bare typed ref).
fn ref_key(r: &Value) -> Option<Key> {
    id_key(r.get("id")?)
}

/// The example payload of an `*Example`: a `google.protobuf.Struct` packed
/// in an `Any`, whose keys mirror the type's FieldSpec names.
fn payload_of(example: &Value) -> Option<Map<String, Value>> {
    let payload = example.get("payload")?.as_object()?;
    let is_struct = payload
        .get("@type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.ends_with(STRUCT_TYPE_SUFFIX));
    if !is_struct {
        return None;
    }
    if let Some(Value::Object(map)) = payload.get("value") {
        return Some(map.clone());
    }
    let mut map = payload.clone();
    map.remove("@type");
    Some(map)
}

#[derive(Debug, Clone)]
struct Field {
    name: String,
    optional: bool,
    derived: Option<String>,
}

fn field_specs(list: &[Value]) -> Vec<Field> {
    list.iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?.to_string();
            let derived = arr(f, "metadata").iter().find_map(|m| {
                let t = m.get("@type")?.as_str()?;
                if !t.ends_with(DERIVED_TYPE_SUFFIX) || str_at(m, "doc").trim().is_empty() {
                    return None;
                }
                Some(
                    m.get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("KIND_OTHER")
                        .trim_start_matches("KIND_")
                        .to_ascii_lowercase(),
                )
            });
            Some(Field {
                name,
                optional: f.get("optional").and_then(Value::as_bool).unwrap_or(false),
                derived,
            })
        })
        .collect()
}

/// FieldSpecs of an inline `Schema`; `None` when the schema is a type
/// library message the walker cannot see into.
fn schema_fields(spec: &Value) -> Option<Vec<Field>> {
    let schema = spec.get("schema")?;
    let is_schema = schema
        .get("@type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.ends_with(trogon_atlas_core::schema::SCHEMA_MESSAGE));
    is_schema.then(|| field_specs(arr(schema, "fields")))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaseDocument {
    api_version: String,
    kind: String,
    metadata: CaseMetadata,
    spec: CaseSpec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseMetadata {
    namespace: String,
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseSpec {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub doc: String,
    #[serde(default)]
    pub given: Given,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub then: Expectations,
}

/// Initial values: events already on their streams, and rows of read
/// models fed from outside the model (external sources).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Given {
    #[serde(default)]
    pub events: Vec<EventValue>,
    #[serde(default)]
    pub read_models: Vec<RowsValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventValue {
    pub event: String,
    #[serde(default)]
    pub payload: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowsValue {
    pub read_model: String,
    #[serde(default)]
    pub rows: Vec<Map<String, Value>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub command: String,
    /// The AutomationSlice or UiSlice that issues the command; omit to let
    /// every issuer the model declares contribute its read models.
    #[serde(default)]
    pub via: Option<String>,
    /// The external answer the command receives: values from outside the
    /// model (a person's input, a gateway's reply).
    #[serde(default)]
    pub answer: Map<String, Value>,
    #[serde(default)]
    pub expect: StepExpect,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepExpect {
    /// Events the step emits, in order.
    #[serde(default)]
    pub emit: Option<Vec<String>>,
    /// The step is refused; the value is the expected reason code, or
    /// empty for any reason.
    #[serde(default)]
    pub reject: Option<String>,
    #[serde(default)]
    pub read_models: Vec<RowsValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectations {
    #[serde(default)]
    pub read_models: Vec<RowsValue>,
}

/// One parsed `Specification` document.
#[derive(Debug, Clone)]
pub struct Case {
    pub source: String,
    pub namespace: String,
    pub name: String,
    pub spec: CaseSpec,
}

impl Case {
    fn label(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

/// Parse every `Specification` document in a multi-document YAML stream.
pub fn parse_cases(text: &str, source: &str) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for (i, document) in serde_norway::Deserializer::from_str(text).enumerate() {
        let at = format!("{source}#doc{i}");
        let value = serde_norway::Value::deserialize(document)
            .map_err(|e| crate::failure::invalid_input(format!("{at}: parsing YAML: {e}")))?;
        if value.is_null() {
            continue;
        }
        let doc: CaseDocument = serde_norway::from_value(value)
            .map_err(|e| crate::failure::invalid_input(format!("{at}: {e}")))?;
        if doc.api_version != manifest::API_VERSION {
            crate::invalid_input!(
                "{at}: apiVersion {:?} is not {:?}",
                doc.api_version,
                manifest::API_VERSION
            );
        }
        if doc.kind != SPECIFICATION_KIND {
            crate::invalid_input!("{at}: kind {:?} is not {SPECIFICATION_KIND:?}", doc.kind);
        }
        trogon_atlas_core::validate_id_component(&doc.metadata.namespace)
            .map_err(|e| crate::failure::invalid_input(format!("{at}: metadata.namespace: {e}")))?;
        for (field, reference) in case_refs(&doc.spec) {
            check_ref(reference)
                .map_err(|e| crate::failure::invalid_input(format!("{at}: {field}: {e}")))?;
        }
        cases.push(Case {
            source: at,
            namespace: doc.metadata.namespace,
            name: doc.metadata.name,
            spec: doc.spec,
        });
    }
    Ok(cases)
}

/// Every entity ref a case names, with the field it came from.
fn case_refs(spec: &CaseSpec) -> Vec<(String, &str)> {
    let mut refs = Vec::new();
    for (i, e) in spec.given.events.iter().enumerate() {
        refs.push((format!("given.events[{i}].event"), e.event.as_str()));
    }
    push_rows(&mut refs, "given.readModels", &spec.given.read_models);
    for (i, step) in spec.steps.iter().enumerate() {
        refs.push((format!("steps[{i}].command"), step.command.as_str()));
        if let Some(via) = &step.via {
            refs.push((format!("steps[{i}].via"), via.as_str()));
        }
        for (j, e) in step.expect.emit.iter().flatten().enumerate() {
            refs.push((format!("steps[{i}].expect.emit[{j}]"), e.as_str()));
        }
        push_rows(
            &mut refs,
            &format!("steps[{i}].expect.readModels"),
            &step.expect.read_models,
        );
    }
    push_rows(&mut refs, "then.readModels", &spec.then.read_models);
    refs
}

fn push_rows<'a>(refs: &mut Vec<(String, &'a str)>, prefix: &str, list: &'a [RowsValue]) {
    for (i, r) in list.iter().enumerate() {
        refs.push((format!("{prefix}[{i}].readModel"), r.read_model.as_str()));
    }
}

/// A ref is `slug`, `ns/slug` or `ns/slug@version`: each part a valid id
/// component and the version a positive integer.
fn check_ref(reference: &str) -> std::result::Result<(), String> {
    let (namespace, slug, version) = ref_parts(reference);
    if let Some(v) = version {
        match v.parse::<u64>() {
            Ok(n) if n >= 1 => {}
            _ => return Err(format!("{reference:?}: version must be a positive integer")),
        }
    }
    for part in namespace.into_iter().chain([slug]) {
        trogon_atlas_core::validate_id_component(part)
            .map_err(|e| format!("{reference:?}: {e}"))?;
    }
    Ok(())
}

/// Read cases from files or directories (walked for `.yaml`/`.yml`).
pub fn read_case_paths(paths: &[PathBuf]) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for file in manifest::collect_yaml_files(paths)? {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        cases.extend(parse_cases(&text, &file.display().to_string())?);
    }
    // A path with no case would otherwise pass as a clean walk in CI.
    if cases.is_empty() {
        crate::invalid_input!("no Specification document found in the case paths");
    }
    Ok(cases)
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Recorded {
    event: Key,
    lane: Option<Key>,
    payload: Map<String, Value>,
    /// `None` for an event without a swimlane: it is on no stream.
    stream: Option<String>,
}

impl Recorded {
    /// Whether the event is on `stream`; with no stream, nothing is.
    fn on(&self, stream: Option<&str>) -> bool {
        stream.is_some() && self.stream.as_deref() == stream
    }
}

type Rows = BTreeMap<String, Map<String, Value>>;

#[derive(Debug, Default)]
struct Touched {
    /// Every slice a case exercised. Storyboards reference slices without
    /// a kind, so the kind is not kept.
    slices: BTreeSet<Key>,
    /// (slice key, scenario id) of every scenario a step matched.
    scenarios: BTreeSet<(K, Key, String)>,
}

/// Walk every case against `model`.
pub fn walk(model: &Model, cases: &[Case]) -> WalkReport {
    let mut touched = Touched::default();
    let mut findings = Vec::new();
    let mut summaries = Vec::new();
    for case in cases {
        let mut run = CaseRun {
            model,
            case,
            log: Vec::new(),
            rows: BTreeMap::new(),
            findings: Vec::new(),
            touched: &mut touched,
            step: None,
        };
        run.run();
        let mut case_findings = run.findings;
        dedup(&mut case_findings);
        let errors = count(&case_findings, Severity::Error);
        let warnings = count(&case_findings, Severity::Warning);
        summaries.push(CaseSummary {
            case: case.label(),
            source: case.source.clone(),
            steps: case.spec.steps.len(),
            errors,
            warnings,
            passed: errors == 0,
        });
        findings.extend(case_findings);
    }
    let namespaces: BTreeSet<&str> = cases.iter().map(|c| c.namespace.as_str()).collect();
    let coverage = coverage(model, &namespaces, &touched, &mut findings);
    WalkReport {
        cases: summaries,
        findings,
        coverage,
    }
}

fn count(findings: &[WalkFinding], severity: Severity) -> usize {
    findings.iter().filter(|f| f.severity == severity).count()
}

fn dedup(findings: &mut Vec<WalkFinding>) {
    let mut seen = HashSet::new();
    findings.retain(|f| seen.insert(f.clone()));
}

struct CaseRun<'a> {
    model: &'a Model,
    case: &'a Case,
    log: Vec<Recorded>,
    rows: BTreeMap<Key, Rows>,
    findings: Vec<WalkFinding>,
    touched: &'a mut Touched,
    step: Option<usize>,
}

/// Which source a command field's value came from.
struct Offer {
    source: String,
    value: Value,
}

impl CaseRun<'_> {
    fn ns(&self) -> &str {
        &self.case.namespace
    }

    fn push(&mut self, mut finding: WalkFinding) {
        finding.case = self.case.label();
        finding.step = self.step;
        self.findings.push(finding);
    }

    fn run(&mut self) {
        let case = self.case;
        for rows in &case.spec.given.read_models {
            let Some(rm) = self.read_model(&rows.read_model, "given.readModels") else {
                continue;
            };
            let key_fields = rm_key_fields(self.model.get(K::ReadModel, &rm));
            let table = self.rows.entry(rm).or_default();
            for row in &rows.rows {
                // Numbered across the table so seeds from several entries all stay.
                let id =
                    row_id(&key_fields, row).unwrap_or_else(|| format!("given#{}", table.len()));
                table.insert(id, row.clone());
            }
        }
        for event in &case.spec.given.events {
            let key = self.model.event_key(&event.event, self.ns());
            let stream = self.event_stream(&key, &event.payload);
            self.record(&key, &event.payload, stream);
        }
        for (i, step) in case.spec.steps.iter().enumerate() {
            self.step = Some(i + 1);
            self.run_step(step);
            for expected in &step.expect.read_models {
                self.check_rows(expected, &format!("steps[{i}].expect.readModels"));
            }
        }
        self.step = None;
        for expected in &case.spec.then.read_models {
            self.check_rows(expected, "then.readModels");
        }
    }

    fn read_model(&mut self, reference: &str, field: &str) -> Option<Key> {
        if let Some((_, key)) = self.model.resolve(&[K::ReadModel], reference, self.ns()) {
            return Some(key);
        }
        self.push(
            WalkFinding::new(
                REF_UNRESOLVED,
                format!("read model {reference:?} is not in the walked model"),
            )
            .field(field),
        );
        None
    }

    // --- streams ---

    /// Stream instance of a payload on `lane`: `lane:stream_id` with the
    /// template's placeholders filled from the payload. Without a lane there
    /// is no stream, so lane-less commands never share history.
    fn stream_of(&mut self, lane: Option<&Key>, payload: &Map<String, Value>) -> Option<String> {
        let lane = lane?;
        let template = self
            .model
            .get(K::Swimlane, lane)
            .map(|s| str_at(s, "streamId").to_string())
            .unwrap_or_default();
        if template.is_empty() {
            return Some(lane.to_string());
        }
        let mut resolved = String::new();
        let mut rest = template.as_str();
        let mut missing = Vec::new();
        while let Some(start) = rest.find('{') {
            resolved.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}') else {
                break;
            };
            let name = &rest[start + 1..start + end];
            match payload.get(name) {
                Some(Value::String(s)) => resolved.push_str(s),
                Some(Value::Number(n)) => resolved.push_str(&number_key(n)),
                Some(v) if !v.is_null() => resolved.push_str(&v.to_string()),
                _ => {
                    missing.push(name.to_string());
                    resolved.push_str(&rest[start..=start + end]);
                }
            }
            rest = &rest[start + end + 1..];
        }
        resolved.push_str(rest);
        if !missing.is_empty() {
            let lane = lane.clone();
            self.push(
                WalkFinding::new(
                    STREAM_UNRESOLVED,
                    format!(
                        "stream id {template:?} of swimlane {lane} needs {} but the payload has none",
                        missing.join(", ")
                    ),
                )
                .subject(K::Swimlane, &lane)
                .field("stream_id"),
            );
        }
        Some(format!("{lane}:{resolved}"))
    }

    fn lane_of(&self, kind: K, key: &Key) -> Option<Key> {
        self.model
            .get(kind, key)
            .and_then(|spec| spec.get("swimlane"))
            .and_then(ref_key)
    }

    /// `Swimlane.transitions` of `lane` as (after, [next]) pairs.
    fn transitions(&self, lane: &Key) -> Vec<(Key, Vec<Key>)> {
        let Some(spec) = self.model.get(K::Swimlane, lane) else {
            return Vec::new();
        };
        arr(spec, "transitions")
            .iter()
            .filter_map(|t| {
                let after = t.get("after").and_then(ref_key)?;
                Some((after, edge_keys(t, "next", "event")))
            })
            .collect()
    }

    fn history(&self, stream: Option<&str>) -> Vec<&Recorded> {
        self.log.iter().filter(|r| r.on(stream)).collect()
    }

    // --- projection ---

    /// Stream instance an event with this payload lands on.
    fn event_stream(&mut self, event: &Key, payload: &Map<String, Value>) -> Option<String> {
        let lane = self.lane_of(K::Event, event);
        self.stream_of(lane.as_ref(), payload)
    }

    fn record(&mut self, event: &Key, payload: &Map<String, Value>, stream: Option<String>) {
        let lane = self.lane_of(K::Event, event);
        self.log.push(Recorded {
            event: event.clone(),
            lane,
            payload: payload.clone(),
            stream,
        });
        self.project(event, payload);
    }

    fn project(&mut self, event: &Key, payload: &Map<String, Value>) {
        let model = self.model;
        for (slice_key, slice) in model.live(K::ReadModelSlice) {
            let projects = arr(slice, "sourceEvents")
                .iter()
                .filter_map(|e| edge_key(e, "event"))
                .any(|source| same_event(&source, event));
            if !projects {
                continue;
            }
            let Some(rm) = edge_at(slice, "readModel", "readModel") else {
                continue;
            };
            self.touched.slices.insert(slice_key.clone());
            let rm_spec = model.get(K::ReadModel, &rm);
            let key_fields = rm_key_fields(rm_spec);
            let Some(id) = row_id(&key_fields, payload) else {
                let missing: Vec<&str> = key_fields
                    .iter()
                    .filter(|k| payload.get(k.as_str()).is_none_or(Value::is_null))
                    .map(String::as_str)
                    .collect();
                self.push(
                    WalkFinding::new(
                        PROJECTION_KEY_UNSOURCED,
                        format!(
                            "{slice_key} projects {event} into {rm}, but the event carries no {}",
                            missing.join(", ")
                        ),
                    )
                    .subject(K::ReadModelSlice, slice_key)
                    .field("read_model"),
                );
                continue;
            };
            let delete = str_at(slice, "projectionRole") == "PROJECTION_ROLE_DELETE";
            let table = self.rows.entry(rm.clone()).or_default();
            if delete {
                table.remove(&id);
            } else {
                let row = table.entry(id.clone()).or_default();
                match rm_spec.and_then(schema_fields) {
                    Some(fields) => {
                        for field in fields {
                            if let Some(v) = payload.get(&field.name) {
                                row.insert(field.name, v.clone());
                            }
                        }
                    }
                    None => row.extend(payload.clone()),
                }
            }
            let row_key: Map<String, Value> = key_fields
                .iter()
                .filter_map(|k| Some((k.clone(), payload.get(k)?.clone())))
                .collect();
            self.check_slice_scenarios(slice_key, slice, &rm, &id, &row_key, delete);
        }
    }

    /// Mark the slice's scenarios whose `when` history this walk has just
    /// completed, and check the row carries every field their `then`
    /// payload names.
    fn check_slice_scenarios(
        &mut self,
        slice_key: &Key,
        slice: &Value,
        rm: &Key,
        row_id: &str,
        row_key: &Map<String, Value>,
        delete: bool,
    ) {
        let Some(last) = self.log.last().map(|r| r.event.clone()) else {
            return;
        };
        // Only events about this row count toward a scenario's `when`; a
        // keyless read model has one row, so every event does.
        let row_history: Vec<Key> = self
            .log
            .iter()
            .filter(|r| carries_key(&r.payload, row_key))
            .map(|r| r.event.clone())
            .collect();
        for scenario in arr(slice, "scenarios") {
            let when: Vec<Key> = edge_keys(scenario, "when", "event");
            let completes = when.last().is_some_and(|w| same_event(w, &last));
            if !completes || !is_subsequence(&when, &row_history) {
                continue;
            }
            let scenario_id = str_at(scenario, "id").to_string();
            self.mark(K::ReadModelSlice, slice_key, &scenario_id);
            // `given` events are the case's own initial values, not model
            // output: they mark coverage but are not checked.
            if delete || self.step.is_none() {
                continue;
            }
            let Some(then) = scenario.get("then").and_then(payload_of) else {
                continue;
            };
            let row = self.rows.get(rm).and_then(|t| t.get(row_id));
            let missing: Vec<String> = then
                .keys()
                .filter(|f| {
                    row.and_then(|r| r.get(f.as_str()))
                        .is_none_or(Value::is_null)
                })
                .cloned()
                .collect();
            if !missing.is_empty() {
                self.push(
                    WalkFinding::new(
                        PROJECTION_FIELD_MISSING,
                        format!(
                            "scenario {scenario_id:?} of {slice_key} says {rm} holds {} after these events, but the walked row has no value for it",
                            missing.join(", ")
                        ),
                    )
                    .subject(K::ReadModelSlice, slice_key)
                    .field(format!("scenarios[{scenario_id}].then")),
                );
            }
        }
    }

    fn check_rows(&mut self, expected: &RowsValue, field: &str) {
        let Some(rm) = self.read_model(&expected.read_model, field) else {
            return;
        };
        let actual: Vec<&Map<String, Value>> = self
            .rows
            .get(&rm)
            .map(|t| t.values().collect())
            .unwrap_or_default();
        if let Some(problem) = rows_mismatch(&expected.rows, &actual) {
            self.push(
                WalkFinding::new(READ_MODEL_MISMATCH, format!("{rm}: {problem}"))
                    .subject(K::ReadModel, &rm)
                    .field(field),
            );
        }
    }

    // --- steps ---

    fn run_step(&mut self, step: &Step) {
        let model = self.model;
        let Some((_, command)) = model.resolve(&[K::Command], &step.command, self.ns()) else {
            self.push(
                WalkFinding::new(
                    REF_UNRESOLVED,
                    format!("command {:?} is not in the walked model", step.command),
                )
                .field("command"),
            );
            return;
        };
        if step.expect.emit.is_some() && step.expect.reject.is_some() {
            self.push(
                WalkFinding::new(
                    CASE_INVALID,
                    "a step expects either emit or reject, not both",
                )
                .field("expect"),
            );
            return;
        }
        // Several slices may handle one command, each describing other
        // timelines of it, so the step weighs all of their scenarios.
        let slices: Vec<(Key, &Value)> = model
            .live(K::CommandSlice)
            .filter(|(_, s)| edge_at(s, "command", "command").as_ref() == Some(&command))
            .map(|(key, s)| (key.clone(), s))
            .collect();
        let Some((slice_key, _)) = slices.first() else {
            self.push(
                WalkFinding::new(
                    COMMAND_UNHANDLED,
                    format!("no command slice handles {command}"),
                )
                .subject(K::Command, &command),
            );
            return;
        };
        let slice_key = slice_key.clone();
        for (key, _) in &slices {
            self.touched.slices.insert(key.clone());
        }

        let (values, unsupplied) = self.command_values(step, &command, &slices);

        let lane = self.lane_of(K::Command, &command);
        let stream = self.stream_of(lane.as_ref(), &values);
        let Some(emitted) = self.decide(
            step,
            &command,
            &slice_key,
            &slices,
            &values,
            stream.as_deref(),
        ) else {
            return;
        };
        self.check_accepted_on_terminal(
            lane.as_ref(),
            stream.as_deref(),
            &emitted,
            &command,
            &slice_key,
        );
        // Another lane's stream key can be swimlane state the answer gives
        // rather than a command field.
        let mut keys = values.clone();
        for (k, v) in &step.answer {
            if !v.is_null() && !keys.contains_key(k) {
                keys.insert(k.clone(), v.clone());
            }
        }
        for event in emitted {
            // The stream is fixed before the payload is built, so field
            // sourcing, lifecycle and recording share one timeline. On the
            // command's own lane it is the stream the command addressed.
            let event_lane = self.lane_of(K::Event, &event);
            let event_stream = if event_lane == lane {
                stream.clone()
            } else {
                self.stream_of(event_lane.as_ref(), &keys)
            };
            let payload = self.event_payload(
                &event,
                &command,
                &values,
                &unsupplied,
                &step.answer,
                event_stream.as_deref(),
            );
            self.check_lifecycle(
                &event,
                event_lane.as_ref(),
                event_stream.as_deref(),
                &command,
                &slice_key,
            );
            self.record(&event, &payload, event_stream);
        }
    }

    /// Rows of `rm` the step can mean: when the answer names key fields,
    /// only rows carrying the same non-null key values.
    fn matching_rows(&self, rm: &Key, answer: &Map<String, Value>) -> Vec<&Map<String, Value>> {
        let key_fields = rm_key_fields(self.model.get(K::ReadModel, rm));
        self.rows
            .get(rm)
            .map(|table| {
                table
                    .values()
                    .filter(|row| {
                        key_fields.iter().all(|k| match answer.get(k) {
                            Some(a) => row
                                .get(k)
                                .filter(|r| !r.is_null())
                                .is_some_and(|r| values_equal(a, r)),
                            None => true,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Check 2: give every command field a value. Returns the values and
    /// the required fields that got none (reported here, on the command).
    fn command_values(
        &mut self,
        step: &Step,
        command: &Key,
        slices: &[(Key, &Value)],
    ) -> (Map<String, Value>, BTreeSet<String>) {
        let model = self.model;
        // An explicit null in the answer supplies nothing.
        let answer: Map<String, Value> = step
            .answer
            .iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Who issues the command, and which read models they observe.
        let mut issuers: Vec<(K, Key, Vec<Key>)> = Vec::new();
        for (key, auto) in model.live(K::AutomationSlice) {
            if edge_at(auto, "emittedCommand", "command").as_ref() == Some(command) {
                issuers.push((K::AutomationSlice, key.clone(), source_rms(auto)));
            }
        }
        let mut uis: Vec<Key> = Vec::new();
        for ui in slices.iter().filter_map(|(_, s)| edge_at(s, "ui", "ui")) {
            if !uis.contains(&ui) {
                uis.push(ui);
            }
        }
        for ui in &uis {
            for (key, ui_slice) in model.live(K::UiSlice) {
                if edge_at(ui_slice, "ui", "ui").as_ref() == Some(ui) {
                    issuers.push((K::UiSlice, key.clone(), source_rms(ui_slice)));
                }
            }
        }
        if let Some(via) = &step.via {
            match model.resolve(&[K::AutomationSlice, K::UiSlice], via, self.ns()) {
                Some((_, via_key)) if issuers.iter().any(|(_, key, _)| *key == via_key) => {
                    issuers.retain(|(_, key, _)| *key == via_key);
                }
                Some((kind, via_key)) => {
                    self.push(
                        WalkFinding::new(
                            VIA_NOT_ISSUER,
                            format!("{via_key} does not issue {command}"),
                        )
                        .subject(kind, &via_key)
                        .field("via"),
                    );
                    issuers.clear();
                }
                None => {
                    self.push(
                        WalkFinding::new(
                            REF_UNRESOLVED,
                            format!(
                                "via {via:?} is not an automation or UI slice in the walked model"
                            ),
                        )
                        .field("via"),
                    );
                    issuers.clear();
                }
            }
        }

        let mut offers: BTreeMap<String, Vec<Offer>> = BTreeMap::new();
        let mut ambiguous: BTreeMap<String, Vec<Key>> = BTreeMap::new();
        let mut observed: Vec<Key> = Vec::new();
        for (issuer_kind, issuer, rms) in &issuers {
            self.touched.slices.insert(issuer.clone());
            for rm in rms {
                if !observed.contains(rm) {
                    observed.push(rm.clone());
                }
                let rows = self.matching_rows(rm, &answer);
                let mut fields: BTreeSet<&String> = BTreeSet::new();
                for row in &rows {
                    fields.extend(row.keys());
                }
                for field in fields {
                    let mut distinct: Vec<&Value> = Vec::new();
                    for row in &rows {
                        if let Some(v) = row.get(field).filter(|v| !v.is_null()) {
                            if !distinct.iter().any(|d| values_equal(d, v)) {
                                distinct.push(v);
                                // Two distinct values already make the field ambiguous.
                                if distinct.len() > 1 {
                                    break;
                                }
                            }
                        }
                    }
                    match distinct.as_slice() {
                        [] => {}
                        [one] => offers.entry(field.clone()).or_default().push(Offer {
                            source: rm.to_string(),
                            value: (*one).clone(),
                        }),
                        _ => {
                            let rms = ambiguous.entry(field.clone()).or_default();
                            if !rms.contains(rm) {
                                rms.push(rm.clone());
                            }
                        }
                    }
                }
            }
            if *issuer_kind == K::AutomationSlice {
                self.mark_automation_scenarios(issuer, command, &answer);
            }
        }

        let mut unsupplied = BTreeSet::new();
        let spec = model.get(K::Command, command);
        let Some(fields) = spec.and_then(schema_fields) else {
            // Nothing to check field by field: pass every observed and
            // answered value through, and say the schema was not inspected.
            let mut values: Map<String, Value> = offers
                .into_iter()
                .filter_map(|(field, o)| Some((field, o.into_iter().next()?.value)))
                .collect();
            values.extend(answer);
            if spec.is_some() {
                self.push(
                    WalkFinding::new(
                        SCHEMA_OPAQUE,
                        format!("{command}'s schema is a type-library message; its fields are not checked"),
                    )
                    .subject(K::Command, command)
                    .field("schema"),
                );
            }
            return (values, unsupplied);
        };
        let mut values = Map::new();
        for field in fields {
            let name = &field.name;
            let required = !field.optional && field.derived.is_none();
            let mut sources = offers.remove(name).unwrap_or_default();
            let from_answer = answer.get(name);
            if let Some(a) = from_answer {
                sources.push(Offer {
                    source: "the step's answer".into(),
                    value: a.clone(),
                });
            }
            let ambiguous_rms = ambiguous.get(name).map_or(&[][..], Vec::as_slice);
            if let Some(a) = from_answer {
                for rm in ambiguous_rms {
                    let held = self
                        .matching_rows(rm, &answer)
                        .iter()
                        .any(|row| row.get(name).is_some_and(|v| values_equal(v, a)));
                    if !held {
                        self.push(
                        WalkFinding::new(
                            COMMAND_FIELD_CONFLICT,
                            format!(
                                "{command} field {name:?} gets {a} from the step's answer, but no matching row of {rm} holds it"
                            ),
                        )
                        .schema_field(K::Command, command, name),
                    );
                    }
                }
            }
            let ambiguity: &[Key] = if from_answer.is_none() {
                ambiguous_rms
            } else {
                &[]
            };
            for rm in ambiguity {
                // Reported even when another source supplies the field: the
                // disagreeing rows are still a gap in the model.
                let unresolved = sources.is_empty() && required;
                let finding = WalkFinding::new(
                    COMMAND_FIELD_AMBIGUOUS,
                    format!(
                        "{command} field {name:?}: several rows of {rm} match and disagree; put {rm}'s key in the step's answer to pick one"
                    ),
                )
                .schema_field(K::Command, command, name);
                self.push(if unresolved {
                    finding.severity(Severity::Error)
                } else {
                    finding
                });
            }
            if let Some(first) = sources.first() {
                let conflict = sources
                    .iter()
                    .any(|o| !values_equal(&o.value, &first.value));
                if conflict {
                    let all: Vec<String> = sources
                        .iter()
                        .map(|o| format!("{} from {}", o.value, o.source))
                        .collect();
                    self.push(
                        WalkFinding::new(
                            COMMAND_FIELD_CONFLICT,
                            format!("{command} field {name:?} gets {}", all.join(" but ")),
                        )
                        .schema_field(K::Command, command, name),
                    );
                }
                // The answer is the last word: it is what the issuer sent.
                let chosen = from_answer.cloned().unwrap_or_else(|| first.value.clone());
                values.insert(name.clone(), chosen);
                continue;
            }
            if !ambiguity.is_empty() {
                if required {
                    unsupplied.insert(name.clone());
                }
                continue;
            }
            if let Some(kind) = &field.derived {
                values.insert(name.clone(), derived_placeholder(kind));
                continue;
            }
            if field.optional {
                continue;
            }
            let why = if observed.is_empty() {
                "no UI or automation issuing it observes a read model".to_string()
            } else {
                let carrying: Vec<String> = observed
                    .iter()
                    .filter(|rm| {
                        model
                            .get(K::ReadModel, rm)
                            .and_then(schema_fields)
                            .is_some_and(|fs| fs.iter().any(|f| f.name == *name))
                    })
                    .map(ToString::to_string)
                    .collect();
                let names: Vec<String> = observed.iter().map(ToString::to_string).collect();
                if carrying.is_empty() {
                    format!(
                        "none of the observed read models ({}) carries it",
                        names.join(", ")
                    )
                } else {
                    format!(
                        "the observed read models that declare it ({}) hold no row with a value for it",
                        carrying.join(", ")
                    )
                }
            };
            self.push(
                WalkFinding::new(
                    COMMAND_FIELD_UNSUPPLIED,
                    format!(
                        "{command} needs {name:?}, but {why}, and the step's answer does not supply it"
                    ),
                )
                .schema_field(K::Command, command, name),
            );
            unsupplied.insert(name.clone());
        }
        (values, unsupplied)
    }

    /// Mark the automation's scenarios that issue `command` when every read
    /// model their `given` names holds a row this step can mean.
    fn mark_automation_scenarios(
        &mut self,
        auto_key: &Key,
        command: &Key,
        answer: &Map<String, Value>,
    ) {
        let Some(auto) = self.model.get(K::AutomationSlice, auto_key) else {
            return;
        };
        for scenario in arr(auto, "scenarios") {
            let issues = edge_at(scenario, "then", "command").is_some_and(|c| c == *command);
            let given_hold = arr(scenario, "given").iter().all(|g| {
                edge_key(g, "readModel")
                    .is_some_and(|rm| !self.matching_rows(&rm, answer).is_empty())
            });
            if issues && given_hold {
                self.mark(K::AutomationSlice, auto_key, str_at(scenario, "id"));
            }
        }
    }

    /// Checks 5 and the outcome: decide what the step emits, or `None`
    /// when it is refused (or cannot proceed).
    fn decide(
        &mut self,
        step: &Step,
        command: &Key,
        slice_key: &Key,
        slices: &[(Key, &Value)],
        values: &Map<String, Value>,
        stream: Option<&str>,
    ) -> Option<Vec<Key>> {
        let history = self.history(stream);
        // The stream's state is its last lifecycle event; events outside
        // the lifecycle (comments, notes) do not change which scenarios
        // apply. Without a lifecycle, the last event is the state.
        let lane = self.lane_of(K::Command, command);
        let transitions = lane
            .as_ref()
            .map(|lane| self.transitions(lane))
            .unwrap_or_default();
        let state = if transitions.is_empty() {
            history.last().copied()
        } else {
            history
                .iter()
                .rev()
                .find(|r| in_lifecycle(&transitions, &r.event))
                .copied()
        };
        // Events on the command's own stream always belong to its timeline.
        // Events on other lanes join it by agreeing on shared fields; other
        // instances of its own lane never do.
        let related: Vec<&Recorded> = self
            .log
            .iter()
            .filter(|r| {
                r.on(stream)
                    || (!(lane.is_some() && r.lane == lane) && correlates(&r.payload, values))
            })
            .collect();
        let at = state.and_then(|s| related.iter().position(|r| std::ptr::eq(*r, s)));
        let mut rejects: Vec<(Key, String, String)> = Vec::new();
        let mut emits: Vec<(Key, String, Vec<Key>)> = Vec::new();
        for (owner, scenario) in slices
            .iter()
            .flat_map(|(key, slice)| arr(slice, "scenarios").iter().map(move |s| (key, s)))
        {
            let when = edge_at(scenario, "when", "command");
            if when.as_ref() != Some(command) {
                continue;
            }
            let given: Vec<Key> = edge_keys(scenario, "given", "event");
            // The scenario must describe the stream's current state: its last
            // mention of that state is the current occurrence, so what it
            // lists before or after must have happened before or after it.
            let applies = match (state, at) {
                _ if given.is_empty() => history.is_empty(),
                (None, _) => is_subsequence(&given, keys_of(&related)),
                (Some(s), Some(at)) => given
                    .iter()
                    .rposition(|g| same_event(g, &s.event))
                    .is_some_and(|j| {
                        is_subsequence(&given[..j], keys_of(&related[..at]))
                            && is_subsequence(&given[j + 1..], keys_of(&related[at + 1..]))
                    }),
                (Some(_), None) => false,
            };
            if !applies {
                continue;
            }
            let id = str_at(scenario, "id").to_string();
            if let Some(reject) = scenario.get("reject") {
                rejects.push((owner.clone(), id, str_at(reject, "reasonCode").to_string()));
            } else if let Some(emit) = scenario.get("emit") {
                let events = edge_keys(emit, "events", "event");
                emits.push((owner.clone(), id, events));
            }
        }
        let mut declared: Vec<Key> = Vec::new();
        for (_, slice) in slices {
            for event in edge_keys(slice, "emittedEvents", "event") {
                if !declared.contains(&event) {
                    declared.push(event);
                }
            }
        }
        let slice_label = if slices.len() == 1 {
            slice_key.to_string()
        } else {
            format!("the {} slices of {command}", slices.len())
        };
        let history_label = if history.is_empty() {
            "an empty stream".to_string()
        } else {
            let names: Vec<String> = history.iter().map(|r| r.event.slug.clone()).collect();
            format!("stream history [{}]", names.join(", "))
        };

        if let Some(reason) = &step.expect.reject {
            if rejects.is_empty() {
                // The model accepts the command: report it, then walk what
                // the model does so later steps see the resulting state.
                self.push(
                    WalkFinding::new(
                        REJECT_UNMODELED,
                        format!(
                            "the case expects {command} to be refused{} after {history_label}, but no reject scenario on {slice_label} matches, so the model accepts it",
                            if reason.is_empty() { String::new() } else { format!(" ({reason})") }
                        ),
                    )
                    .subject(K::CommandSlice, slice_key)
                    .field("scenarios"),
                );
            } else {
                let matching = rejects
                    .iter()
                    .find(|(_, _, code)| reason.is_empty() || code == reason);
                if let Some((owner, id, _)) = matching {
                    self.mark(K::CommandSlice, owner, id);
                } else {
                    let (owner, id, code) = &rejects[0];
                    self.mark(K::CommandSlice, owner, id);
                    self.push(
                        WalkFinding::new(
                            REJECT_REASON_MISMATCH,
                            format!(
                                "scenario {id:?} of {owner} refuses {command} with {code:?}, the case expects {reason:?}"
                            ),
                        )
                        .subject(K::CommandSlice, owner)
                        .field(format!("scenarios[{id}].reject")),
                    );
                }
                return None;
            }
        }

        if let Some(expected) = &step.expect.emit {
            if let Some((owner, id, code)) = rejects.first() {
                self.mark(K::CommandSlice, owner, id);
                self.push(
                    WalkFinding::new(
                        UNEXPECTED_REJECT,
                        format!(
                            "the case expects {command} to emit events after {history_label}, but scenario {id:?} of {owner} refuses it ({code})"
                        ),
                    )
                    .subject(K::CommandSlice, owner)
                    .field(format!("scenarios[{id}].reject")),
                );
                return None;
            }
            let mut keys = Vec::new();
            for reference in expected {
                let mut key = self.model.event_key(reference, self.ns());
                // An unpinned ref to an event outside the model takes the
                // version the slice declares.
                if let Some(d) = declared.iter().find(|d| same_event(d, &key)) {
                    key = d.clone();
                }
                if !declared.contains(&key) {
                    self.push(
                        WalkFinding::new(
                            EVENT_NOT_EMITTED_BY_SLICE,
                            format!("the case expects {key}, which is not among the events {slice_label} emits"),
                        )
                        .subject(K::CommandSlice, slice_key)
                        .field("emitted_events"),
                    );
                }
                keys.push(key);
            }
            let label = |events: &[Key]| {
                events
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            if let Some((owner, id, _)) = emits.iter().find(|(_, _, events)| *events == keys) {
                self.mark(K::CommandSlice, owner, id);
                return Some(keys);
            }
            if let Some((owner, id, events)) = emits.into_iter().next() {
                // A scenario decides this step and it says something else:
                // the model wins, and the contradiction is an error.
                self.mark(K::CommandSlice, &owner, &id);
                self.push(
                    WalkFinding::new(
                        EMIT_MISMATCH,
                        format!(
                            "the case expects {command} to emit [{}] after {history_label}, but scenario {id:?} of {owner} emits [{}]",
                            label(&keys),
                            label(&events)
                        ),
                    )
                    .subject(K::CommandSlice, &owner)
                    .field(format!("scenarios[{id}].emit")),
                );
                return Some(events);
            }
            // The model cannot produce an event the slice does not emit, so
            // later steps must not see it.
            if keys.iter().any(|k| !declared.contains(k)) {
                return None;
            }
            self.push(
                WalkFinding::new(
                    SCENARIO_MISSING,
                    format!(
                        "no scenario on {slice_label} has {command} emit [{}] after {history_label}",
                        label(&keys)
                    ),
                )
                .subject(K::CommandSlice, slice_key)
                .field("scenarios"),
            );
            return Some(keys);
        }

        // No stated outcome: the model decides.
        if let Some((owner, id, _)) = rejects.first() {
            self.mark(K::CommandSlice, owner, id);
            return None;
        }
        if let Some((owner, id, events)) = emits.into_iter().next() {
            self.mark(K::CommandSlice, &owner, &id);
            return Some(events);
        }
        if let [only] = declared.as_slice() {
            self.push(
                WalkFinding::new(
                    SCENARIO_MISSING,
                    format!(
                        "no scenario on {slice_label} covers {command} after {history_label}; walking its only emitted event {only}"
                    ),
                )
                .subject(K::CommandSlice, slice_key)
                .field("scenarios"),
            );
            return Some(vec![only.clone()]);
        }
        self.push(
            WalkFinding::new(
                OUTCOME_UNDETERMINED,
                format!(
                    "no scenario on {slice_label} covers {command} after {history_label}, and it emits {} events; state expect.emit or expect.reject",
                    declared.len()
                ),
            )
            .subject(K::CommandSlice, slice_key)
            .field("scenarios"),
        );
        None
    }

    fn mark(&mut self, kind: K, slice: &Key, scenario: &str) {
        self.touched
            .scenarios
            .insert((kind, slice.clone(), scenario.to_string()));
    }

    /// Check 3: give every field of an emitted event a value.
    fn event_payload(
        &mut self,
        event: &Key,
        command: &Key,
        values: &Map<String, Value>,
        unsupplied: &BTreeSet<String>,
        answer: &Map<String, Value>,
        stream: Option<&str>,
    ) -> Map<String, Value> {
        let model = self.model;
        let spec = model.get(K::Event, event);
        let Some(fields) = spec.and_then(schema_fields) else {
            if spec.is_some() {
                self.push(
                    WalkFinding::new(
                        SCHEMA_OPAQUE,
                        format!("{event}'s schema is a type-library message; its fields are not checked"),
                    )
                    .subject(K::Event, event)
                    .field("schema"),
                );
            }
            return values.clone();
        };
        let lane = self.lane_of(K::Event, event);
        let state: Vec<Field> = lane
            .as_ref()
            .and_then(|l| model.get(K::Swimlane, l))
            .map(|s| field_specs(arr(s, "state")))
            .unwrap_or_default();
        let earlier = self.history(stream);
        let mut payload = Map::new();
        let mut unsourced = Vec::new();
        for field in fields {
            let name = &field.name;
            let is_state = state.iter().any(|s| s.name == *name);
            if let Some(v) = values.get(name) {
                payload.insert(name.clone(), v.clone());
            } else if let Some(v) = answer.get(name).filter(|v| is_state && !v.is_null()) {
                // Aggregate state is set by the runtime; the step's answer
                // is where a case states what the runtime decided.
                payload.insert(name.clone(), v.clone());
            } else if let Some(v) = earlier
                .iter()
                .rev()
                .find_map(|r| r.payload.get(name))
                .filter(|v| !v.is_null())
            {
                // The latest event carrying the field wins: a null there
                // cleared it, so an older value does not come back.
                payload.insert(name.clone(), v.clone());
            } else if is_state {
                payload.insert(name.clone(), Value::String(format!("<state:{name}>")));
            } else if let Some(kind) = &field.derived {
                payload.insert(name.clone(), derived_placeholder(kind));
            } else if !field.optional && !unsupplied.contains(name) {
                // A command field the walk could not supply was already
                // reported on the command; any other gap is the event's.
                unsourced.push(field.name);
            }
        }
        for name in unsourced {
            self.push(
                WalkFinding::new(
                    EVENT_FIELD_UNSOURCED,
                    format!(
                        "{event} needs {name:?}, but {command} does not carry it, no earlier event on the stream recorded it, and the swimlane state does not declare it"
                    ),
                )
                .schema_field(K::Event, event, &name),
            );
        }
        payload
    }

    fn last_lifecycle(&self, transitions: &[(Key, Vec<Key>)], stream: &str) -> Option<Key> {
        self.history(Some(stream))
            .into_iter()
            .rev()
            .map(|r| r.event.clone())
            .find(|k| in_lifecycle(transitions, k))
    }

    /// A command accepted on a stream that already ended is a gap even when
    /// its events land on other lanes; ones landing here are checked per event.
    fn check_accepted_on_terminal(
        &mut self,
        lane: Option<&Key>,
        stream: Option<&str>,
        emitted: &[Key],
        command: &Key,
        slice_key: &Key,
    ) {
        let (Some(lane), Some(stream)) = (lane, stream) else {
            return;
        };
        if emitted
            .iter()
            .any(|e| self.lane_of(K::Event, e).as_ref() == Some(lane))
        {
            return;
        }
        let transitions = self.transitions(lane);
        let Some(prev) = self
            .last_lifecycle(&transitions, stream)
            .filter(|p| is_terminal(&transitions, p))
        else {
            return;
        };
        self.push(
            WalkFinding::new(
                ACCEPTED_AFTER_TERMINAL,
                format!(
                    "{command} was accepted after terminal event {prev} on {stream}; no reject scenario on {slice_key} refuses it"
                ),
            )
            .subject(K::CommandSlice, slice_key)
            .field("scenarios"),
        );
    }

    /// Checks 1 and 5: the event must be a legal successor of the stream's
    /// previous lifecycle event.
    fn check_lifecycle(
        &mut self,
        event: &Key,
        lane: Option<&Key>,
        stream: Option<&str>,
        command: &Key,
        slice_key: &Key,
    ) {
        let (Some(lane), Some(stream)) = (lane, stream) else {
            return;
        };
        let transitions = self.transitions(lane);
        if transitions.is_empty() {
            return;
        }
        let previous = self.last_lifecycle(&transitions, stream);
        // After a terminal event nothing may land on the stream, lifecycle
        // event or not.
        if let Some(prev) = &previous {
            if is_terminal(&transitions, prev) {
                self.push(
                    WalkFinding::new(
                        ACCEPTED_AFTER_TERMINAL,
                        format!(
                            "{command} emitted {event} after terminal event {prev} on {stream}; no reject scenario on {slice_key} refuses it"
                        ),
                    )
                    .subject(K::CommandSlice, slice_key)
                    .field("scenarios"),
                );
                return;
            }
        }
        if !in_lifecycle(&transitions, event) {
            return;
        }
        match previous {
            None => {
                let only_successor = transitions.iter().any(|(_, n)| n.contains(event))
                    && !transitions.iter().any(|(a, _)| a == event);
                if only_successor {
                    self.push(
                        WalkFinding::new(
                            LIFECYCLE_START,
                            format!("{event} opens stream {stream}, but the lifecycle only reaches it as a successor"),
                        )
                        .subject(K::Swimlane, lane)
                        .field("transitions"),
                    );
                }
            }
            // A terminal predecessor was reported above; otherwise the
            // event must be one of the predecessor's successors.
            Some(prev) => {
                if let Some((_, next)) = transitions
                    .iter()
                    .find(|(a, _)| *a == prev)
                    .filter(|(_, next)| !next.contains(event))
                {
                    let legal: Vec<String> = next.iter().map(ToString::to_string).collect();
                    self.push(
                        WalkFinding::new(
                            LIFECYCLE_VIOLATION,
                            format!(
                                "{event} cannot follow {prev} on {stream}; the lifecycle allows [{}]",
                                legal.join(", ")
                            ),
                        )
                        .subject(K::Swimlane, lane)
                        .field("transitions"),
                    );
                }
            }
        }
    }
}

fn keys_of<'a>(records: &'a [&'a Recorded]) -> impl Iterator<Item = &'a Key> {
    records.iter().map(|r| &r.event)
}

fn is_terminal(transitions: &[(Key, Vec<Key>)], event: &Key) -> bool {
    !transitions
        .iter()
        .any(|(a, next)| a == event && !next.is_empty())
}

/// Stand-in value for a field the runtime generates.
fn derived_placeholder(kind: &str) -> Value {
    Value::String(format!("<derived:{kind}>"))
}

/// Whether `v` is a stand-in the walk put in place of a runtime value.
fn is_placeholder(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        (s.starts_with("<derived:") || s.starts_with("<state:")) && s.ends_with('>')
    })
}

fn source_rms(slice: &Value) -> Vec<Key> {
    edge_keys(slice, "sourceReadModels", "readModel")
}

fn rm_key_fields(rm: Option<&Value>) -> Vec<String> {
    rm.map(|spec| {
        arr(spec, "key")
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect()
    })
    .unwrap_or_default()
}

/// Row address: the key field values, or the single row of a keyless read
/// model. `None` when a key field is missing.
fn row_id(key_fields: &[String], payload: &Map<String, Value>) -> Option<String> {
    let mut parts = Vec::new();
    for k in key_fields {
        let v = payload.get(k).filter(|v| !v.is_null())?;
        // Tagged so `4` and `4.0` share a row but the string "4" does not.
        parts.push(match v {
            Value::Number(n) => serde_json::json!(["n", number_key(n)]),
            other => serde_json::json!(["v", other]),
        });
    }
    Some(Value::Array(parts).to_string())
}

/// Whether an event is about the row addressed by `row_key`: it carries
/// every key part. A keyless read model has one row, so every event is.
fn carries_key(payload: &Map<String, Value>, row_key: &Map<String, Value>) -> bool {
    row_key
        .iter()
        .all(|(k, v)| payload.get(k).is_some_and(|p| values_equal(p, v)))
}

/// An earlier event belongs to the same timeline as the command when they
/// share at least one field and agree on every shared field.
///
/// Placeholders stand for values the runtime picks, so they say nothing about
/// which timeline an event belongs to and are left out of the comparison.
fn correlates(payload: &Map<String, Value>, values: &Map<String, Value>) -> bool {
    let mut shared = 0;
    for (k, v) in values {
        if is_placeholder(v) {
            continue;
        }
        if let Some(p) = payload.get(k).filter(|p| !is_placeholder(p)) {
            if !values_equal(p, v) {
                return false;
            }
            shared += 1;
        }
    }
    shared > 0
}

fn is_subsequence<'k>(needle: &[Key], haystack: impl IntoIterator<Item = &'k Key>) -> bool {
    let mut it = haystack.into_iter();
    needle.iter().all(|n| it.any(|h| same_event(n, h)))
}

/// Whether a model ref and a recorded event name the same event. A recorded
/// event at version 0 is an unpinned event from outside the walked model
/// and matches any version of the same namespace and slug.
fn same_event(model_ref: &Key, recorded: &Key) -> bool {
    model_ref == recorded
        || (recorded.version == 0
            && model_ref.namespace == recorded.namespace
            && model_ref.slug == recorded.slug)
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => number_key(x) == number_key(y),
        _ => a == b,
    }
}

/// A canonical form of a JSON number: integers compare exactly (no `f64`
/// rounding above 2^53), and a float with an integral value equals the
/// integer (`4.0` == `4`).
fn number_key(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    match n.as_f64() {
        // `{:.0}` prints an integral f64's exact value at any magnitude.
        Some(f) if f.fract() == 0.0 => format!("{f:.0}"),
        Some(f) => f.to_string(),
        None => n.to_string(),
    }
}

fn row_matches(expected: &Map<String, Value>, actual: &Map<String, Value>) -> bool {
    expected
        .iter()
        .all(|(k, v)| actual.get(k).map_or(v.is_null(), |a| values_equal(v, a)))
}

/// Bipartite matching costs up to rows³ steps, and the MCP tool runs it
/// inline, so expected row lists are capped well above any hand-written case.
const MAX_MATCHED_ROWS: usize = 256;

fn rows_mismatch(
    expected: &[Map<String, Value>],
    actual: &[&Map<String, Value>],
) -> Option<String> {
    let render = |rows: &[&Map<String, Value>]| {
        let values: Vec<Value> = rows.iter().map(|r| Value::Object((*r).clone())).collect();
        Value::Array(values).to_string()
    };
    if expected.len() != actual.len() {
        return Some(format!(
            "expected {} row(s), the walk left {}: {}",
            expected.len(),
            actual.len(),
            render(actual)
        ));
    }
    if expected.len() > MAX_MATCHED_ROWS {
        return Some(format!(
            "the case lists {} rows; the walk compares at most {MAX_MATCHED_ROWS} per read model",
            expected.len()
        ));
    }
    // Expectations may be partial, so pair them with rows as a bipartite
    // matching: the result must not depend on the order they are listed in.
    let candidates: Vec<Vec<usize>> = expected
        .iter()
        .map(|row| {
            actual
                .iter()
                .enumerate()
                .filter(|(_, a)| row_matches(row, a))
                .map(|(i, _)| i)
                .collect()
        })
        .collect();
    let mut owner: Vec<Option<usize>> = vec![None; actual.len()];
    for (e, row) in expected.iter().enumerate() {
        let mut seen = vec![false; actual.len()];
        if !augment(e, &candidates, &mut owner, &mut seen) {
            return Some(format!(
                "no row matches {}; the walk left {}",
                Value::Object(row.clone()),
                render(actual)
            ));
        }
    }
    None
}

/// Kuhn's augmenting path: give expectation `e` an actual row, moving
/// earlier expectations to other rows they also match when needed.
fn augment(
    e: usize,
    candidates: &[Vec<usize>],
    owner: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    for &a in &candidates[e] {
        if seen[a] {
            continue;
        }
        seen[a] = true;
        if owner[a].is_none_or(|other| augment(other, candidates, owner, seen)) {
            owner[a] = Some(e);
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

fn coverage(
    model: &Model,
    namespaces: &BTreeSet<&str>,
    touched: &Touched,
    findings: &mut Vec<WalkFinding>,
) -> Coverage {
    let mut cov = Coverage::default();
    let in_scope = |key: &Key| namespaces.contains(key.namespace.as_str());

    for (key, board) in model.live(K::Storyboard) {
        if !in_scope(key) {
            continue;
        }
        cov.storyboards_total += 1;
        let walked = arr(board, "slices")
            .iter()
            .filter_map(ref_key)
            .any(|slice| touched.slices.contains(&slice));
        if walked {
            cov.storyboards_walked += 1;
        } else {
            findings.push(
                WalkFinding::new(
                    STORYBOARD_UNWALKED,
                    format!("no case walked storyboard {key}"),
                )
                .subject(K::Storyboard, key),
            );
        }
    }

    for kind in [K::CommandSlice, K::ReadModelSlice, K::AutomationSlice] {
        for (key, slice) in model.live(kind) {
            if !in_scope(key) {
                continue;
            }
            for scenario in arr(slice, "scenarios") {
                // No step issues a command when the automation stays idle, so
                // nothing a case does can walk such a scenario.
                if kind == K::AutomationSlice && scenario.get("noCommand").is_some() {
                    continue;
                }
                let id = str_at(scenario, "id").to_string();
                cov.scenarios_total += 1;
                if touched.scenarios.contains(&(kind, key.clone(), id.clone())) {
                    cov.scenarios_walked += 1;
                } else {
                    findings.push(
                        WalkFinding::new(
                            SCENARIO_UNWALKED,
                            format!("no case step matched scenario {id:?} of {key}"),
                        )
                        .subject(kind, key)
                        .field(format!("scenarios[{id}]")),
                    );
                }
            }
        }
    }
    cov
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn map(v: Value) -> Map<String, Value> {
        let Value::Object(m) = v else { unreachable!() };
        m
    }

    #[test]
    fn numbers_compare_exactly_and_ignore_float_form() {
        assert!(values_equal(&json!(4), &json!(4.0)));
        assert!(!values_equal(
            &json!(9_007_199_254_740_992_u64),
            &json!(9_007_199_254_740_993_u64)
        ));
        let keys = vec!["n".to_string()];
        assert_eq!(
            row_id(&keys, &map(json!({"n": 4}))),
            row_id(&keys, &map(json!({"n": 4.0})))
        );
    }

    #[test]
    fn partial_row_expectations_pair_one_to_one_in_any_order() {
        let r1 = map(json!({"review_id": "r-1", "status": "submitted"}));
        let r2 = map(json!({"review_id": "r-2", "status": "submitted"}));
        let actual = vec![&r1, &r2];
        let expected = vec![
            map(json!({"status": "submitted"})),
            map(json!({"review_id": "r-1"})),
        ];
        assert_eq!(rows_mismatch(&expected, &actual), None);
        let impossible = vec![
            map(json!({"review_id": "r-1"})),
            map(json!({"review_id": "r-1"})),
        ];
        assert!(rows_mismatch(&impossible, &actual).is_some());
    }

    #[test]
    fn an_after_entry_with_no_successor_is_terminal() {
        let key = |slug: &str| Key {
            namespace: "n".into(),
            slug: slug.into(),
            version: 1,
        };
        let transitions = vec![(key("a"), vec![key("b")]), (key("b"), vec![])];
        assert!(!is_terminal(&transitions, &key("a")));
        assert!(is_terminal(&transitions, &key("b")));
    }

    #[test]
    fn extra_namespaces_add_to_the_cases_own() {
        let cases = parse_cases(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Specification\nmetadata: {namespace: local, name: c}\nspec: {}\n",
            "inline",
        )
        .unwrap_or_default();
        let extra = vec!["upstream".to_string()];
        let scope = fetch_scope(&extra, &cases);
        assert_eq!(
            scope.into_iter().collect::<Vec<_>>(),
            vec!["local", "upstream"]
        );
    }

    #[test]
    fn large_integral_floats_equal_their_exact_integer() {
        assert!(values_equal(
            &json!(9_223_372_036_854_775_808_u64),
            &json!(9_223_372_036_854_775_808.0_f64)
        ));
    }

    #[test]
    fn numeric_and_string_row_keys_stay_distinct() {
        let keys = vec!["n".to_string()];
        assert_ne!(
            row_id(&keys, &map(json!({"n": 4}))),
            row_id(&keys, &map(json!({"n": "#4"})))
        );
    }

    #[test]
    fn row_history_needs_every_key_part() {
        let key = map(json!({"tenant_id": "t-1", "review_id": "r-2"}));
        assert!(!carries_key(&map(json!({"tenant_id": "t-1"})), &key));
        assert!(carries_key(
            &map(json!({"tenant_id": "t-1", "review_id": "r-2", "x": 1})),
            &key
        ));
        assert!(!carries_key(
            &map(json!({"tenant_id": "t-1", "review_id": "r-1"})),
            &key
        ));
    }

    #[test]
    fn row_matching_is_bounded() {
        let row = map(json!({"status": "submitted"}));
        let actual = vec![&row; MAX_MATCHED_ROWS + 1];
        let expected = vec![row.clone(); MAX_MATCHED_ROWS + 1];
        let problem = rows_mismatch(&expected, &actual);
        assert!(
            problem.as_deref().is_some_and(|p| p.contains("at most")),
            "{problem:?}"
        );
        let fits = vec![row.clone(); MAX_MATCHED_ROWS];
        assert_eq!(rows_mismatch(&fits, &actual[..MAX_MATCHED_ROWS]), None);
    }

    #[test]
    fn placeholders_do_not_decide_correlation() {
        let values = map(json!({"review_id": "r-1", "updated_at": "<derived:timestamp>"}));
        // A real value for a field the command only has a placeholder for
        // must not split the timeline.
        let same = map(json!({"review_id": "r-1", "updated_at": "2026-01-01T00:00:00Z"}));
        assert!(correlates(&same, &values));
        // Matching placeholders alone must not join unrelated timelines.
        let other = map(json!({"review_id": "r-2", "updated_at": "<derived:timestamp>"}));
        assert!(!correlates(&other, &values));
        let only_placeholder = map(json!({"updated_at": "<derived:timestamp>"}));
        assert!(!correlates(&only_placeholder, &values));
    }
}
