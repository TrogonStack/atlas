// Manifest JSON Schema (draft 2020-12), derived from the descriptor pool and
// `manifest.rs`'s own shorthand rules so it accepts exactly what the parser
// accepts. `serde_json::Map` is a BTreeMap here, so output keys are sorted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::{Context, Result};
use prost_reflect::{EnumDescriptor, FieldDescriptor, Kind as FieldKind, MessageDescriptor};
use serde_json::{json, Map, Value};
use trogon_atlas_proto as pb;

use crate::manifest::{is_ref_like, shorthand_ref_field, ENTITY_KIND_ENUM, ID_MESSAGE};

/// Synthetic `$defs` key for the manifest envelope's `metadata` object.
/// Leading underscore keeps it unambiguous against proto full names, which
/// always contain a package-qualifying dot.
const METADATA_DEF: &str = "_Metadata";
/// Synthetic `$defs` key for the shorthand ref/id string
/// (`"slug"`, `"ns/slug"`, `"ns/slug@version"`).
const SHORTHAND_DEF: &str = "_ShorthandRef";

enum Pending {
    Message(MessageDescriptor),
    Enum(EnumDescriptor),
}

/// Generate the manifest JSON Schema document.
pub fn generate() -> Result<Value> {
    let entity_desc =
        trogon_atlas_core::transcode::entity_descriptor().context("Entity descriptor not found")?;

    let mut roots: BTreeSet<String> = BTreeSet::new();
    let mut kind_bodies: Vec<(pb::EntityKind, MessageDescriptor)> = Vec::new();
    for &kind in pb::canonical::ALL_KINDS {
        let field = entity_desc
            .fields()
            .find(|f| f.name() == pb::canonical::kind_short(kind))
            .with_context(|| format!("Entity has no field for kind {kind:?}"))?;
        let FieldKind::Message(body) = field.kind() else {
            anyhow::bail!("Entity field for {kind:?} is not a message");
        };
        roots.insert(body.full_name().to_string());
        kind_bodies.push((kind, body));
    }

    let mut queue: VecDeque<Pending> = VecDeque::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    seen.insert(ID_MESSAGE.to_string());
    seen.insert(ENTITY_KIND_ENUM.to_string());
    for (_, body) in &kind_bodies {
        enqueue_message(&mut queue, &mut seen, body.clone());
    }

    let mut defs: BTreeMap<String, Value> = BTreeMap::new();
    defs.insert(ID_MESSAGE.to_string(), id_schema());
    defs.insert(ENTITY_KIND_ENUM.to_string(), entity_kind_schema());
    defs.insert(SHORTHAND_DEF.to_string(), shorthand_schema());
    defs.insert(METADATA_DEF.to_string(), metadata_schema());

    while let Some(pending) = queue.pop_front() {
        match pending {
            Pending::Message(desc) => {
                let schema = message_schema(&desc, &roots, &mut queue, &mut seen);
                defs.insert(desc.full_name().to_string(), schema);
            }
            Pending::Enum(desc) => {
                defs.insert(desc.full_name().to_string(), enum_schema(&desc));
            }
        }
    }

    let mut all_of = Vec::new();
    for (kind, body) in &kind_bodies {
        let aliases: Vec<String> = kind_aliases(*kind).into_iter().collect();
        all_of.push(json!({
            "if": {
                "properties": { "kind": { "enum": aliases } },
                "required": ["kind"]
            },
            "then": {
                "properties": { "spec": def_ref(body.full_name()) }
            }
        }));
    }

    Ok(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "trogon-atlas manifest",
        "description": "CRD-style YAML document accepted by `trogon-atlas apply` / trogon-atlas-client manifest loading.",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "apiVersion": { "const": crate::manifest::API_VERSION },
            "kind": def_ref(ENTITY_KIND_ENUM),
            "metadata": def_ref(METADATA_DEF),
            "spec": { "type": "object" }
        },
        "required": ["apiVersion", "kind", "metadata"],
        "allOf": all_of,
        "$defs": defs,
    }))
}

fn def_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/$defs/{name}") })
}

fn enqueue_message(
    queue: &mut VecDeque<Pending>,
    seen: &mut BTreeSet<String>,
    desc: MessageDescriptor,
) {
    if desc.full_name().starts_with("google.protobuf.") {
        return; // well-known types are handled generically, never walked
    }
    if seen.insert(desc.full_name().to_string()) {
        queue.push_back(Pending::Message(desc));
    }
}

