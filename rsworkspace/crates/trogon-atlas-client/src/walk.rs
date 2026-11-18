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
//! produced them. The walker reads manifest JSON (what `export` writes and
//! `apply` reads), so it runs offline against manifest files or against a
//! namespace fetched from the live store.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
};

use anyhow::{anyhow, Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use trogon_atlas_core::transcode::TranscodePool;
use trogon_atlas_proto as pb;

use crate::{client::Client, export, manifest};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
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
        $(const $ident: &str = $code;)*
        /// Every code the walker can emit.
        pub const RULES: &[WalkRule] = &[
            $(WalkRule { code: $code, default_severity: Severity::$sev, title: $title, doc: $doc },)*
        ];
    };
}

rules! {
    REF_UNRESOLVED = "WALK_REF_UNRESOLVED", Error, "Case names an entity the model lacks",
        "A step's command, a `via` slice, or an expected read model does not resolve in the walked model. Refs are `slug`, `namespace/slug` or `namespace/slug@version`; a bare slug resolves in the case's namespace and an unpinned ref resolves to the latest version.";
    CASE_INVALID = "WALK_CASE_INVALID", Error, "Case is malformed",
        "A step expects both an emit and a reject; state one of them, or neither to let the model decide.";
    COMMAND_UNHANDLED = "WALK_COMMAND_UNHANDLED", Error, "No command slice handles the command",
        "A step issues a command that no CommandSlice names, so the model says nothing about what the command does.";
    COMMAND_FIELD_UNSUPPLIED = "WALK_COMMAND_FIELD_UNSUPPLIED", Error, "Command field gets no value",
        "At this step no read model observed by the UI or automation that issues the command holds a value for the field, and the step's external answer does not supply it. Project the field into an observed read model, observe a read model that carries it, mark it with a DerivedFieldAnnotation, or document it as an external answer.";
    COMMAND_FIELD_CONFLICT = "WALK_COMMAND_FIELD_CONFLICT", Error, "Command field sources disagree",
        "Two sources (observed read-model rows or the external answer) offer different values for one command field. The model does not say which one wins.";
    COMMAND_FIELD_AMBIGUOUS = "WALK_COMMAND_FIELD_AMBIGUOUS", Warning, "Observed rows disagree on a command field",
        "Several rows of an observed read model match the step and hold different values for the field. Put the read model's key fields in the step's answer to pick the row.";
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

fn rule(code: &str) -> &'static WalkRule {
    RULES.iter().find(|r| r.code == code).unwrap_or(&RULES[0])
}

// ---------------------------------------------------------------------------
// Findings and report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Key {
    pub namespace: String,
    pub slug: String,
    pub version: u64,
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}@{}", self.namespace, self.slug, self.version)
    }
}

/// `ValidationIssue.subject`: an `EntityRef` in proto JSON form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Subject {
    pub kind: String,
    pub id: Key,
}

