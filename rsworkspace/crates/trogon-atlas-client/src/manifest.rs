// Manifest loading: YAML CRD-style documents → typed `pb::Entity`.
//
// A document looks like:
//
//   apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
//   kind: CommandSlice
//   metadata:
//     namespace: shop
//     name: place-order-slice
//     version: 1            # optional, defaults to 1
//   spec:
//     title: Place order
//     command: place-order            # ref shorthand
//     emittedEvents: [order.placed]   # edge shorthand
//
// `spec` is the proto3-JSON body of the entity message named by `kind`
// (minus `id`, which comes from metadata). Before transcoding, the spec is
// normalized AGAINST THE PROTO SCHEMA so authors get ergonomics without a
// hand-maintained parallel schema:
//
//   * wherever the proto expects an Id, a Ref, an Edge, or an Example
//     wrapper, a bare string "slug", "ns/slug", or "ns/slug@version" is
//     expanded to the full object;
//   * Id objects missing `namespace` inherit the manifest's namespace and
//     missing `version` defaults to 1;
//   * EntityKind enum values accept the short form ("storyboard") in
//     addition to the proto name ("ENTITY_KIND_STORYBOARD").
//
// Schema-awareness is load-bearing: scenario messages carry a plain-string
// `id` field of their own, so a purely structural "expand things under an
// id key" walk would corrupt them.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use prost_reflect::{Kind as FieldKind, MessageDescriptor};
use serde_json::{Map, Value};
use trogon_atlas_core::transcode::TranscodePool;
// The transcode primitive lives in trogon-atlas-core: one descriptor pool,
// one JSON implementation, shared by every surface. Re-exported here so
// callers of this crate keep working.
pub use trogon_atlas_core::transcode::{entity_descriptor, entity_json_value, entity_to_json};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    failure::invalid_input,
    tenant_types::{self, TypeScope},
};

pub const API_VERSION: &str = "eventmodel.atlas.trogonstack.com/v1alpha1";

pub(crate) const ID_MESSAGE: &str = "trogonatlas.type.v1alpha1.Id";
const ENTITY_MESSAGE: &str = "trogonatlas.eventmodel.v1alpha1.Entity";
pub(crate) const ENTITY_KIND_ENUM: &str = "trogonatlas.eventmodel.v1alpha1.EntityKind";

/// One parsed manifest document, ready to apply.
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    /// Where the document came from, for error messages: `path#docN`.
    pub source: String,
    pub kind: pb::EntityKind,
    pub id: pb::Id,
    pub entity: pb::Entity,
}

/// One manifest document read and normalized, whose spec has not yet been
/// transcoded: a spec may name tenant types that only the server, or a
/// type library elsewhere in the same set, can describe.
#[derive(Debug, Clone)]
pub struct ManifestDocument {
    pub source: String,
    pub kind: pb::EntityKind,
    pub id: pb::Id,
    entity_json: Value,
}

impl ManifestDocument {
    /// Transcode the spec with `types`, which must know every message its
    /// `Any` values name.
    pub fn load(&self, types: &TranscodePool) -> Result<LoadedManifest> {
        let entity: pb::Entity = types
            .message_from_json_value(ENTITY_MESSAGE, &self.entity_json)
            .map_err(|e| invalid_input(format!("{}: parsing manifest: {e:#}", self.source)))?;
        Ok(LoadedManifest {
            source: self.source.clone(),
            kind: self.kind,
            id: self.id.clone(),
            entity,
        })
    }

    #[must_use]
    pub fn is_type_library(&self) -> bool {
        self.kind == pb::EntityKind::TypeLibrary
    }
}

/// Manifests transcoded with the tenant types they reach, and the pool that
/// did it, so diffs and exports render the same `Any` values back.
#[derive(Debug, Clone)]
pub struct ResolvedManifests {
    pub manifests: Vec<LoadedManifest>,
    pub types: TranscodePool,
}