fn enqueue_enum(queue: &mut VecDeque<Pending>, seen: &mut BTreeSet<String>, desc: EnumDescriptor) {
    if seen.insert(desc.full_name().to_string()) {
        queue.push_back(Pending::Enum(desc));
    }
}

/// `{"type":"string","minLength":1}`: the shorthand grammar manifest.rs
/// expands (`expand_id_string`), kept intentionally unconstrained beyond
/// non-emptiness. The parser is the source of truth for the exact
/// namespace/slug/version split; pattern-matching it here would risk the
/// schema rejecting something the parser accepts.
fn shorthand_schema() -> Value {
    json!({ "type": "string", "minLength": 1 })
}

fn version_schema() -> Value {
    json!({
        "oneOf": [
            { "type": "integer", "minimum": 1 },
            { "type": "string", "pattern": "^[0-9]+$" }
        ]
    })
}

fn id_schema() -> Value {
    json!({
        "oneOf": [
            def_ref(SHORTHAND_DEF),
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "namespace": { "type": "string" },
                    "slug": { "type": "string", "minLength": 1 },
                    "version": version_schema()
                },
                "required": ["slug"]
            }
        ]
    })
}

fn metadata_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "namespace": { "type": "string", "minLength": 1 },
            "name": { "type": "string", "minLength": 1 },
            "version": version_schema()
        },
        "required": ["namespace", "name"]
    })
}

fn enum_schema(desc: &EnumDescriptor) -> Value {
    let mut values: Vec<String> = desc.values().map(|v| v.name().to_string()).collect();
    values.sort();
    json!({ "type": "string", "enum": values })
}

/// The literal spellings `pb::canonical::parse_kind` accepts for `kind`,
/// restricted to the forms a schema `enum` can express without relying on
/// case-insensitive matching (which JSON Schema does not support): the
/// canonical snake_case short form, its dash and fully-collapsed
/// (PascalCase) variants, and the proto wire name. `parse_kind` itself is
/// case-insensitive, so unusual casing the parser would still accept (e.g.
/// "EVENT" or "CoMmAnD") is not enumerated here; see the generator's test
/// module / final report for this as a deliberate scope limit.
fn kind_aliases(kind: pb::EntityKind) -> BTreeSet<String> {
    let short = pb::canonical::kind_short(kind);
    let mut set = BTreeSet::new();
    set.insert(short.to_string());
    set.insert(short.replace('_', "-"));
    set.insert(format!("ENTITY_KIND_{}", short.to_uppercase()));
    set.insert(crate::export::manifest_kind(kind));
    set
}

fn entity_kind_schema() -> Value {
    let mut all: BTreeSet<String> = BTreeSet::new();
    for &kind in pb::canonical::ALL_KINDS {
        all.extend(kind_aliases(kind));
    }
    json!({ "type": "string", "enum": all.into_iter().collect::<Vec<_>>() })
}

/// Well-known proto types never get a `$defs` entry (manifest.rs's
/// `normalize_message` treats anything under `google.protobuf.` as opaque).
/// `Any` is the only one reachable from a manifest body today; its proto3
/// JSON shape is `{"@type": "...", ...flattened target fields}`, so the
/// schema can validate the discriminator and otherwise stay permissive.
/// Any other well-known type that might reach a field in the future falls
/// back to an unconstrained schema rather than guessing its JSON mapping.
fn well_known_schema(full_name: &str) -> Value {
    if full_name == "google.protobuf.Any" {
        json!({
            "type": "object",
            "properties": { "@type": { "type": "string" } },
            "required": ["@type"]
        })
    } else {
        json!(true)
    }
}

fn scalar_schema(kind: &FieldKind) -> Value {
    match kind {
        FieldKind::Double | FieldKind::Float => json!({ "type": "number" }),
        FieldKind::Int32 | FieldKind::Sint32 | FieldKind::Sfixed32 => json!({ "type": "integer" }),
        FieldKind::Uint32 | FieldKind::Fixed32 => json!({ "type": "integer", "minimum": 0 }),
        FieldKind::Int64 | FieldKind::Sint64 | FieldKind::Sfixed64 => json!({
            "oneOf": [
                { "type": "integer" },
                { "type": "string", "pattern": "^-?[0-9]+$" }
            ]
        }),
        FieldKind::Uint64 | FieldKind::Fixed64 => json!({
            "oneOf": [
                { "type": "integer", "minimum": 0 },
                { "type": "string", "pattern": "^[0-9]+$" }
            ]
        }),
        FieldKind::Bool => json!({ "type": "boolean" }),
        FieldKind::String => json!({ "type": "string" }),
        FieldKind::Bytes => json!({ "type": "string", "description": "base64-encoded bytes" }),
        FieldKind::Message(_) | FieldKind::Enum(_) => {
            unreachable!("message/enum handled by field_schema")
        }
    }
}