/// One finding, in the `ValidationIssue` shape plus the case location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        let r = rule(code);
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

    fn subject(mut self, kind: &str, key: &Key) -> Self {
        self.subject = Some(Subject {
            kind: entity_kind_enum(kind),
            id: key.clone(),
        });
        self
    }

    fn schema_field(self, kind: &str, key: &Key, name: &str) -> Self {
        self.subject(kind, key)
            .field(format!("schema.fields[{name}]"))
    }

    fn field(mut self, field: impl Into<String>) -> Self {
        self.subject_field = field.into();
        self
    }

    /// `ERROR WALK_X [case step 2] message (subject)`.
    pub fn render(&self) -> String {
        let mut out = format!("{:<7} {}", self.severity.label().to_uppercase(), self.code);
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

/// Manifest specs keyed by manifest kind, then exact id. Refs resolve to
/// the exact version they pin; iteration over "live" entities sees only the
/// latest version of each slug, so a superseded slice never projects.
#[derive(Debug, Default, Clone)]
pub struct Model {
    docs: BTreeMap<String, BTreeMap<Key, Value>>,
    /// Per kind, the latest version of every slug; built once.
    live: BTreeMap<String, Vec<Key>>,
}

impl Model {
    /// Build from manifest documents (`{kind, metadata, spec}` JSON).
    pub fn from_manifest_docs(docs: impl IntoIterator<Item = Value>) -> Result<Self> {
        let mut model = Model::default();
        for mut doc in docs {
            let kind = doc
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("manifest document has no kind"))?
                .to_string();
            let meta = doc.get("metadata").unwrap_or(&Value::Null);
            let key = Key {
                namespace: str_at(meta, "namespace").to_string(),
                slug: str_at(meta, "name").to_string(),
                version: meta.get("version").and_then(as_u64).unwrap_or(1),
            };
            let spec = doc
                .as_object_mut()
                .and_then(|d| d.remove("spec"))
                .unwrap_or(Value::Null);
            model.docs.entry(kind).or_default().insert(key, spec);
        }
        for (kind, entities) in &model.docs {
            // Keys sort by (namespace, slug, version): the last of each
            // (namespace, slug) run is its latest version.
            let mut latest: Vec<Key> = Vec::new();
            for key in entities.keys() {
                match latest.last_mut() {
                    Some(last) if last.namespace == key.namespace && last.slug == key.slug => {
                        *last = key.clone();
                    }
                    _ => latest.push(key.clone()),
                }
            }
            model.live.insert(kind.clone(), latest);
        }
        Ok(model)
    }

    /// Build from stored or parsed entities, rendered the way `export`
    /// renders them. Entities that cannot be rendered are returned by
    /// label, never dropped silently.
    pub fn from_entities(entities: &[pb::Entity], types: &TranscodePool) -> (Self, Vec<String>) {
        let (docs, skipped) = export::entities_to_docs(entities, types);
        // Every doc came out of entity_to_manifest, so each has a kind.
        let model = Self::from_manifest_docs(docs).unwrap_or_default();
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
        let namespaces: BTreeSet<&str> = if namespaces.is_empty() {
            cases.iter().map(|c| c.namespace.as_str()).collect()
        } else {
            namespaces.iter().map(String::as_str).collect()
        };
        let mut docs = Vec::new();
        let mut skipped = Vec::new();
        for namespace in namespaces {
            trogon_atlas_core::validate_id_component(namespace).map_err(|e| {
                crate::failure::invalid_input(format!("namespace {namespace:?}: {e}"))
            })?;
            let (ns_docs, ns_skipped) = export::namespace_manifest_docs(client, namespace).await?;
            docs.extend(ns_docs);
            skipped.extend(ns_skipped);
        }
        Ok((Self::from_manifest_docs(docs)?, skipped))
    }

    fn get(&self, kind: &str, key: &Key) -> Option<&Value> {
        self.docs.get(kind)?.get(key)
    }

    fn latest_version(&self, kind: &str, namespace: &str, slug: &str) -> Option<u64> {
        let bound = |version| Key {
            namespace: namespace.to_string(),
            slug: slug.to_string(),
            version,
        };
        self.docs
            .get(kind)?
            .range(bound(0)..=bound(u64::MAX))
            .next_back()
            .map(|(key, _)| key.version)
    }

    /// Latest version of every entity of `kind`.
    fn live(&self, kind: &str) -> Vec<(&Key, &Value)> {
        let (Some(keys), Some(docs)) = (self.live.get(kind), self.docs.get(kind)) else {
            return Vec::new();
        };
        keys.iter()
            .filter_map(|key| docs.get_key_value(key))
            .collect()
    }

    /// Resolve a case ref (`slug`, `ns/slug`, `ns/slug@v`) against `kinds`.
    fn resolve(&self, kinds: &[&str], reference: &str, default_ns: &str) -> Option<(String, Key)> {
        let (namespace, slug, version) = split_ref(reference, default_ns);
        for kind in kinds {
            let version = match version {
                Some(v) => v,
                None => match self.latest_version(kind, &namespace, &slug) {
                    Some(v) => v,
                    None => continue,
                },
            };
            let key = Key {
                namespace: namespace.clone(),
                slug: slug.clone(),
                version,
            };
            if self.get(kind, &key).is_some() {
                return Some(((*kind).to_string(), key));
            }
        }
        None
    }

    /// Like `resolve`, but an event the model lacks (one owned by a
    /// namespace that is not loaded) still yields a key, so `given` can
    /// carry upstream events that read-model slices project.
    fn event_key(&self, reference: &str, default_ns: &str) -> Key {
        if let Some((_, key)) = self.resolve(&["Event"], reference, default_ns) {
            return key;
        }
        let (namespace, slug, version) = split_ref(reference, default_ns);
        Key {
            namespace,
            slug,
            version: version.unwrap_or(1),
        }
    }
}