/// Transcode `documents` with the live type libraries of every namespace
/// they touch, plus the libraries they define themselves. Type libraries
/// come first so one apply can introduce a type and its first users.
pub async fn load_with_tenant_types(
    client: &mut Client,
    mut documents: Vec<ManifestDocument>,
) -> Result<ResolvedManifests> {
    let mut scope = TypeScope::default();
    for document in &documents {
        scope.add_namespace(document.id.namespace.clone());
        scope.merge(TypeScope::of_json(&document.entity_json));
    }
    let types = tenant_types::transcode_pool(client, &scope).await?;
    documents.sort_by_key(|document| !document.is_type_library());
    let manifests = documents
        .iter()
        .map(|document| document.load(&types))
        .collect::<Result<_>>()?;
    Ok(ResolvedManifests { manifests, types })
}

/// Collect manifest files from files/directories into a sorted, deduped
/// list, so callers (apply, fmt) see a stable order regardless of
/// filesystem iteration order. Directories are walked recursively for
/// `.yaml`/`.yml`.
pub(crate) fn collect_yaml_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();
    for path in paths {
        collect_files(path, &mut files, true)?;
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        crate::invalid_input!("no .yaml/.yml manifest files found under the given paths");
    }
    Ok(files)
}

/// Collect manifest files from files/directories and parse every document
/// in each one.
pub fn load_paths(paths: &[PathBuf]) -> Result<Vec<LoadedManifest>> {
    load_builtin(&read_paths(paths)?)
}

/// Collect manifest files from files/directories and read every document
/// in each one, leaving their specs for `load_with_tenant_types`.
pub fn read_paths(paths: &[PathBuf]) -> Result<Vec<ManifestDocument>> {
    let files = collect_yaml_files(paths)?;
    let mut documents = Vec::new();
    for file in &files {
        documents.extend(read_file(file)?);
    }
    Ok(documents)
}

fn load_builtin(documents: &[ManifestDocument]) -> Result<Vec<LoadedManifest>> {
    let types = TranscodePool::builtin()?;
    documents
        .iter()
        .map(|document| document.load(&types))
        .collect()
}

/// Walk `path` into `out`. When `explicit` is true (a path the user named
/// with `-f`), non-yaml files are errors; when walking a directory they are
/// skipped, matching kubectl's directory walk.
fn collect_files(path: &Path, out: &mut Vec<PathBuf>, explicit: bool) -> Result<()> {
    let meta = std::fs::metadata(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            invalid_input(format!("reading {}: {err}", path.display()))
        } else {
            anyhow::Error::new(err).context(format!("reading {}", path.display()))
        }
    })?;
    if meta.is_dir() {
        for entry in
            std::fs::read_dir(path).with_context(|| format!("listing {}", path.display()))?
        {
            collect_files(&entry?.path(), out, false)?;
        }
        return Ok(());
    }
    let is_yaml = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"));
    if is_yaml {
        out.push(path.to_path_buf());
    } else if explicit {
        crate::invalid_input!(
            "{}: not a .yaml/.yml manifest file (unsupported)",
            path.display()
        );
    }
    Ok(())
}

/// Parse every YAML document in one file.
pub fn parse_file(path: &Path) -> Result<Vec<LoadedManifest>> {
    load_builtin(&read_file(path)?)
}

fn read_file(path: &Path) -> Result<Vec<ManifestDocument>> {
    let text = std::fs::read_to_string(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            invalid_input(format!("reading {}: {err}", path.display()))
        } else {
            anyhow::Error::new(err).context(format!("reading {}", path.display()))
        }
    })?;
    read_str(&text, &path.display().to_string())
}

/// Parse every YAML document in a string. `source` labels errors.
///
/// Documents deserialize straight into `serde_json::Value` (YAML is a JSON
/// superset for our purposes; non-string keys are rejected by serde_json).
pub fn parse_str(text: &str, source: &str) -> Result<Vec<LoadedManifest>> {
    load_builtin(&read_str(text, source)?)
}

