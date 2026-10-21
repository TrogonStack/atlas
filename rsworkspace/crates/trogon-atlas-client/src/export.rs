// Export: live store → manifest YAML, the mechanical migration path away
// from imperative push scripts. The store is the net truth of whatever
// sequence of scripts produced it (including their delete/restore passes),
// so exporting and then verifying with `trogon-atlas diff` (expecting no drift)
// proves the manifests reproduce the model exactly.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use anyhow::{anyhow, bail, Context as _, Result};
use prost_reflect::{Kind as FieldKind, MessageDescriptor};
use serde_json::{Map, Value};
use trogon_atlas_core::{
    id_component::is_safe_id_component,
    refs::{self, outbound_refs},
    transcode::TranscodePool,
};
use trogon_atlas_proto as pb;

use crate::{
    client::Client,
    manifest::{is_ref_like, shorthand_ref_field, API_VERSION, ID_MESSAGE},
    tenant_types::{self, TypeScope},
};

/// PascalCase manifest kind from the canonical short form:
/// "read_model" → "ReadModel", "ui" → "Ui".
pub(crate) fn manifest_kind(kind: pb::EntityKind) -> String {
    pb::canonical::kind_short(kind)
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Render one entity as a manifest document (JSON value; YAML comes from
/// serde_norway at write time). Strips `id` (moves to metadata) and the
/// server-owned `system` block. Shared with `fmt`, which re-manifests
/// entities freshly parsed from disk rather than fetched from the store.
pub(crate) fn entity_to_manifest(
    entity: &pb::Entity,
    types: &TranscodePool,
) -> Result<(pb::EntityKind, pb::Id, Value)> {
    let kind = entity
        .kind
        .as_ref()
        .map(entity_kind_of)
        .ok_or_else(|| anyhow!("stored entity has no kind"))?;
    let json = types
        .entity_json_value(entity)
        .context("rendering Entity as proto JSON")?;
    let Value::Object(mut envelope) = json else {
        bail!("Entity rendered as non-object JSON");
    };
    envelope.remove("system");
    let body_key = envelope
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| anyhow!("Entity JSON has no kind field"))?;
    let Some(Value::Object(mut body)) = envelope.remove(&body_key) else {
        bail!("Entity body is not an object");
    };
    let id_value = body
        .remove("id")
        .ok_or_else(|| anyhow!("entity body has no id"))?;
    let id = id_from_json(&id_value)?;
    let body_desc = types
        .entity_descriptor()
        .ok()
        .and_then(|entity| entity.fields().find(|f| f.json_name() == body_key))
        .and_then(|field| match field.kind() {
            FieldKind::Message(md) => Some(md),
            _ => None,
        })
        .ok_or_else(|| anyhow!("Entity has no message field {body_key:?}"))?;
    let mut spec = Value::Object(body);
    compact_message(&mut spec, &body_desc, &id.namespace);

    let mut metadata = Map::new();
    metadata.insert("namespace".into(), Value::String(id.namespace.clone()));
    metadata.insert("name".into(), Value::String(id.slug.clone()));
    if id.version != 1 {
        metadata.insert("version".into(), Value::Number(id.version.into()));
    }

    let mut doc = Map::new();
    doc.insert("apiVersion".into(), Value::String(API_VERSION.into()));
    doc.insert("kind".into(), Value::String(manifest_kind(kind)));
    doc.insert("metadata".into(), Value::Object(metadata));
    doc.insert("spec".into(), spec);
    Ok((kind, id, Value::Object(doc)))
}

/// Undo `manifest::normalize_message`: write each Id, Ref, Edge and Example
/// wrapper as the shortest string the parser expands back to the same value.
/// A wrapper with any other key, or an Id the string grammar can't carry
/// exactly, stays an object.
fn compact_message(value: &mut Value, desc: &MessageDescriptor, default_ns: &str) {
    if desc.full_name() == ID_MESSAGE {
        if let Some(short) = short_id(value, default_ns) {
            *value = Value::String(short);
        }
        return;
    }
    // Any-packed annotations are opaque to the parser's schema walk too.
    if desc.full_name().starts_with("google.protobuf.") {
        return;
    }
    let Value::Object(map) = value else {
        return;
    };
    for (key, val) in map.iter_mut() {
        let Some(field) = desc
            .fields()
            .find(|f| f.json_name() == key || f.name() == key)
        else {
            continue;
        };
        let FieldKind::Message(md) = field.kind() else {
            continue;
        };
        if field.is_map() {
            continue;
        }
        match val {
            Value::Array(items) if field.is_list() => {
                for item in items {
                    compact_message(item, &md, default_ns);
                }
            }
            _ if field.is_list() => {}
            _ => compact_message(val, &md, default_ns),
        }
    }
    // Mirror the parser's order: a Ref-like message takes a string as its
    // `id`, and only a non-Ref message routes a string to its shorthand field.
    let wrapped = if is_ref_like(desc) {
        "id".to_string()
    } else if let Some(field) = shorthand_ref_field(desc) {
        field.json_name().to_string()
    } else {
        return;
    };
    if map.len() == 1 {
        if let Some(Value::String(short)) = map.get(&wrapped) {
            *value = Value::String(short.clone());
        }
    }
}

/// `slug`, `ns/slug` or `ns/slug@v`, or None when `expand_id_string` would
/// not read the string back as exactly this Id.
fn short_id(value: &Value, default_ns: &str) -> Option<String> {
    let map = value.as_object()?;
    if map.len() != 3 {
        return None;
    }
    let namespace = map.get("namespace")?.as_str()?;
    let slug = map.get("slug")?.as_str()?;
    let version = match map.get("version")? {
        Value::String(s) => s.parse::<u64>().ok()?,
        Value::Number(n) => n.as_u64()?,
        _ => return None,
    };
    if version == 0 || !is_safe_id_component(namespace) || !is_safe_id_component(slug) {
        return None;
    }
    let qualified = if namespace == default_ns {
        slug.to_string()
    } else {
        format!("{namespace}/{slug}")
    };
    Some(match version {
        1 => qualified,
        v => format!("{qualified}@{v}"),
    })
}

// Proto3 JSON renders uint64 as a string; extract the triple by hand
// (prost types carry no serde impls).
fn id_from_json(value: &Value) -> Result<pb::Id> {
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow!("exported id is not an object"))?;
    let field = |key: &str| obj.get(key).and_then(Value::as_str).unwrap_or_default();
    let version = match obj.get("version") {
        Some(Value::String(s)) => s.parse::<u64>().context("parsing id.version")?,
        Some(Value::Number(n)) => n.as_u64().ok_or_else(|| anyhow!("negative id.version"))?,
        _ => bail!("exported id has no version"),
    };
    Ok(pb::Id {
        namespace: field("namespace").to_string(),
        slug: field("slug").to_string(),
        version,
    })
}