fn split_ref(reference: &str, default_ns: &str) -> (String, String, Option<u64>) {
    let (path, version) = match reference.rsplit_once('@') {
        Some((path, v)) => (path, v.parse::<u64>().ok()),
        None => (reference, None),
    };
    let (namespace, slug) = match path.split_once('/') {
        Some((ns, slug)) => (ns.to_string(), slug.to_string()),
        None => (default_ns.to_string(), path.to_string()),
    };
    (namespace, slug, version)
}

fn entity_kind_enum(manifest_kind: &str) -> String {
    pb::canonical::parse_kind(manifest_kind)
        .map_or("ENTITY_KIND_UNSPECIFIED", |k| k.as_str_name())
        .to_string()
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
                if !t.ends_with(DERIVED_TYPE_SUFFIX) || str_at(m, "doc").is_empty() {
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
        cases.push(Case {
            source: at,
            namespace: doc.metadata.namespace,
            name: doc.metadata.name,
            spec: doc.spec,
        });
    }
    Ok(cases)
}

/// Read cases from files or directories (walked for `.yaml`/`.yml`).
pub fn read_case_paths(paths: &[PathBuf]) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for file in manifest::collect_yaml_files(paths)? {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        cases.extend(parse_cases(&text, &file.display().to_string())?);
    }
    Ok(cases)
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Recorded {
    event: Key,
    payload: Map<String, Value>,
    stream: String,
}

type Rows = BTreeMap<String, Map<String, Value>>;