fn field_schema(
    field: &FieldDescriptor,
    queue: &mut VecDeque<Pending>,
    seen: &mut BTreeSet<String>,
) -> Value {
    // Maps must be checked before the generic message match: `field.kind()`
    // on a map field is the synthetic `FooEntry` message, which is never a
    // real reachable type and must not be walked as one.
    if field.is_map() {
        let FieldKind::Message(entry) = field.kind() else {
            unreachable!("is_map implies a synthetic map-entry message")
        };
        let value_field = entry.map_entry_value_field();
        return json!({
            "type": "object",
            "additionalProperties": field_schema(&value_field, queue, seen)
        });
    }

    let base = match field.kind() {
        FieldKind::Message(md) if md.full_name() == ID_MESSAGE => def_ref(ID_MESSAGE),
        FieldKind::Message(md) if md.full_name().starts_with("google.protobuf.") => {
            well_known_schema(md.full_name())
        }
        FieldKind::Message(md) => {
            enqueue_message(queue, seen, md.clone());
            def_ref(md.full_name())
        }
        FieldKind::Enum(ed) if ed.full_name() == ENTITY_KIND_ENUM => def_ref(ENTITY_KIND_ENUM),
        FieldKind::Enum(ed) => {
            enqueue_enum(queue, seen, ed.clone());
            def_ref(ed.full_name())
        }
        other => scalar_schema(&other),
    };

    if field.is_list() {
        json!({ "type": "array", "items": base })
    } else {
        base
    }
}

/// Build the plain `{"type":"object", "properties": {...}}` shape for a
/// message, mirroring `normalize_message`'s field walk one property at a
/// time. Root manifest-kind bodies drop `id`: manifest specs never carry it
/// (it comes from `metadata`), matching `parse_document`'s explicit
/// `spec.contains_key("id")` rejection.
fn plain_object_schema(
    desc: &MessageDescriptor,
    roots: &BTreeSet<String>,
    queue: &mut VecDeque<Pending>,
    seen: &mut BTreeSet<String>,
) -> Value {
    let is_root = roots.contains(desc.full_name());
    let mut properties = Map::new();
    for field in desc.fields() {
        if is_root && field.name() == "id" {
            continue;
        }
        let schema = field_schema(&field, queue, seen);
        properties.insert(field.json_name().to_string(), schema);
    }

    let mut all_of = Vec::new();
    for oneof in desc.oneofs() {
        let names: Vec<String> = oneof.fields().map(|f| f.json_name().to_string()).collect();
        for i in 0..names.len() {
            for j in (i + 1)..names.len() {
                all_of.push(json!({ "not": { "required": [names[i].clone(), names[j].clone()] } }));
            }
        }
    }

    let mut obj = Map::new();
    obj.insert("type".to_string(), json!("object"));
    obj.insert("additionalProperties".to_string(), json!(false));
    obj.insert("properties".to_string(), Value::Object(properties));
    if !all_of.is_empty() {
        obj.insert("allOf".to_string(), Value::Array(all_of));
    }
    Value::Object(obj)
}