/// Best-effort "kind:ns/slug@v" label for reporting, even when the entity
/// cannot be rendered as JSON.
pub(crate) fn entity_label(entity: &pb::Entity) -> String {
    let Some(kind) = entity.kind.as_ref() else {
        return "<no kind>".to_string();
    };
    use pb::entity::Kind as K;
    let id = match kind {
        K::Event(x) => x.id.as_ref(),
        K::Command(x) => x.id.as_ref(),
        K::ReadModel(x) => x.id.as_ref(),
        K::Processor(x) => x.id.as_ref(),
        K::Ui(x) => x.id.as_ref(),
        K::Persona(x) => x.id.as_ref(),
        K::Swimlane(x) => x.id.as_ref(),
        K::CommandSlice(x) => x.id.as_ref(),
        K::ReadModelSlice(x) => x.id.as_ref(),
        K::AutomationSlice(x) => x.id.as_ref(),
        K::Storyboard(x) => x.id.as_ref(),
        K::EventModel(x) => x.id.as_ref(),
        K::Component(x) => x.id.as_ref(),
        K::ExternalSystem(x) => x.id.as_ref(),
        K::Tracker(x) => x.id.as_ref(),
        K::BoundedContext(x) => x.id.as_ref(),
        K::Domain(x) => x.id.as_ref(),
        K::Subdomain(x) => x.id.as_ref(),
        K::Schema(x) => x.id.as_ref(),
        K::Project(x) => x.id.as_ref(),
        K::Screen(x) => x.id.as_ref(),
        K::Term(x) => x.id.as_ref(),
        K::Ambiguity(x) => x.id.as_ref(),
        K::UiSlice(x) => x.id.as_ref(),
        K::ServiceLevelIndicator(x) => x.id.as_ref(),
        K::ServiceLevelObjective(x) => x.id.as_ref(),
        K::AlertPolicy(x) => x.id.as_ref(),
        K::AlertNotificationTarget(x) => x.id.as_ref(),
        K::TypeLibrary(x) => x.id.as_ref(),
        K::Channel(x) => x.id.as_ref(),
        K::DataStore(x) => x.id.as_ref(),
        K::DataProduct(x) => x.id.as_ref(),
        K::Specification(x) => x.id.as_ref(),
        K::Team(x) => x.id.as_ref(),
        K::Person(x) => x.id.as_ref(),
    };
    match id {
        Some(id) => pb::canonical::id_string(entity_kind_of(kind), id),
        None => format!(
            "{}:<no id>",
            pb::canonical::kind_short(entity_kind_of(kind))
        ),
    }
}