/// Read every YAML document in a string without transcoding its spec.
pub fn read_str(text: &str, source: &str) -> Result<Vec<ManifestDocument>> {
    use serde::Deserialize as _;
    let mut out = Vec::new();
    for (i, doc) in serde_yaml::Deserializer::from_str(text).enumerate() {
        let value = Value::deserialize(doc)
            .map_err(|e| invalid_input(format!("{source}#doc{i}: invalid YAML: {e}")))?;
        if value.is_null() {
            continue; // empty document (e.g. trailing `---`)
        }
        let label = format!("{source}#doc{i}");
        out.push(parse_document(value, &label).with_context(|| label.clone())?);
    }
    if out.is_empty() {
        crate::invalid_input!("{source}: no manifest documents found");
    }
    Ok(out)
}

fn parse_document(value: Value, source: &str) -> Result<ManifestDocument> {
    let Value::Object(mut root) = value else {
        crate::invalid_input!("manifest document must be a mapping");
    };
    for key in root.keys() {
        if !matches!(key.as_str(), "apiVersion" | "kind" | "metadata" | "spec") {
            crate::invalid_input!(
                "unknown top-level key {key:?} (expected apiVersion, kind, metadata, spec)"
            );
        }
    }

    let api_version = str_field(&root, "apiVersion")?;
    if api_version != API_VERSION {
        crate::invalid_input!("unsupported apiVersion {api_version:?} (expected {API_VERSION:?})");
    }

    let kind_str = str_field(&root, "kind")?;
    let kind = pb::canonical::parse_kind(&kind_str)
        .ok_or_else(|| invalid_input(format!("unknown kind {kind_str:?}")))?;

    let Some(Value::Object(metadata)) = root.remove("metadata") else {
        crate::invalid_input!("metadata must be a mapping");
    };
    for key in metadata.keys() {
        if !matches!(key.as_str(), "namespace" | "name" | "version") {
            crate::invalid_input!(
                "unknown metadata key {key:?} (expected namespace, name, version)"
            );
        }
    }
    let namespace = metadata
        .get("namespace")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid_input("metadata.namespace is required"))?
        .to_string();
    let name = metadata
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid_input("metadata.name is required"))?
        .to_string();
    let version = match metadata.get("version") {
        None => 1,
        Some(v) => v
            .as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .filter(|v| *v >= 1)
            .ok_or_else(|| invalid_input("metadata.version must be a positive integer"))?,
    };

    let spec = match root.remove("spec") {
        None => Map::new(),
        Some(Value::Object(m)) => m,
        Some(_) => crate::invalid_input!("spec must be a mapping"),
    };
    if spec.contains_key("id") {
        crate::invalid_input!("spec must not contain `id`: identity comes from metadata");
    }
    if spec.contains_key("system") {
        crate::invalid_input!("spec must not contain `system`: it is server-owned");
    }

    let entity_desc = entity_descriptor().context("Entity descriptor not found")?;
    let body_field = entity_desc
        .fields()
        .find(|f| f.name() == pb::canonical::kind_short(kind))
        .ok_or_else(|| anyhow!("Entity has no field for kind {kind_str:?}"))?;
    let FieldKind::Message(body_desc) = body_field.kind() else {
        anyhow::bail!("Entity field for {kind_str:?} is not a message");
    };

    let mut spec_value = Value::Object(spec);
    normalize_message(&mut spec_value, &body_desc, &namespace)?;
    let Value::Object(spec) = &mut spec_value else {
        unreachable!("spec stays an object through normalization");
    };
    spec.insert(
        "id".to_string(),
        serde_json::json!({
            "namespace": namespace,
            "slug": name,
            "version": version.to_string(),
        }),
    );

    let mut entity_json = Map::new();
    entity_json.insert(body_field.json_name().to_string(), spec_value);

    Ok(ManifestDocument {
        source: source.to_string(),
        kind,
        id: pb::Id {
            namespace,
            slug: name,
            version,
        },
        entity_json: Value::Object(entity_json),
    })
}