/// Build the `$defs` entry for one reachable message. Root manifest-kind
/// bodies always use the plain object form (never the bare-string
/// shorthand: `spec` is always a mapping, enforced by `parse_document`
/// before normalization ever runs). Everything else mirrors
/// `normalize_message`'s precedence: `is_ref_like` first, then
/// `shorthand_ref_field`, else a plain object with no string form.
fn message_schema(
    desc: &MessageDescriptor,
    roots: &BTreeSet<String>,
    queue: &mut VecDeque<Pending>,
    seen: &mut BTreeSet<String>,
) -> Value {
    let object = plain_object_schema(desc, roots, queue, seen);
    if roots.contains(desc.full_name()) {
        return object;
    }
    if is_ref_like(desc) || shorthand_ref_field(desc).is_some() {
        json!({ "oneOf": [def_ref(SHORTHAND_DEF), object] })
    } else {
        object
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn generates_without_error() {
        let schema = generate().expect("schema generation");
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }

    #[test]
    fn is_deterministic_across_runs() {
        let a = generate().expect("schema generation");
        let b = generate().expect("schema generation");
        assert_eq!(a, b);
    }

    #[test]
    fn every_manifest_kind_has_a_spec_branch() {
        let schema = generate().expect("schema generation");
        let all_of = schema["allOf"].as_array().expect("allOf array");
        assert_eq!(all_of.len(), pb::canonical::ALL_KINDS.len());
    }

    #[test]
    fn entity_kind_enum_accepts_pascal_case_and_snake_case() {
        let schema = generate().expect("schema generation");
        let values: Vec<String> = schema["$defs"][ENTITY_KIND_ENUM]["enum"]
            .as_array()
            .expect("enum array")
            .iter()
            .map(|v| v.as_str().expect("string enum value").to_string())
            .collect();
        assert!(values.contains(&"CommandSlice".to_string()));
        assert!(values.contains(&"command_slice".to_string()));
    }

    #[test]
    fn root_spec_schema_has_no_id_property() {
        let schema = generate().expect("schema generation");
        let event = &schema["$defs"]["trogonatlas.eventmodel.v1alpha1.Event"];
        assert!(event["properties"].get("id").is_none());
    }

    // The committed file is what `trogon-atlas schema` prints and what editors
    // point yaml-language-server at; it must never silently drift from the
    // descriptors. Regenerate with:
    //   cargo run -p trogon-atlas-cli -- schema > rsworkspace/crates/trogon-atlas-client/manifest.schema.json
    // (run from the `atlas` directory).
    #[test]
    fn committed_schema_file_matches_generator() {
        let generated = generate().expect("schema generation");
        let committed: Value = serde_json::from_str(include_str!("../manifest.schema.json"))
            .expect("committed schema is valid JSON");
        assert_eq!(
            generated, committed,
            "manifest.schema.json is stale; regenerate with \
             `cargo run -p trogon-atlas-cli -- schema > rsworkspace/crates/trogon-atlas-client/manifest.schema.json` \
             (from the atlas directory)"
        );
    }

    fn crate_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn collect_yaml_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_yaml_files(&path, out);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"))
            {
                out.push(path);
            }
        }
    }

    /// Every YAML document in a manifest file, as raw (pre-normalization)
    /// JSON, the same shape `manifest::parse_str` feeds into
    /// `parse_document` before ref/id/kind shorthand expansion. Validating
    /// this raw shape against the schema is what proves the schema accepts
    /// shorthand the way the parser does, not just its expanded form.
    fn raw_documents(path: &std::path::Path) -> Vec<(String, Value)> {
        use serde::Deserialize as _;
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
        let mut out = Vec::new();
        for (i, doc) in serde_yaml::Deserializer::from_str(&text).enumerate() {
            let value = Value::deserialize(doc)
                .unwrap_or_else(|err| panic!("{}#doc{i}: invalid YAML: {err}", path.display()));
            if value.is_null() {
                continue;
            }
            out.push((format!("{}#doc{i}", path.display()), value));
        }
        out
    }

    #[test]
    fn every_fixture_manifest_validates_against_schema() {
        let schema = generate().expect("schema generation");
        let validator = jsonschema::validator_for(&schema).expect("schema compiles");

        let root = crate_root();
        let mut dirs = vec![
            root.join("../trogon-atlas-cli/examples/manifests"),
            root.join("../../../examples/manifests"),
        ];
        if let Some(extra) = std::env::var_os("TROGON_ATLAS_EXTRA_FIXTURE_DIRS") {
            dirs.extend(std::env::split_paths(&extra));
        }
        dirs.retain(|d| d.is_dir());
        assert!(!dirs.is_empty(), "no manifest fixture directories found");

        let mut files = Vec::new();
        for dir in &dirs {
            collect_yaml_files(dir, &mut files);
        }
        assert!(!files.is_empty(), "no manifest fixture files found");

        let mut failures = Vec::new();
        for file in &files {
            for (label, doc) in raw_documents(file) {
                if let Err(err) = validator.validate(&doc) {
                    failures.push(format!("{label}: {err}"));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} manifest document(s) failed schema validation:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