fn entity_kind_of(kind: &pb::entity::Kind) -> pb::EntityKind {
    use pb::entity::Kind as K;
    match kind {
        K::Event(_) => pb::EntityKind::Event,
        K::Command(_) => pb::EntityKind::Command,
        K::ReadModel(_) => pb::EntityKind::ReadModel,
        K::Processor(_) => pb::EntityKind::Processor,
        K::Ui(_) => pb::EntityKind::Ui,
        K::Persona(_) => pb::EntityKind::Persona,
        K::Swimlane(_) => pb::EntityKind::Swimlane,
        K::CommandSlice(_) => pb::EntityKind::CommandSlice,
        K::ReadModelSlice(_) => pb::EntityKind::ReadModelSlice,
        K::AutomationSlice(_) => pb::EntityKind::AutomationSlice,
        K::Storyboard(_) => pb::EntityKind::Storyboard,
        K::EventModel(_) => pb::EntityKind::EventModel,
        K::Component(_) => pb::EntityKind::Component,
        K::ExternalSystem(_) => pb::EntityKind::ExternalSystem,
        K::Tracker(_) => pb::EntityKind::Tracker,
        K::BoundedContext(_) => pb::EntityKind::BoundedContext,
        K::Domain(_) => pb::EntityKind::Domain,
        K::Subdomain(_) => pb::EntityKind::Subdomain,
        K::Schema(_) => pb::EntityKind::Schema,
        K::Project(_) => pb::EntityKind::Project,
        K::Screen(_) => pb::EntityKind::Screen,
        K::Term(_) => pb::EntityKind::Term,
        K::Ambiguity(_) => pb::EntityKind::Ambiguity,
        K::UiSlice(_) => pb::EntityKind::UiSlice,
        K::ServiceLevelIndicator(_) => pb::EntityKind::ServiceLevelIndicator,
        K::ServiceLevelObjective(_) => pb::EntityKind::ServiceLevelObjective,
        K::AlertPolicy(_) => pb::EntityKind::AlertPolicy,
        K::AlertNotificationTarget(_) => pb::EntityKind::AlertNotificationTarget,
        K::TypeLibrary(_) => pb::EntityKind::TypeLibrary,
        K::Channel(_) => pb::EntityKind::Channel,
        K::DataStore(_) => pb::EntityKind::DataStore,
        K::DataProduct(_) => pb::EntityKind::DataProduct,
        K::Specification(_) => pb::EntityKind::Specification,
        K::Team(_) => pb::EntityKind::Team,
        K::Person(_) => pb::EntityKind::Person,
    }
}