fn str_field(map: &Map<String, Value>, key: &str) -> Result<String> {
    map.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| invalid_input(format!("{key} is required and must be a non-empty string")))
}

// ---------------------------------------------------------------------------
// Schema-aware normalization
// ---------------------------------------------------------------------------

/// True when `md` is a Ref-style wrapper: a single `id` field of type Id
/// (EventRef, CommandRef, ..., and EntityRef which adds a `kind` field).
pub(crate) fn is_ref_like(md: &MessageDescriptor) -> bool {
    md.get_field_by_name("id").is_some_and(|f| match f.kind() {
        FieldKind::Message(inner) => inner.full_name() == ID_MESSAGE,
        _ => false,
    })
}

/// The field to route a bare-string shorthand into, for Edge/Example
/// wrappers: the first field whose type is a Ref-like message.
pub(crate) fn shorthand_ref_field(
    md: &MessageDescriptor,
) -> Option<prost_reflect::FieldDescriptor> {
    md.fields().find(|f| match f.kind() {
        FieldKind::Message(inner) => is_ref_like(&inner),
        _ => false,
    })
}

/// Expand "slug", "ns/slug", or "ns/slug@version" into an Id object.
/// Version must be a positive integer (same rule as `metadata.version`).
/// Empty slug is invalid (proto identity contract).
fn expand_id_string(s: &str, default_ns: &str) -> Result<Value> {
    let (rest, version) = match s.rsplit_once('@') {
        Some((rest, v)) if v.chars().all(|c| c.is_ascii_digit()) && !v.is_empty() => (rest, v),
        _ => (s, "1"),
    };
    let version_num: u64 = version.parse().map_err(|_| {
        invalid_input(format!(
            "id version must be a positive integer, got {version:?}"
        ))
    })?;
    if version_num < 1 {
        crate::invalid_input!("id version must be a positive integer, got {version_num}");
    }
    let (namespace, slug) = match rest.split_once('/') {
        Some((ns, slug)) if !ns.is_empty() => (ns, slug),
        _ => (default_ns, rest),
    };
    if slug.is_empty() {
        crate::invalid_input!("id slug is required and must be a non-empty string");
    }
    Ok(serde_json::json!({
        "namespace": namespace,
        "slug": slug,
        "version": version_num.to_string(),
    }))
}

fn parse_positive_version(value: &Value) -> Result<u64> {
    let n = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| invalid_input("id version must be a positive integer"))?;
    if n < 1 {
        crate::invalid_input!("id version must be a positive integer, got {n}");
    }
    Ok(n)
}

fn normalize_id(value: &mut Value, default_ns: &str) -> Result<()> {
    if let Value::String(s) = value {
        *value = expand_id_string(s, default_ns)?;
        return Ok(());
    }
    if let Value::Object(map) = value {
        for key in map.keys() {
            if !matches!(key.as_str(), "namespace" | "slug" | "version") {
                crate::invalid_input!("unknown id key {key:?} (expected namespace, slug, version)");
            }
        }
        let ns_present = map
            .get("namespace")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        if !ns_present {
            map.insert(
                "namespace".to_string(),
                Value::String(default_ns.to_string()),
            );
        }
        if map
            .get("slug")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            crate::invalid_input!("id slug is required and must be a non-empty string");
        }
        match map.get("version") {
            None => {
                map.insert("version".to_string(), Value::String("1".to_string()));
            }
            Some(v) => {
                let n = parse_positive_version(v)?;
                map.insert("version".to_string(), Value::String(n.to_string()));
            }
        }
    }
    Ok(())
}