#[derive(Debug, Default)]
struct Touched {
    /// Every slice a case exercised.
    slices: BTreeSet<Key>,
    /// (slice key, scenario id) of every scenario a step matched.
    scenarios: BTreeSet<(Key, String)>,
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
    let mut seen = Vec::new();
    findings.retain(|f| {
        if seen.contains(f) {
            false
        } else {
            seen.push(f.clone());
            true
        }
    });
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
            let key_fields = rm_key_fields(self.model.get("ReadModel", &rm));
            let table = self.rows.entry(rm).or_default();
            for (i, row) in rows.rows.iter().enumerate() {
                let id = row_id(&key_fields, row).unwrap_or_else(|| format!("given#{i}"));
                table.insert(id, row.clone());
            }
        }
        for event in &case.spec.given.events {
            let key = self.model.event_key(&event.event, self.ns());
            self.record(&key, &event.payload);
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
        if let Some((_, key)) = self.model.resolve(&["ReadModel"], reference, self.ns()) {
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
    /// template's placeholders filled from the payload.
    fn stream_of(&mut self, lane: Option<&Key>, payload: &Map<String, Value>) -> String {
        let Some(lane) = lane else {
            return "<no swimlane>".to_string();
        };
        let template = self
            .model
            .get("Swimlane", lane)
            .map(|s| str_at(s, "streamId").to_string())
            .unwrap_or_default();
        if template.is_empty() {
            return lane.to_string();
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
                .subject("Swimlane", &lane)
                .field("stream_id"),
            );
        }
        format!("{lane}:{resolved}")
    }

    fn lane_of(&self, kind: &str, key: &Key) -> Option<Key> {
        self.model
            .get(kind, key)
            .and_then(|spec| spec.get("swimlane"))
            .and_then(ref_key)
    }

    fn history(&self, stream: &str) -> Vec<&Recorded> {
        self.log.iter().filter(|r| r.stream == stream).collect()
    }

    // --- projection ---

    fn record(&mut self, event: &Key, payload: &Map<String, Value>) {
        let lane = self.lane_of("Event", event);
        let stream = self.stream_of(lane.as_ref(), payload);
        self.log.push(Recorded {
            event: event.clone(),
            payload: payload.clone(),
            stream,
        });
        self.project(event, payload);
    }

    fn project(&mut self, event: &Key, payload: &Map<String, Value>) {
        let model = self.model;
        for (slice_key, slice) in model.live("ReadModelSlice") {
            let projects = arr(slice, "sourceEvents")
                .iter()
                .any(|e| edge_key(e, "event").as_ref() == Some(event));
            if !projects {
                continue;
            }
            let Some(rm) = edge_at(slice, "readModel", "readModel") else {
                continue;
            };
            self.touched.slices.insert(slice_key.clone());
            let rm_spec = model.get("ReadModel", &rm);
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
                    .subject("ReadModelSlice", slice_key)
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
            self.check_slice_scenarios(slice_key, slice, &rm, &id, delete);
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
        delete: bool,
    ) {
        let Some(last) = self.log.last().map(|r| r.event.clone()) else {
            return;
        };
        for scenario in arr(slice, "scenarios") {
            let when: Vec<Key> = edge_keys(scenario, "when", "event");
            let log = self.log.iter().map(|r| &r.event);
            if when.last() != Some(&last) || !is_subsequence(&when, log) {
                continue;
            }
            let scenario_id = str_at(scenario, "id").to_string();
            self.mark(slice_key, &scenario_id);
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
                    .subject("ReadModelSlice", slice_key)
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
                    .subject("ReadModel", &rm)
                    .field(field),
            );
        }
    }

    // --- steps ---

    fn run_step(&mut self, step: &Step) {
        let model = self.model;
        let Some((_, command)) = model.resolve(&["Command"], &step.command, self.ns()) else {
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
        let Some((slice_key, slice)) = model.live("CommandSlice").into_iter().find(|(_, s)| {
            s.get("command")
                .and_then(|e| edge_key(e, "command"))
                .as_ref()
                == Some(&command)
        }) else {
            self.push(
                WalkFinding::new(
                    COMMAND_UNHANDLED,
                    format!("no command slice handles {command}"),
                )
                .subject("Command", &command),
            );
            return;
        };
        let slice_key = slice_key.clone();
        self.touched.slices.insert(slice_key.clone());

        let values = self.command_values(step, &command, slice);

        let lane = self.lane_of("Command", &command);
        let stream = self.stream_of(lane.as_ref(), &values);
        let Some(emitted) = self.decide(step, &command, &slice_key, slice, &values, &stream) else {
            return;
        };
        for event in emitted {
            let payload = self.event_payload(&event, &command, &values, &step.answer, &stream);
            self.check_lifecycle(&event, &payload, &command, &slice_key);
            self.record(&event, &payload);
        }
    }

    /// Check 2: give every command field a value.
    fn command_values(&mut self, step: &Step, command: &Key, slice: &Value) -> Map<String, Value> {
        let model = self.model;
        // Who issues the command, and which read models they observe.
        let mut issuers: Vec<(&str, Key, Vec<Key>)> = Vec::new();
        for (key, auto) in model.live("AutomationSlice") {
            if edge_at(auto, "emittedCommand", "command").as_ref() == Some(command) {
                issuers.push(("AutomationSlice", key.clone(), source_rms(auto)));
            }
        }
        if let Some(ui) = edge_at(slice, "ui", "ui") {
            for (key, ui_slice) in model.live("UiSlice") {
                if edge_at(ui_slice, "ui", "ui").as_ref() == Some(&ui) {
                    issuers.push(("UiSlice", key.clone(), source_rms(ui_slice)));
                }
            }
        }
        if let Some(via) = &step.via {
            if let Some((_, via_key)) =
                model.resolve(&["AutomationSlice", "UiSlice"], via, self.ns())
            {
                issuers.retain(|(_, key, _)| *key == via_key);
            } else {
                self.push(
                    WalkFinding::new(
                        REF_UNRESOLVED,
                        format!("via {via:?} is not an automation or UI slice in the walked model"),
                    )
                    .field("via"),
                );
                issuers.clear();
            }
        }

        let answer = &step.answer;
        let mut offers: BTreeMap<String, Vec<Offer>> = BTreeMap::new();
        let mut ambiguous: BTreeMap<String, Key> = BTreeMap::new();
        let mut observed: Vec<Key> = Vec::new();
        for (issuer_kind, issuer, rms) in &issuers {
            self.touched.slices.insert(issuer.clone());
            for rm in rms {
                if !observed.contains(rm) {
                    observed.push(rm.clone());
                }
                let key_fields = rm_key_fields(model.get("ReadModel", rm));
                let rows: Vec<&Map<String, Value>> = self
                    .rows
                    .get(rm)
                    .map(|t| {
                        t.values()
                            .filter(|row| {
                                key_fields
                                    .iter()
                                    .all(|k| match (answer.get(k), row.get(k)) {
                                        (Some(a), Some(r)) => values_equal(a, r),
                                        _ => true,
                                    })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if rows.is_empty() {
                    continue;
                }
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
                            ambiguous.insert(field.clone(), rm.clone());
                        }
                    }
                }
            }
            if *issuer_kind == "AutomationSlice" {
                self.mark_automation_scenarios(issuer, command);
            }
        }

        let Some(fields) = model.get("Command", command).and_then(schema_fields) else {
            return answer.clone();
        };
        let mut values = Map::new();
        for field in fields {
            let name = &field.name;
            let mut sources = offers.remove(name).unwrap_or_default();
            let from_answer = answer.get(name);
            if let Some(a) = from_answer {
                sources.push(Offer {
                    source: "the step's answer".into(),
                    value: a.clone(),
                });
            }
            if let Some(first) = sources.first() {
                let conflict: Vec<&Offer> = sources
                    .iter()
                    .filter(|o| !values_equal(&o.value, &first.value))
                    .collect();
                if !conflict.is_empty() {
                    let all: Vec<String> = sources
                        .iter()
                        .map(|o| format!("{} from {}", o.value, o.source))
                        .collect();
                    self.push(
                        WalkFinding::new(
                            COMMAND_FIELD_CONFLICT,
                            format!("{command} field {name:?} gets {}", all.join(" but ")),
                        )
                        .schema_field("Command", command, name),
                    );
                }
                // The answer is the last word: it is what the issuer sent.
                let chosen = from_answer.cloned().unwrap_or_else(|| first.value.clone());
                values.insert(name.clone(), chosen);
                continue;
            }
            if let Some(rm) = ambiguous.get(name) {
                self.push(
                    WalkFinding::new(
                        COMMAND_FIELD_AMBIGUOUS,
                        format!(
                            "{command} field {name:?}: several rows of {rm} match and disagree; put {rm}'s key in the step's answer to pick one"
                        ),
                    )
                    .schema_field("Command", command, name),
                );
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
                            .get("ReadModel", rm)
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
                .schema_field("Command", command, name),
            );
        }
        values
    }

    fn mark_automation_scenarios(&mut self, auto_key: &Key, command: &Key) {
        let Some(auto) = self.model.get("AutomationSlice", auto_key) else {
            return;
        };
        for scenario in arr(auto, "scenarios") {
            let issues = edge_at(scenario, "then", "command").is_some_and(|c| c == *command);
            let given_hold = arr(scenario, "given").iter().all(|g| {
                edge_key(g, "readModel")
                    .and_then(|rm| self.rows.get(&rm))
                    .is_some_and(|t| !t.is_empty())
            });
            if issues && given_hold {
                self.mark(auto_key, str_at(scenario, "id"));
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
        slice: &Value,
        values: &Map<String, Value>,
        stream: &str,
    ) -> Option<Vec<Key>> {
        let history: Vec<Key> = self
            .history(stream)
            .iter()
            .map(|r| r.event.clone())
            .collect();
        let related: Vec<&Key> = self
            .log
            .iter()
            .filter(|r| correlates(&r.payload, values))
            .map(|r| &r.event)
            .collect();
        let mut rejects: Vec<(String, String)> = Vec::new();
        let mut emits: Vec<(String, Vec<Key>)> = Vec::new();
        for scenario in arr(slice, "scenarios") {
            let when = edge_at(scenario, "when", "command");
            if when.as_ref() != Some(command) {
                continue;
            }
            let given: Vec<Key> = edge_keys(scenario, "given", "event");
            let tail_ok = history.last().is_none_or(|last| given.contains(last));
            if !tail_ok || !is_subsequence(&given, related.iter().copied()) {
                continue;
            }
            let id = str_at(scenario, "id").to_string();
            if let Some(reject) = scenario.get("reject") {
                rejects.push((id, str_at(reject, "reasonCode").to_string()));
            } else if let Some(emit) = scenario.get("emit") {
                let events = edge_keys(emit, "events", "event");
                emits.push((id, events));
            }
        }
        let declared: Vec<Key> = edge_keys(slice, "emittedEvents", "event");
        let history_label = if history.is_empty() {
            "an empty stream".to_string()
        } else {
            let names: Vec<String> = history.iter().map(|k| k.slug.clone()).collect();
            format!("stream history [{}]", names.join(", "))
        };

        if let Some(reason) = &step.expect.reject {
            if rejects.is_empty() {
                self.push(
                    WalkFinding::new(
                        REJECT_UNMODELED,
                        format!(
                            "the case expects {command} to be refused after {history_label}, but no reject scenario on {slice_key} matches, so the model accepts it"
                        ),
                    )
                    .subject("CommandSlice", slice_key)
                    .field("scenarios"),
                );
                return None;
            }
            let matching = rejects
                .iter()
                .find(|(_, code)| reason.is_empty() || code == reason);
            if let Some((id, _)) = matching {
                self.mark(slice_key, id);
            } else {
                let (id, code) = &rejects[0];
                self.mark(slice_key, id);
                self.push(
                    WalkFinding::new(
                        REJECT_REASON_MISMATCH,
                        format!(
                            "scenario {id:?} of {slice_key} refuses {command} with {code:?}, the case expects {reason:?}"
                        ),
                    )
                    .subject("CommandSlice", slice_key)
                    .field(format!("scenarios[{id}].reject")),
                );
            }
            return None;
        }

        if let Some(expected) = &step.expect.emit {
            if let Some((id, code)) = rejects.first() {
                self.push(
                    WalkFinding::new(
                        UNEXPECTED_REJECT,
                        format!(
                            "the case expects {command} to emit events after {history_label}, but scenario {id:?} of {slice_key} refuses it ({code})"
                        ),
                    )
                    .subject("CommandSlice", slice_key)
                    .field(format!("scenarios[{id}].reject")),
                );
                return None;
            }
            let mut keys = Vec::new();
            for reference in expected {
                let key = self.model.event_key(reference, self.ns());
                if !declared.contains(&key) {
                    self.push(
                        WalkFinding::new(
                            EVENT_NOT_EMITTED_BY_SLICE,
                            format!("the case expects {key}, which {slice_key} does not emit"),
                        )
                        .subject("CommandSlice", slice_key)
                        .field("emitted_events"),
                    );
                }
                keys.push(key);
            }
            match emits.iter().find(|(_, events)| *events == keys) {
                Some((id, _)) => {
                    let id = id.clone();
                    self.mark(slice_key, &id);
                }
                None => self.push(
                    WalkFinding::new(
                        SCENARIO_MISSING,
                        format!(
                            "no scenario on {slice_key} has {command} emit [{}] after {history_label}",
                            keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                        ),
                    )
                    .subject("CommandSlice", slice_key)
                    .field("scenarios"),
                ),
            }
            return Some(keys);
        }

        // No stated outcome: the model decides.
        if let Some((id, _)) = rejects.first() {
            let id = id.clone();
            self.mark(slice_key, &id);
            return None;
        }
        if let Some((id, events)) = emits.first() {
            let (id, events) = (id.clone(), events.clone());
            self.mark(slice_key, &id);
            return Some(events);
        }
        if let [only] = declared.as_slice() {
            self.push(
                WalkFinding::new(
                    SCENARIO_MISSING,
                    format!(
                        "no scenario on {slice_key} covers {command} after {history_label}; walking its only emitted event {only}"
                    ),
                )
                .subject("CommandSlice", slice_key)
                .field("scenarios"),
            );
            return Some(vec![only.clone()]);
        }
        self.push(
            WalkFinding::new(
                OUTCOME_UNDETERMINED,
                format!(
                    "no scenario on {slice_key} covers {command} after {history_label}, and it emits {} events; state expect.emit or expect.reject",
                    declared.len()
                ),
            )
            .subject("CommandSlice", slice_key)
            .field("scenarios"),
        );
        None
    }

    fn mark(&mut self, slice: &Key, scenario: &str) {
        self.touched
            .scenarios
            .insert((slice.clone(), scenario.to_string()));
    }

    /// Check 3: give every field of an emitted event a value.
    fn event_payload(
        &mut self,
        event: &Key,
        command: &Key,
        values: &Map<String, Value>,
        answer: &Map<String, Value>,
        stream: &str,
    ) -> Map<String, Value> {
        let model = self.model;
        let Some(fields) = model.get("Event", event).and_then(schema_fields) else {
            return values.clone();
        };
        // A command field the walk could not supply was already reported
        // against the command; do not report it again on the event.
        let command_fields: Vec<String> = model
            .get("Command", command)
            .and_then(schema_fields)
            .map(|fs| fs.into_iter().map(|f| f.name).collect())
            .unwrap_or_default();
        let lane = self.lane_of("Event", event);
        let state: Vec<Field> = lane
            .as_ref()
            .and_then(|l| model.get("Swimlane", l))
            .map(|s| field_specs(arr(s, "state")))
            .unwrap_or_default();
        let earlier: Vec<Map<String, Value>> = self
            .history(stream)
            .into_iter()
            .map(|r| r.payload.clone())
            .collect();
        let mut payload = Map::new();
        for field in fields {
            let name = &field.name;
            let is_state = state.iter().any(|s| s.name == *name);
            if let Some(v) = values.get(name) {
                payload.insert(name.clone(), v.clone());
            } else if let Some(v) = answer.get(name).filter(|_| is_state) {
                // Aggregate state is set by the runtime; the step's answer
                // is where a case states what the runtime decided.
                payload.insert(name.clone(), v.clone());
            } else if let Some(v) = earlier.iter().rev().find_map(|p| p.get(name)) {
                payload.insert(name.clone(), v.clone());
            } else if is_state {
                payload.insert(name.clone(), Value::String(format!("<state:{name}>")));
            } else if let Some(kind) = &field.derived {
                payload.insert(name.clone(), derived_placeholder(kind));
            } else if !field.optional && !command_fields.contains(name) {
                self.push(
                    WalkFinding::new(
                        EVENT_FIELD_UNSOURCED,
                        format!(
                            "{event} needs {name:?}, but {command} does not carry it, no earlier event on the stream recorded it, and the swimlane state does not declare it"
                        ),
                    )
                    .schema_field("Event", event, name),
                );
            }
        }
        payload
    }

    /// Checks 1 and 5: the event must be a legal successor of the stream's
    /// previous lifecycle event.
    fn check_lifecycle(
        &mut self,
        event: &Key,
        payload: &Map<String, Value>,
        command: &Key,
        slice_key: &Key,
    ) {
        let model = self.model;
        let Some(lane) = self.lane_of("Event", event) else {
            return;
        };
        let Some(lane_spec) = model.get("Swimlane", &lane) else {
            return;
        };
        let transitions: Vec<(Key, Vec<Key>)> = arr(lane_spec, "transitions")
            .iter()
            .filter_map(|t| {
                let after = t.get("after").and_then(ref_key)?;
                let next = arr(t, "next")
                    .iter()
                    .filter_map(|n| edge_key(n, "event"))
                    .collect();
                Some((after, next))
            })
            .collect();
        if transitions.is_empty() {
            return;
        }
        let in_lifecycle = |k: &Key| transitions.iter().any(|(a, n)| a == k || n.contains(k));
        if !in_lifecycle(event) {
            return;
        }
        let stream = self.stream_of(Some(&lane), payload);
        let previous = self
            .history(&stream)
            .into_iter()
            .rev()
            .map(|r| r.event.clone())
            .find(|k| in_lifecycle(k));
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
                        .subject("Swimlane", &lane)
                        .field("transitions"),
                    );
                }
            }
            Some(prev) => match transitions.iter().find(|(a, _)| *a == prev) {
                None => self.push(
                    WalkFinding::new(
                        ACCEPTED_AFTER_TERMINAL,
                        format!(
                            "{command} emitted {event} after terminal event {prev} on {stream}; no reject scenario on {slice_key} refuses it"
                        ),
                    )
                    .subject("CommandSlice", slice_key)
                    .field("scenarios"),
                ),
                Some((_, next)) if !next.contains(event) => {
                    let legal: Vec<String> = next.iter().map(ToString::to_string).collect();
                    self.push(
                        WalkFinding::new(
                            LIFECYCLE_VIOLATION,
                            format!(
                                "{event} cannot follow {prev} on {stream}; the lifecycle allows [{}]",
                                legal.join(", ")
                            ),
                        )
                        .subject("Swimlane", &lane)
                        .field("transitions"),
                    );
                }
                Some(_) => {}
            },
        }
    }
}