pub struct ExportOutcome {
    /// file name → multi-document YAML content.
    pub files: BTreeMap<String, String>,
    /// Entities that could not be rendered (e.g. annotations packed with a
    /// type url no longer present in the schema). Never silently dropped:
    /// the caller must show these.
    pub skipped: Vec<String>,
}

/// How `export_namespace*` lays the manifest documents out on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportLayout {
    Single,
    Slices,
}

pub(crate) async fn namespace_types(client: &mut Client, namespace: &str) -> Result<TranscodePool> {
    let mut scope = TypeScope::default();
    scope.add_namespace(namespace);
    tenant_types::transcode_pool(client, &scope).await
}

pub async fn fetch_namespace_entities(
    client: &mut Client,
    namespace: &str,
) -> Result<Vec<pb::Entity>> {
    let mut entities: Vec<pb::Entity> = Vec::new();
    let mut page_token = String::new();
    loop {
        let resp = client
            .list_entities(pb::ListEntitiesRequest {
                kinds: vec![],
                namespaces: vec![namespace.to_string()],
                latest_versions_only: false,
                page_size: 200,
                page_token: page_token.clone(),
                lifecycle_status_in: vec![],
                lifecycle_status_not_in: vec![],
            })
            .await
            .context("ListEntities failed")?
            .into_inner();
        entities.extend(resp.entities);
        if resp.next_page_token.is_empty() {
            break;
        }
        page_token = resp.next_page_token;
    }
    if entities.is_empty() {
        crate::invalid_input!("namespace {namespace:?} has no entities");
    }
    Ok(entities)
}

pub(crate) fn render_docs<'a>(docs: impl IntoIterator<Item = &'a Value>) -> Result<String> {
    let mut out = String::new();
    for (i, doc) in docs.into_iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        out.push_str(&serde_norway::to_string(doc).context("rendering YAML")?);
    }
    Ok(out)
}

/// Fetch every entity version in `namespace` and render manifest files,
/// one multi-document YAML per kind, entities sorted by (slug, version).
pub async fn export_namespace(client: &mut Client, namespace: &str) -> Result<ExportOutcome> {
    let entities = fetch_namespace_entities(client, namespace).await?;
    let types = namespace_types(client, namespace).await?;

    // kind → [(slug, version, doc)]
    let mut by_kind: BTreeMap<String, Vec<(String, u64, Value)>> = BTreeMap::new();
    let mut skipped = Vec::new();
    for entity in &entities {
        match entity_to_manifest(entity, &types) {
            Ok((kind, id, doc)) => {
                by_kind
                    .entry(pb::canonical::kind_short(kind).replace('_', "-"))
                    .or_default()
                    .push((id.slug, id.version, doc));
            }
            Err(err) => skipped.push(format!("{}: {err:#}", entity_label(entity))),
        }
    }

    let mut files = BTreeMap::new();
    for (kind_file, mut docs) in by_kind {
        docs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let rendered = render_docs(docs.iter().map(|(_, _, doc)| doc))?;
        files.insert(format!("{kind_file}.yaml"), rendered);
    }
    Ok(ExportOutcome { files, skipped })
}

/// A board rendered as manifests: its own entities, and the entities in
/// other namespaces it references directly.
#[derive(Debug)]
pub struct BoardManifests {
    pub manifests: String,
    pub external_manifests: String,
    pub skipped: Vec<pb::SkippedEntity>,
}

/// Render a board's entities as multi-document YAML, each set sorted by
/// (kind, namespace, slug, version) so the output is stable across exports.
pub fn render_board(
    own: &[pb::Entity],
    external: &[pb::Entity],
    types: &TranscodePool,
) -> Result<BoardManifests> {
    render_board_scoped(own, external, |_| Some(types))
}