/// Recursively normalize a JSON value expected to be message `desc`.
fn normalize_message(value: &mut Value, desc: &MessageDescriptor, default_ns: &str) -> Result<()> {
    if desc.full_name() == ID_MESSAGE {
        return normalize_id(value, default_ns);
    }
    // Any-packed annotations are opaque to the schema walk.
    if desc.full_name().starts_with("google.protobuf.") {
        return Ok(());
    }
    if value.is_string() {
        if is_ref_like(desc) {
            let s = std::mem::take(value);
            *value = serde_json::json!({ "id": s });
        } else if let Some(ref_field) = shorthand_ref_field(desc) {
            let s = std::mem::take(value);
            let mut map = Map::new();
            map.insert(ref_field.json_name().to_string(), s);
            *value = Value::Object(map);
        } else {
            return Ok(());
        }
    }
    let Value::Object(map) = value else {
        return Ok(());
    };
    for (key, val) in map.iter_mut() {
        let Some(field) = desc
            .fields()
            .find(|f| f.json_name() == key || f.name() == key)
        else {
            continue; // unknown key: the transcoder rejects it with context
        };
        if field.is_map() {
            continue; // no map fields carry Ids in this schema
        }
        if field.is_list() {
            if let Value::Array(items) = val {
                for item in items {
                    normalize_field_value(item, &field, default_ns)?;
                }
            }
            continue;
        }
        normalize_field_value(val, &field, default_ns)?;
    }
    Ok(())
}