/// Stand-in value for a field the runtime generates.
fn derived_placeholder(kind: &str) -> Value {
    Value::String(format!("<derived:{kind}>"))
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
        parts.push(v.clone());
    }
    Some(Value::Array(parts).to_string())
}

/// An earlier event belongs to the same timeline as the command when they
/// share at least one field and agree on every shared field.
fn correlates(payload: &Map<String, Value>, values: &Map<String, Value>) -> bool {
    let mut shared = 0;
    for (k, v) in values {
        if let Some(p) = payload.get(k) {
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
    needle.iter().all(|n| it.any(|h| h == n))
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn row_matches(expected: &Map<String, Value>, actual: &Map<String, Value>) -> bool {
    expected
        .iter()
        .all(|(k, v)| actual.get(k).map_or(v.is_null(), |a| values_equal(v, a)))
}

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
    let mut used = vec![false; actual.len()];
    for row in expected {
        let found = actual
            .iter()
            .enumerate()
            .find(|(i, a)| !used[*i] && row_matches(row, a));
        match found {
            Some((i, _)) => used[i] = true,
            None => {
                return Some(format!(
                    "no row matches {}; the walk left {}",
                    Value::Object(row.clone()),
                    render(actual)
                ))
            }
        }
    }
    None
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

    for (key, board) in model.live("Storyboard") {
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
                .subject("Storyboard", key),
            );
        }
    }

    for kind in ["CommandSlice", "ReadModelSlice", "AutomationSlice"] {
        for (key, slice) in model.live(kind) {
            if !in_scope(key) {
                continue;
            }
            for scenario in arr(slice, "scenarios") {
                let id = str_at(scenario, "id").to_string();
                cov.scenarios_total += 1;
                if touched.scenarios.contains(&(key.clone(), id.clone())) {
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