/// Render each entity with its namespace's types, so independent namespaces
/// can reuse protobuf file and message names.
pub fn render_board_scoped<'a>(
    own: &[pb::Entity],
    external: &[pb::Entity],
    types: impl Fn(&str) -> Option<&'a TranscodePool>,
) -> Result<BoardManifests> {
    let mut skipped = Vec::new();
    let manifests = render_sorted(own, &types, &mut skipped)?;
    let external_manifests = render_sorted(external, &types, &mut skipped)?;
    Ok(BoardManifests {
        manifests,
        external_manifests,
        skipped,
    })
}

fn render_sorted<'a>(
    entities: &[pb::Entity],
    types: &impl Fn(&str) -> Option<&'a TranscodePool>,
    skipped: &mut Vec<pb::SkippedEntity>,
) -> Result<String> {
    let mut docs: Vec<(pb::EntityKey, Value)> = Vec::new();
    for entity in entities {
        let rendered = refs::entity_id(entity)
            .ok_or_else(|| anyhow!("stored entity has no id"))
            .and_then(|id| {
                types(&id.namespace)
                    .ok_or_else(|| anyhow!("no type pool for namespace {:?}", id.namespace))
            })
            .and_then(|types| entity_to_manifest(entity, types));
        match rendered {
            Ok((kind, id, doc)) => docs.push((pb::EntityKey::new(kind, &id), doc)),
            Err(err) => skipped.push(pb::SkippedEntity {
                entity: Some(pb::EntityRef {
                    kind: refs::entity_kind(entity).map_or(0, |k| k as i32),
                    id: refs::entity_id(entity).cloned(),
                }),
                reason: format!("{err:#}"),
            }),
        }
    }
    docs.sort_by(|a, b| sort_key(&a.0).cmp(&sort_key(&b.0)));
    render_docs(docs.iter().map(|(_, doc)| doc))
}

fn is_slice_kind(kind: pb::EntityKind) -> bool {
    matches!(
        kind,
        pb::EntityKind::CommandSlice
            | pb::EntityKind::ReadModelSlice
            | pb::EntityKind::AutomationSlice
            | pb::EntityKind::UiSlice
    )
}

fn closure_refs(
    start: &pb::EntityKey,
    entities: &HashMap<pb::EntityKey, pb::Entity>,
) -> Vec<pb::EntityKey> {
    let mut visited: HashSet<pb::EntityKey> = HashSet::new();
    visited.insert(start.clone());
    let mut queue: VecDeque<pb::EntityKey> = VecDeque::new();
    queue.push_back(start.clone());
    let mut closure = Vec::new();
    while let Some(key) = queue.pop_front() {
        let Some(entity) = entities.get(&key) else {
            continue;
        };
        let mut targets: Vec<pb::EntityKey> = outbound_refs(entity)
            .into_iter()
            .filter_map(|r| r.to_kind.map(|k| pb::EntityKey::new(k, &r.to_id)))
            .collect();
        if let Some(id) = trogon_atlas_core::schema::declared_schema_id(entity) {
            targets.push(pb::EntityKey::new(pb::EntityKind::Schema, &id));
        }
        for target in targets {
            if visited.insert(target.clone()) {
                closure.push(target.clone());
                queue.push_back(target);
            }
        }
    }
    closure
}

fn sort_key(key: &pb::EntityKey) -> (&'static str, &str, &str, u64) {
    (
        pb::canonical::kind_short(key.kind),
        key.namespace.as_str(),
        key.slug.as_str(),
        key.version,
    )
}

/// Safe as a path because `validate_id_component` forbids `/` and `@`.
fn slice_file_path(key: &pb::EntityKey) -> String {
    if key.version == 1 {
        format!("{}/{}.yaml", key.namespace, key.slug)
    } else {
        format!("{}/{}@{}.yaml", key.namespace, key.slug, key.version)
    }
}