fn normalize_field_value(
    value: &mut Value,
    field: &prost_reflect::FieldDescriptor,
    default_ns: &str,
) -> Result<()> {
    match field.kind() {
        FieldKind::Message(md) => normalize_message(value, &md, default_ns),
        FieldKind::Enum(ed) if ed.full_name() == ENTITY_KIND_ENUM => {
            if let Value::String(s) = value {
                if !s.starts_with("ENTITY_KIND_") {
                    if let Some(kind) = pb::canonical::parse_kind(s) {
                        if let Some(variant) = ed.get_value(kind as i32) {
                            *value = Value::String(variant.name().to_string());
                        }
                    }
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use serde::Deserialize as _;

    use super::*;

    fn one(yaml: &str) -> LoadedManifest {
        let mut all = parse_str(yaml, "test.yaml").expect("parse");
        assert_eq!(all.len(), 1);
        all.remove(0)
    }

    #[test]
    fn parses_command_slice_with_shorthand_refs() {
        let m = one(r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata:
  namespace: shop
  name: place-order-slice
spec:
  title: Place order
  ui: checkout
  command: place-order
  emittedEvents:
    - order.placed
    - upstream/payment.requested@2
");
        assert_eq!(m.kind, pb::EntityKind::CommandSlice);
        assert_eq!(m.id.namespace, "shop");
        assert_eq!(m.id.slug, "place-order-slice");
        assert_eq!(m.id.version, 1);
        let Some(pb::entity::Kind::CommandSlice(slice)) = m.entity.kind else {
            panic!("expected commandSlice");
        };
        assert_eq!(slice.title, "Place order");
        let cmd = slice.command.unwrap().command.unwrap().id.unwrap();
        assert_eq!(
            (cmd.namespace.as_str(), cmd.slug.as_str(), cmd.version),
            ("shop", "place-order", 1)
        );
        let ui = slice.ui.unwrap().ui.unwrap().id.unwrap();
        assert_eq!(ui.slug, "checkout");
        let events: Vec<pb::Id> = slice
            .emitted_events
            .into_iter()
            .map(|e| e.event.unwrap().id.unwrap())
            .collect();
        assert_eq!(
            (
                events[0].namespace.as_str(),
                events[0].slug.as_str(),
                events[0].version
            ),
            ("shop", "order.placed", 1)
        );
        assert_eq!(
            (
                events[1].namespace.as_str(),
                events[1].slug.as_str(),
                events[1].version
            ),
            ("upstream", "payment.requested", 2)
        );
    }

    #[test]
    fn scenario_string_ids_survive_normalization() {
        let m = one(r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: AutomationSlice
metadata:
  namespace: shop
  name: fulfill-auto
spec:
  title: Auto fulfill
  sourceReadModels: [order-queue]
  processor: fulfiller
  emittedCommand: fulfill
  scenarios:
    - id: happy
      title: Row appears
      given: [order-queue]
      whenDoc: poll finds a row
      then: fulfill
");
        let Some(pb::entity::Kind::AutomationSlice(slice)) = m.entity.kind else {
            panic!("expected automationSlice");
        };
        assert_eq!(slice.scenarios.len(), 1);
        // Scenario id stays the literal string, NOT expanded into an Id.
        assert_eq!(slice.scenarios[0].id, "happy");
        let given = slice.scenarios[0].given[0]
            .read_model
            .clone()
            .unwrap()
            .id
            .unwrap();
        assert_eq!(given.slug, "order-queue");
        assert_eq!(given.namespace, "shop");
        let then = slice.scenarios[0]
            .then
            .clone()
            .unwrap()
            .command
            .unwrap()
            .id
            .unwrap();
        assert_eq!(then.slug, "fulfill");
        let rms: Vec<pb::Id> = slice
            .source_read_models
            .into_iter()
            .map(|e| e.read_model.unwrap().id.unwrap())
            .collect();
        assert_eq!(rms[0].slug, "order-queue");
    }

    #[test]
    fn event_model_members_accept_kind_aliases() {
        let m = one(r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata:
  namespace: shop
  name: shop-model
spec:
  title: Shop
  members:
    - kind: storyboard
      id: buy-flow
    - kind: read-model
      id: order-status
");
        let Some(pb::entity::Kind::EventModel(em)) = m.entity.kind else {
            panic!("expected eventModel");
        };
        assert_eq!(em.members[0].kind, pb::EntityKind::Storyboard as i32);
        assert_eq!(em.members[0].id.clone().unwrap().slug, "buy-flow");
        assert_eq!(em.members[1].kind, pb::EntityKind::ReadModel as i32);
    }

    #[test]
    fn metadata_version_defaults_and_parses() {
        let m = one(r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: shop, name: order.placed, version: 3 }
spec: { title: Order placed }
");
        assert_eq!(m.id.version, 3);
        let Some(pb::entity::Kind::Event(ev)) = m.entity.kind else {
            panic!("expected event");
        };
        assert_eq!(ev.id.unwrap().version, 3);
    }

    #[test]
    fn rejects_unknown_kind_and_top_level_keys() {
        let err = parse_str(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Widget\nmetadata: {namespace: a, name: b}\n",
            "t",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("unknown kind"), "{err:#}");

        let err = parse_str(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {namespace: a, name: b}\nstatus: {}\n",
            "t",
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown top-level key"),
            "{err:#}"
        );
    }

    #[test]
    fn rejects_wrong_api_version_and_spec_id() {
        let err = parse_str(
            "apiVersion: v1\nkind: Event\nmetadata: {namespace: a, name: b}\n",
            "t",
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("unsupported apiVersion"),
            "{err:#}"
        );

        let err = parse_str(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {namespace: a, name: b}\nspec: {id: {slug: x}}\n",
            "t",
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("must not contain `id`"),
            "{err:#}"
        );
    }

    #[test]
    fn rejects_unknown_spec_fields_via_transcoder() {
        let err = parse_str(
            "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1\nkind: Event\nmetadata: {namespace: a, name: b}\nspec: {titel: typo}\n",
            "t",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("titel"), "{err:#}");
    }

    #[test]
    fn multi_document_files_parse_in_order() {
        let all = parse_str(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: shop, name: a }
spec: { title: A }
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Command
metadata: { namespace: shop, name: b }
spec: { title: B }
",
            "t",
        )
        .expect("parse");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].kind, pb::EntityKind::Event);
        assert_eq!(all[1].kind, pb::EntityKind::Command);
    }

    // Compile-time-ish guard that the FromYamlDoc trait stays wired.
    #[test]
    fn yaml_scalar_docs_are_skipped_gracefully() {
        let err = parse_str("---\n", "t").unwrap_err();
        assert!(
            format!("{err:#}").contains("no manifest documents"),
            "{err:#}"
        );
        let _ = serde_json::Value::deserialize(serde_yaml::Deserializer::from_str("x: 1"))
            .expect("yaml to json");
    }

    // Regression: metadata.version rejects <1, but Id shorthand `@0` and
    // nested Id objects with version 0 were accepted, producing refs the
    // rest of the toolchain treats as invalid / non-positive.
    #[test]
    fn rejects_zero_version_in_id_shorthand() {
        let err = parse_str(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: shop, name: place-order-slice }
spec:
  title: Place order
  command: place-order@0
",
            "t",
        )
        .expect_err("version 0 in @version shorthand must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("version") && (msg.contains("positive") || msg.contains('0')),
            "expected a version-rejection error, got: {msg}"
        );
    }

    #[test]
    fn rejects_zero_version_in_nested_id_object() {
        let err = parse_str(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: shop, name: shop-model }
spec:
  title: Shop
  members:
    - kind: storyboard
      id: { slug: buy-flow, version: 0 }
",
            "t",
        )
        .expect_err("version 0 in nested Id object must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("version") && (msg.contains("positive") || msg.contains('0')),
            "expected a version-rejection error, got: {msg}"
        );
    }

    // Proto identity contract: empty slug is invalid. Shorthand refs like
    // "shop/" or nested `{ slug: "" }` must fail at parse time, not only when
    // the server later rejects the put.
    #[test]
    fn rejects_empty_slug_in_id_shorthand() {
        let err = parse_str(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: shop, name: place-order-slice }
spec:
  title: Place order
  command: shop/
",
            "t",
        )
        .expect_err("empty slug in ns/slug shorthand must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("slug") && (msg.contains("empty") || msg.contains("required")),
            "expected empty-slug rejection, got: {msg}"
        );
    }

    #[test]
    fn rejects_empty_slug_in_nested_id_object() {
        let err = parse_str(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: shop, name: shop-model }
spec:
  title: Shop
  members:
    - kind: storyboard
      id: { slug: '' }
",
            "t",
        )
        .expect_err("empty slug in nested Id object must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("slug") && (msg.contains("empty") || msg.contains("required")),
            "expected empty-slug rejection, got: {msg}"
        );
    }

    #[test]
    fn accepts_entity_kind_prefix_as_manifest_kind() {
        let m = one(r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: ENTITY_KIND_EVENT
metadata: { namespace: shop, name: order.placed }
spec: { title: Order placed }
");
        assert_eq!(m.kind, pb::EntityKind::Event);
    }

    #[test]
    fn rejects_unknown_keys_in_id_object() {
        // EntityRef.id is a real Id message; typo must not be silently dropped.
        let err = parse_str(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: EventModel
metadata: { namespace: shop, name: shop-model }
spec:
  title: Shop
  members:
    - kind: storyboard
      id: { namespace: shop, slug: buy-flow, version: 1, typo: oops }
",
            "t",
        )
        .expect_err("unknown Id field must be rejected");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("typo") || msg.contains("unknown"),
            "expected unknown-field rejection mentioning typo, got: {msg}"
        );
    }

    #[test]
    fn load_paths_rejects_explicit_non_yaml_file() {
        let dir = std::env::temp_dir().join(format!(
            "trogon-atlas-manifest-nonyaml-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let yaml_path = dir.join("ok.yaml");
        let txt_path = dir.join("override.json");
        std::fs::write(
            &yaml_path,
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: shop, name: a }
spec: { title: A }
",
        )
        .unwrap();
        std::fs::write(&txt_path, r#"{"not":"a manifest"}"#).unwrap();

        // Explicitly naming a non-yaml file must not silently drop it while
        // still applying the yaml sibling (kubectl errors on -f of a bad type).
        let err = load_paths(&[yaml_path, txt_path])
            .expect_err("explicit non-yaml path must be rejected, not silently skipped");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("override.json")
                || msg.contains("not a")
                || msg.contains(".yaml")
                || msg.contains("unsupported"),
            "error should name the rejected file or reason, got: {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