pub async fn export_namespace_slices(
    client: &mut Client,
    namespace: &str,
) -> Result<ExportOutcome> {
    let entities = fetch_namespace_entities(client, namespace).await?;
    let types = namespace_types(client, namespace).await?;

    let mut docs: HashMap<pb::EntityKey, Value> = HashMap::new();
    let mut raw: HashMap<pb::EntityKey, pb::Entity> = HashMap::new();
    let mut skipped = Vec::new();
    for entity in &entities {
        match entity_to_manifest(entity, &types) {
            Ok((kind, id, doc)) => {
                let key = pb::EntityKey::new(kind, &id);
                docs.insert(key.clone(), doc);
                raw.insert(key, entity.clone());
            }
            Err(err) => skipped.push(format!("{}: {err:#}", entity_label(entity))),
        }
    }

    let mut slice_keys: Vec<pb::EntityKey> = raw
        .keys()
        .filter(|key| is_slice_kind(key.kind))
        .cloned()
        .collect();
    slice_keys.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

    let mut files = BTreeMap::new();
    let mut claimed: HashMap<String, pb::EntityKey> = HashMap::new();
    let mut referenced: HashSet<pb::EntityKey> = HashSet::new();

    for slice_key in &slice_keys {
        let closure = closure_refs(slice_key, &raw);
        referenced.extend(closure.iter().cloned());

        let mut bundled: Vec<&pb::EntityKey> = closure
            .iter()
            .filter(|key| docs.contains_key(*key))
            .collect();
        bundled.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));

        let mut ordered_docs: Vec<&Value> = vec![&docs[slice_key]];
        ordered_docs.extend(bundled.into_iter().map(|key| &docs[key]));
        let content = render_docs(ordered_docs)?;

        let path = slice_file_path(slice_key);
        if let Some(prev) = claimed.insert(path.clone(), slice_key.clone()) {
            bail!("slice file name collision at {path:?} between {prev} and {slice_key}");
        }
        files.insert(path, content);
    }

    let mut unsliced_keys: Vec<&pb::EntityKey> = docs
        .keys()
        .filter(|key| !is_slice_kind(key.kind) && !referenced.contains(*key))
        .collect();
    unsliced_keys.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    if !unsliced_keys.is_empty() {
        let content = render_docs(unsliced_keys.into_iter().map(|key| &docs[key]))?;
        files.insert("unsliced.yaml".to_string(), content);
    }

    Ok(ExportOutcome { files, skipped })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn exported_spec(yaml: &str) -> Value {
        let loaded = crate::manifest::parse_str(yaml, "test.yaml").unwrap();
        let types = TranscodePool::builtin().unwrap();
        let (_, _, doc) = entity_to_manifest(&loaded[0].entity, &types).unwrap();
        doc["spec"].clone()
    }

    #[test]
    fn exports_refs_in_short_form() {
        let spec = exported_spec(
            r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: CommandSlice
metadata: { namespace: acme-storefront, name: add-item-to-cart-slice }
spec:
  title: Add item to cart
  command: add-item-to-cart
  emittedEvents:
    - cart.item-added
    - acme-billing/invoice.raised@2
    - { event: cart.item-noted, doc: Kept as a mapping because it has a doc. }
",
        );
        assert_eq!(spec["command"], json!("add-item-to-cart"));
        assert_eq!(
            spec["emittedEvents"],
            json!([
                "cart.item-added",
                "acme-billing/invoice.raised@2",
                { "event": "cart.item-noted", "doc": "Kept as a mapping because it has a doc." },
            ])
        );
    }

    #[test]
    fn keeps_long_form_when_short_form_would_parse_differently() {
        let desc = crate::manifest::entity_descriptor()
            .unwrap()
            .get_field_by_name("command_slice")
            .map(|f| match f.kind() {
                prost_reflect::Kind::Message(md) => md,
                _ => unreachable!(),
            })
            .unwrap();
        // "cart@2" as a string would read back as slug "cart", version 2.
        let long = json!({
            "command": { "command": { "id": {
                "namespace": "acme-storefront", "slug": "cart@2", "version": "1",
            } } },
        });
        let mut value = long.clone();
        compact_message(&mut value, &desc, "acme-storefront");
        assert_eq!(value, long);
    }
}
