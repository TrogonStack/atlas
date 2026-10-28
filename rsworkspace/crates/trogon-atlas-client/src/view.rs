//! Read-only views of one entity or of a namespace's structure. The
//! builders and renderers are pure functions over the gRPC response types,
//! so the CLI's text output and any later terminal UI show the same thing.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
};

use anyhow::{Context as _, Result};
use prost::Message as _;
use prost_reflect::{DynamicMessage, ReflectMessage as _};
use serde::Serialize;
use serde_json::{json, Value};
use trogon_atlas_core::{
    entity_id, entity_kind, outbound_refs,
    schema::{entity_schema, schema_fields_in},
    transcode::TranscodePool,
};
use trogon_atlas_proto as pb;

use crate::{client::Client, export, failure::invalid_input};

/// `<kind>/<namespace>/<slug>[@version]`, or the `<kind>:<namespace>/<slug>[@version]`
/// form `describe` prints; `version: None` means latest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityAddress {
    pub kind: pb::EntityKind,
    pub namespace: String,
    pub slug: String,
    pub version: Option<u64>,
}

pub fn parse_address(s: &str) -> Result<EntityAddress> {
    let Some((kind, namespace, rest)) = s
        .split_once([':', '/'])
        .and_then(|(kind, rest)| rest.split_once('/').map(|(ns, rest)| (kind, ns, rest)))
    else {
        crate::invalid_input!("expected <kind>/<namespace>/<slug>[@version], got {s:?}");
    };
    let kind = pb::canonical::parse_kind(kind)
        .ok_or_else(|| invalid_input(format!("unknown entity kind: {kind}")))?;
    let (slug, version) = match rest.rsplit_once('@') {
        Some((slug, v)) => match v.parse::<u64>() {
            Ok(n) if n >= 1 => (slug, Some(n)),
            _ => crate::invalid_input!("version must be a positive integer, got {v:?}"),
        },
        None => (rest, None),
    };
    for (what, value) in [("namespace", namespace), ("slug", slug)] {
        trogon_atlas_core::validate_id_component(value)
            .map_err(|e| invalid_input(format!("{what} {e}")))?;
    }
    Ok(EntityAddress {
        kind,
        namespace: namespace.to_string(),
        slug: slug.to_string(),
        version,
    })
}

pub async fn fetch_entity(client: &mut Client, address: &EntityAddress) -> Result<pb::Entity> {
    let entity = if let Some(version) = address.version {
        let id = pb::Id {
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
            version,
        };
        let req = pb::GetEntityRequest {
            kind: address.kind as i32,
            id: Some(id),
        };
        let resp = client.get_entity(req).await.context("GetEntity failed")?;
        resp.into_inner().entity
    } else {
        let req = pb::GetLatestVersionRequest {
            kind: address.kind as i32,
            namespace: address.namespace.clone(),
            slug: address.slug.clone(),
        };
        let resp = client
            .get_latest_version(req)
            .await
            .context("GetLatestVersion failed")?;
        resp.into_inner().entity
    };
    // GetLatestVersion answers OK with no entity for an unknown slug; report
    // it as NOT_FOUND so it exits like a missing pinned version does.
    entity.ok_or_else(|| {
        let kind = pb::canonical::kind_short(address.kind);
        let (ns, slug) = (&address.namespace, &address.slug);
        anyhow::Error::new(tonic::Status::not_found(format!(
            "{kind}:{ns}/{slug} not found"
        )))
    })
}

/// The built-in types plus the tenant types of the entity's namespace.
pub async fn entity_types(client: &mut Client, entity: &pb::Entity) -> Result<TranscodePool> {
    let namespace = entity_id(entity)
        .map(|id| id.namespace.clone())
        .unwrap_or_default();
    export::namespace_types(client, &namespace).await
}

/// The entity as the same manifest document `export` writes, both as JSON
/// and as rendered YAML.
pub fn entity_manifest(entity: &pb::Entity, types: &TranscodePool) -> Result<(Value, String)> {
    let (_, _, doc) = export::entity_to_manifest(entity, types)?;
    let yaml = export::render_docs([&doc])?;
    Ok((doc, yaml))
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Outgoing,
    Incoming,
}

pub async fn fetch_references(
    client: &mut Client,
    entity: &pb::Entity,
    direction: Direction,
) -> Result<Vec<pb::Reference>> {
    let kind = entity_kind(entity).unwrap_or_default();
    let id = entity_id(entity).cloned();
    let mut references = Vec::new();
    let mut page_token = String::new();
    loop {
        let req = pb::GetReferencesRequest {
            kind: kind as i32,
            id: id.clone(),
            filter_kinds: vec![],
            page_size: 0,
            page_token,
        };
        let resp = match direction {
            Direction::Outgoing => client.get_outgoing_references(req).await,
            Direction::Incoming => client.get_incoming_references(req).await,
        }
        .with_context(|| format!("Get{direction:?}References failed"))?
        .into_inner();
        references.extend(resp.references);
        if resp.next_page_token.is_empty() {
            return Ok(references);
        }
        page_token = resp.next_page_token;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Description {
    pub id: String,
    pub title: String,
    pub doc: String,
    /// `None` for kinds that carry no schema, so "no fields declared" stays
    /// distinguishable from "this kind has no fields".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<FieldLine>>,
    pub metadata: Vec<Value>,
    pub outgoing: Vec<RefLine>,
    pub incoming: Vec<RefLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldLine {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
}

/// `field` is the reference site inside the entity that holds the pointer;
/// `entity` is the other end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefLine {
    pub field: String,
    pub entity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

/// `types` must know the entity's tenant types, or a tenant-typed schema
/// shows no fields.
pub fn describe(
    entity: &pb::Entity,
    types: &TranscodePool,
    outgoing: &[pb::Reference],
    incoming: &[pb::Reference],
) -> Result<Description> {
    let has_schema = matches!(
        entity_kind(entity),
        Some(pb::EntityKind::Event | pb::EntityKind::Command | pb::EntityKind::ReadModel)
    );
    let fields = has_schema.then(|| {
        schema_fields_in(types, entity_schema(entity))
            .iter()
            .map(|f| {
                let label = f
                    .r#type
                    .as_ref()
                    .map_or("unspecified", pb::canonical::field_type_label);
                let suffix = match (f.repeated, f.optional) {
                    (true, _) => "[]",
                    (false, true) => "?",
                    (false, false) => "",
                };
                FieldLine {
                    name: f.name.clone(),
                    field_type: format!("{label}{suffix}"),
                }
            })
            .collect()
    });
    let line = |r: &pb::Reference, other: Option<&pb::EntityRef>| -> Result<RefLine> {
        let provenance = (!r.from_uid.is_empty()
            || r.responsibility.is_some()
            || r.membership.is_some()
            || r.catalog_subject.is_some()
            || r.document_selection.is_some()
            || r.connection_id.is_some())
        .then(|| types.message_json_value("trogonatlas.api.eventmodel.v1alpha1.Reference", r))
        .transpose()
        .context("encode exact reference provenance")?;
        Ok(RefLine {
            field: r.from_field.clone(),
            entity: ref_label(other),
            provenance,
        })
    };
    let mut incoming = incoming
        .iter()
        .map(|r| line(r, r.from.as_ref()))
        .collect::<Result<Vec<_>>>()?;
    incoming.sort_by(|a, b| (&a.entity, &a.field).cmp(&(&b.entity, &b.field)));
    let dynamic = DynamicMessage::decode(
        types.entity_descriptor()?,
        entity.encode_to_vec().as_slice(),
    )?;
    let mut metadata = Vec::new();
    for (_, value) in dynamic.fields() {
        if let prost_reflect::Value::Message(body) = value {
            if let Some(field) = body.descriptor().get_field_by_name("metadata") {
                if let prost_reflect::Value::List(items) = body.get_field(&field).as_ref() {
                    for item in items {
                        if let prost_reflect::Value::Message(annotation) = item {
                            let any =
                                prost_types::Any::decode(annotation.encode_to_vec().as_slice())?;
                            metadata.push(
                                types
                                    .message_json_value("google.protobuf.Any", &any)
                                    .context("decode entity metadata")?,
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(Description {
        id: export::entity_label(entity),
        title: text_field(entity, "title"),
        doc: text_field(entity, "doc"),
        fields,
        metadata,
        outgoing: outgoing
            .iter()
            .map(|r| line(r, r.to.as_ref()))
            .collect::<Result<_>>()?,
        incoming,
    })
}

pub fn render_description(d: &Description) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Id:     {}", d.id);
    let _ = writeln!(out, "Title:  {}", d.title);
    let _ = writeln!(out, "Doc:    {}", d.doc.replace('\n', "\n        "));
    for annotation in &d.metadata {
        let _ = writeln!(out, "Metadata: {annotation}");
    }
    if let Some(fields) = &d.fields {
        let rows = fields
            .iter()
            .map(|f| (f.name.as_str(), f.field_type.as_str()));
        push_section(&mut out, "Fields:", rows);
    }
    let rows = d
        .outgoing
        .iter()
        .map(|r| (r.field.as_str(), r.entity.as_str()));
    push_section(&mut out, "References:", rows);
    let rows = d
        .incoming
        .iter()
        .map(|r| (r.entity.as_str(), r.field.as_str()));
    push_section(&mut out, "Referenced by:", rows);
    for line in d.outgoing.iter().chain(&d.incoming) {
        if let Some(provenance) = &line.provenance {
            let _ = writeln!(out, "Exact reference provenance: {provenance}");
        }
    }
    out
}

fn push_section<'a>(
    out: &mut String,
    heading: &str,
    rows: impl Iterator<Item = (&'a str, &'a str)>,
) {
    let rows: Vec<_> = rows.collect();
    let _ = writeln!(out, "{heading}");
    if rows.is_empty() {
        out.push_str("  <none>\n");
    }
    let width = rows.iter().map(|(left, _)| left.len()).max().unwrap_or(0);
    for (left, right) in rows {
        let _ = writeln!(out, "  {left:<width$}  {right}");
    }
}

/// Every namespace the caller can see, sorted by name.
pub async fn fetch_namespaces(
    client: &mut Client,
) -> Result<Vec<pb::list_namespaces_response::Namespace>> {
    let resp = client
        .list_namespaces(pb::ListNamespacesRequest {})
        .await
        .context("ListNamespaces failed")?;
    let mut namespaces = resp.into_inner().namespaces;
    namespaces.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    Ok(namespaces)
}

pub fn render_namespaces(namespaces: &[pb::list_namespaces_response::Namespace]) -> String {
    if namespaces.is_empty() {
        return "no namespaces\n".to_string();
    }
    let width = |f: fn(&pb::list_namespaces_response::Namespace) -> &str, header: &str| {
        namespaces
            .iter()
            .map(|n| f(n).len())
            .max()
            .unwrap_or(0)
            .max(header.len())
    };
    let name_width = width(|n| &n.name, "NAME");
    let id_width = width(|n| &n.id, "ID");
    let mut out = format!("{:<name_width$}  {:<id_width$}  ENTITIES\n", "NAME", "ID");
    for n in namespaces {
        let _ = writeln!(
            out,
            "{:<name_width$}  {:<id_width$}  {}",
            n.name, n.id, n.entity_count
        );
    }
    out
}

pub fn namespaces_json(namespaces: &[pb::list_namespaces_response::Namespace]) -> Value {
    let rows: Vec<Value> = namespaces
        .iter()
        .map(|n| {
            json!({
                "id": n.id,
                "name": n.name,
                "entity_count": n.entity_count,
                "parent": n.parent,
            })
        })
        .collect();
    json!({"namespaces": rows})
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamespaceTree {
    pub namespace: String,
    pub domains: Vec<TreeNode>,
    pub storyboards: Vec<TreeNode>,
    /// Slices no storyboard lists, so nothing in the namespace is hidden.
    pub unplaced_slices: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeNode {
    pub kind: &'static str,
    pub namespace: String,
    pub slug: String,
    pub version: u64,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
}

const SLICE_KINDS: [pb::EntityKind; 4] = [
    pb::EntityKind::CommandSlice,
    pb::EntityKind::ReadModelSlice,
    pb::EntityKind::AutomationSlice,
    pb::EntityKind::UiSlice,
];

/// Builds domain -> subdomain and storyboard -> slice -> command / event /
/// read model / processor from the latest version of each entity. A slice's
/// scenario references are left out: they restate its edges as examples.
pub fn namespace_tree(namespace: &str, entities: &[pb::Entity]) -> NamespaceTree {
    let index: HashMap<pb::EntityKey, &pb::Entity> = entities
        .iter()
        .filter_map(|e| Some((pb::EntityKey::new(entity_kind(e)?, entity_id(e)?), e)))
        .collect();
    let mut newest: HashMap<(pb::EntityKind, &str, &str), u64> = HashMap::new();
    for key in index.keys() {
        let v = newest
            .entry((key.kind, key.namespace.as_str(), key.slug.as_str()))
            .or_default();
        *v = (*v).max(key.version);
    }
    let mut latest: Vec<&pb::EntityKey> = index
        .keys()
        .filter(|k| newest[&(k.kind, k.namespace.as_str(), k.slug.as_str())] == k.version)
        .collect();
    latest.sort_by(|a, b| (&a.slug, &a.namespace).cmp(&(&b.slug, &b.namespace)));
    let of_kind = |kind: pb::EntityKind| latest.iter().copied().filter(move |k| k.kind == kind);
    let node = |key: &pb::EntityKey, children: Vec<TreeNode>| TreeNode {
        kind: pb::canonical::kind_short(key.kind),
        namespace: key.namespace.clone(),
        slug: key.slug.clone(),
        version: key.version,
        title: index
            .get(key)
            .map(|e| text_field(e, "title"))
            .unwrap_or_default(),
        children,
    };
    let refs_of =
        |key: &pb::EntityKey| index.get(key).map(|e| outbound_refs(e)).unwrap_or_default();

    let domains = of_kind(pb::EntityKind::Domain)
        .map(|domain| {
            let subdomains = of_kind(pb::EntityKind::Subdomain)
                .filter(|s| {
                    refs_of(s).iter().any(|r| {
                        r.from_field == "domain"
                            && r.to_id.namespace == domain.namespace
                            && r.to_id.slug == domain.slug
                    })
                })
                .map(|s| node(s, vec![]))
                .collect();
            node(domain, subdomains)
        })
        .collect();

    let slice_node = |key: &pb::EntityKey| {
        let mut seen = HashSet::new();
        let children = refs_of(key)
            .into_iter()
            .filter(|r| !r.from_field.starts_with("scenarios"))
            .filter_map(|r| {
                let kind = r.to_kind.filter(|k| {
                    matches!(
                        k,
                        pb::EntityKind::Command
                            | pb::EntityKind::Event
                            | pb::EntityKind::ReadModel
                            | pb::EntityKind::Processor
                    )
                })?;
                Some(pb::EntityKey::new(kind, &r.to_id))
            })
            .filter(|k| seen.insert(k.clone()))
            .map(|k| node(&k, vec![]))
            .collect();
        node(key, children)
    };

    // Keyed without the version, so a storyboard that pins an older slice
    // still places that slice's newer versions.
    let mut placed = HashSet::new();
    let mut storyboards = Vec::new();
    for storyboard in of_kind(pb::EntityKind::Storyboard) {
        let mut slices = Vec::new();
        for r in refs_of(storyboard)
            .iter()
            .filter(|r| r.from_field.starts_with("slices["))
        {
            let found = SLICE_KINDS
                .iter()
                .map(|k| pb::EntityKey::new(*k, &r.to_id))
                .find(|k| index.contains_key(k));
            slices.push(match found {
                Some(key) => {
                    placed.insert((key.kind, key.namespace.clone(), key.slug.clone()));
                    slice_node(&key)
                }
                None => TreeNode {
                    kind: "slice",
                    namespace: r.to_id.namespace.clone(),
                    slug: r.to_id.slug.clone(),
                    version: r.to_id.version,
                    title: "(not found)".into(),
                    children: vec![],
                },
            });
        }
        storyboards.push(node(storyboard, slices));
    }
    let unplaced_slices = latest
        .iter()
        .filter(|k| {
            SLICE_KINDS.contains(&k.kind)
                && !placed.contains(&(k.kind, k.namespace.clone(), k.slug.clone()))
        })
        .map(|k| slice_node(k))
        .collect();
    NamespaceTree {
        namespace: namespace.to_string(),
        domains,
        storyboards,
        unplaced_slices,
    }
}

pub fn render_tree(tree: &NamespaceTree) -> String {
    let ns = tree.namespace.as_str();
    let mut top: Vec<(String, &[TreeNode])> = tree
        .domains
        .iter()
        .chain(&tree.storyboards)
        .map(|n| (tree_label(n, ns), n.children.as_slice()))
        .collect();
    if !tree.unplaced_slices.is_empty() {
        top.push(("(not in a storyboard)".into(), &tree.unplaced_slices));
    }
    let mut out = format!("{ns}\n");
    push_branches(&mut out, "", &top, ns);
    out
}

fn push_branches(out: &mut String, prefix: &str, items: &[(String, &[TreeNode])], ns: &str) {
    for (i, (label, children)) in items.iter().enumerate() {
        let last = i + 1 == items.len();
        let _ = writeln!(out, "{prefix}{}{label}", if last { "└── " } else { "├── " });
        let nested: Vec<_> = children
            .iter()
            .map(|c| (tree_label(c, ns), c.children.as_slice()))
            .collect();
        let indent = format!("{prefix}{}", if last { "    " } else { "│   " });
        push_branches(out, &indent, &nested, ns);
    }
}

/// Namespace and version only appear when they differ from the tree's
/// namespace and from 1, which keeps the common case short.
fn tree_label(n: &TreeNode, ns: &str) -> String {
    let mut label = format!("{} ", n.kind);
    if n.namespace != ns {
        let _ = write!(label, "{}/", n.namespace);
    }
    label.push_str(&n.slug);
    if n.version != 1 {
        let _ = write!(label, "@{}", n.version);
    }
    if !n.title.is_empty() {
        let _ = write!(label, "  {}", n.title);
    }
    label
}

fn ref_label(r: Option<&pb::EntityRef>) -> String {
    let id = r.and_then(|r| r.id.clone()).unwrap_or_default();
    match r.and_then(|r| pb::EntityKind::try_from(r.kind).ok()) {
        Some(kind) if kind != pb::EntityKind::Unspecified => pb::canonical::id_string(kind, &id),
        _ => format!("{}/{}@{}", id.namespace, id.slug, id.version),
    }
}

/// Reads a string field of whichever kind the entity holds by reflection, so
/// one function covers every kind. `Any` payloads stay undecoded, so tenant
/// types are not needed.
fn text_field(entity: &pb::Entity, name: &str) -> String {
    let Ok(desc) = trogon_atlas_core::transcode::entity_descriptor() else {
        return String::new();
    };
    let Ok(message) = DynamicMessage::decode(desc, entity.encode_to_vec().as_slice()) else {
        return String::new();
    };
    let text = message
        .fields()
        .filter(|(field, _)| field.containing_oneof().is_some())
        .find_map(|(_, value)| {
            let field = value.as_message()?.get_field_by_name(name)?;
            field.as_str().map(str::to_owned).or_else(|| {
                field
                    .as_message()?
                    .get_field_by_name("value")?
                    .as_str()
                    .map(str::to_owned)
            })
        });
    text.unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use trogon_atlas_core::schema::pack_fields;

    use super::*;

    const NS: &str = "shop";

    fn builtin() -> TranscodePool {
        TranscodePool::builtin().unwrap()
    }

    fn id(slug: &str, version: u64) -> pb::Id {
        pb::Id {
            namespace: NS.into(),
            slug: slug.into(),
            version,
        }
    }

    fn entity(kind: pb::entity::Kind) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(kind),
        }
    }

    fn field(name: &str, kind: pb::field_type::Kind, repeated: bool) -> pb::FieldSpec {
        pb::FieldSpec {
            name: name.into(),
            r#type: Some(pb::FieldType { kind: Some(kind) }),
            repeated,
            ..Default::default()
        }
    }

    fn reference(
        from: (pb::EntityKind, &str),
        field: &str,
        to: (pb::EntityKind, &str),
    ) -> pb::Reference {
        pb::Reference {
            from: Some(pb::EntityRef {
                kind: from.0 as i32,
                id: Some(id(from.1, 1)),
            }),
            from_field: field.into(),
            to: Some(pb::EntityRef {
                kind: to.0 as i32,
                id: Some(id(to.1, 1)),
            }),
            ..Default::default()
        }
    }

    fn event(slug: &str, version: u64, title: &str) -> pb::Entity {
        entity(pb::entity::Kind::Event(pb::Event {
            id: Some(id(slug, version)),
            title: title.into(),
            ..Default::default()
        }))
    }

    fn command_slice(slug: &str, command: &str, events: &[&str]) -> pb::Entity {
        entity(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(slug, 1)),
            title: format!("{slug} title"),
            command: Some(pb::CommandEdge {
                command: Some(pb::CommandRef {
                    id: Some(id(command, 1)),
                }),
                ..Default::default()
            }),
            emitted_events: events
                .iter()
                .map(|e| pb::EventEdge {
                    event: Some(pb::EventRef { id: Some(id(e, 1)) }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }))
    }

    #[test]
    fn parse_address_defaults_to_latest_and_accepts_a_pinned_version() {
        let latest = parse_address("read-model/shop/cart-summary").unwrap();
        assert_eq!(latest.kind, pb::EntityKind::ReadModel);
        assert_eq!(
            (latest.namespace.as_str(), latest.slug.as_str()),
            ("shop", "cart-summary")
        );
        assert_eq!(latest.version, None);
        let pinned = parse_address("event/shop/order.placed@2").unwrap();
        assert_eq!(pinned.version, Some(2));
    }

    #[test]
    fn parse_address_accepts_the_id_describe_prints() {
        let printed = pb::canonical::id_string(pb::EntityKind::CommandSlice, &id("place-order", 3));
        let parsed = parse_address(&printed).unwrap();
        assert_eq!(
            parsed,
            parse_address("command_slice/shop/place-order@3").unwrap()
        );
        assert_eq!(parse_address("event:shop/x").unwrap().version, None);
    }

    #[test]
    fn parse_address_rejects_malformed_input_as_validation() {
        for (input, needle) in [
            ("event/shop", "expected <kind>/<namespace>/<slug>"),
            ("event:shop", "expected <kind>/<namespace>/<slug>"),
            ("widget/shop/x", "unknown entity kind: widget"),
            ("event/shop/x@0", "version must be a positive integer"),
            ("event/shop/x@v2", "version must be a positive integer"),
            ("event/shop/a/b", "slug contains invalid character '/'"),
            ("event//x", "namespace must not be empty"),
        ] {
            let err = parse_address(input).unwrap_err();
            assert!(err.to_string().contains(needle), "{input}: {err}");
            let failure = crate::failure::Failure::classify(&err);
            assert_eq!(
                failure.category,
                crate::failure::FailureCategory::Validation,
                "{input}"
            );
        }
    }

    #[test]
    fn describe_renders_fields_and_both_reference_directions() {
        let mut placed = event("order.placed", 1, "Order placed");
        if let Some(pb::entity::Kind::Event(e)) = placed.kind.as_mut() {
            e.doc = "A shopper placed an order.\nPayment follows.".into();
            e.schema = Some(pack_fields(vec![
                field(
                    "order_id",
                    pb::field_type::Kind::Uuid(pb::field_type::UuidType::default()),
                    false,
                ),
                field(
                    "line_items",
                    pb::field_type::Kind::String(pb::field_type::StringType {}),
                    true,
                ),
            ]));
        }
        let outgoing = [reference(
            (pb::EntityKind::Event, "order.placed"),
            "swimlane",
            (pb::EntityKind::Swimlane, "order"),
        )];
        let incoming = [
            reference(
                (pb::EntityKind::ReadModelSlice, "status-rms"),
                "source_events[0]",
                (pb::EntityKind::Event, "order.placed"),
            ),
            reference(
                (pb::EntityKind::CommandSlice, "place-order-slice"),
                "emitted_events[0]",
                (pb::EntityKind::Event, "order.placed"),
            ),
        ];
        let d = describe(&placed, &builtin(), &outgoing, &incoming).unwrap();
        assert_eq!(
            render_description(&d),
            "\
Id:     event:shop/order.placed@1
Title:  Order placed
Doc:    A shopper placed an order.
        Payment follows.
Fields:
  order_id    uuid
  line_items  string[]
References:
  swimlane  swimlane:shop/order@1
Referenced by:
  command_slice:shop/place-order-slice@1  emitted_events[0]
  read_model_slice:shop/status-rms@1      source_events[0]
"
        );
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(
            json["fields"][1],
            serde_json::json!({"name": "line_items", "type": "string[]"})
        );
    }

    #[test]
    fn describe_resolves_tenant_schema_fields_through_the_given_types() {
        use prost_types::{
            field_descriptor_proto::Type, DescriptorProto, FieldDescriptorProto,
            FileDescriptorProto,
        };
        let file = FileDescriptorProto {
            name: Some("shop/orders/v1/placed.proto".into()),
            package: Some("shop.orders.v1".into()),
            syntax: Some("proto3".into()),
            message_type: vec![DescriptorProto {
                name: Some("OrderPlaced".into()),
                field: vec![FieldDescriptorProto {
                    name: Some("order_id".into()),
                    json_name: Some("orderId".into()),
                    number: Some(1),
                    r#type: Some(Type::String as i32),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut placed = event("order.placed", 1, "Order placed");
        if let Some(pb::entity::Kind::Event(e)) = placed.kind.as_mut() {
            e.schema = Some(prost_types::Any {
                type_url: "type.googleapis.com/shop.orders.v1.OrderPlaced".into(),
                value: vec![],
            });
        }
        let tenant = builtin().with_files([file]).unwrap();
        let fields = describe(&placed, &tenant, &[], &[])
            .unwrap()
            .fields
            .unwrap();
        assert_eq!(
            fields,
            vec![FieldLine {
                name: "order_id".into(),
                field_type: "string".into(),
            }]
        );
        assert_eq!(
            describe(&placed, &builtin(), &[], &[]).unwrap().fields,
            Some(vec![])
        );
    }

    #[test]
    fn describe_omits_fields_for_kinds_without_a_schema() {
        let slice = command_slice("place-order-slice", "place-order", &[]);
        let d = describe(&slice, &builtin(), &[], &[]).unwrap();
        assert_eq!(d.fields, None);
        assert!(serde_json::to_value(&d).unwrap().get("fields").is_none());
        assert!(
            render_description(&d).ends_with("References:\n  <none>\nReferenced by:\n  <none>\n")
        );
    }

    #[test]
    fn namespaces_render_as_a_name_id_and_count_table() {
        let ns = |name: &str, id: &str, entity_count| pb::list_namespaces_response::Namespace {
            name: name.into(),
            id: id.into(),
            entity_count,
            ..Default::default()
        };
        assert_eq!(
            render_namespaces(&[ns("acme-reviews", "acme-reviews", 35), ns("shop", "ns_1", 0)]),
            "NAME          ID            ENTITIES\nacme-reviews  acme-reviews  35\nshop          ns_1          0\n"
        );
        assert_eq!(render_namespaces(&[]), "no namespaces\n");
        let json = namespaces_json(&[ns("shop", "ns_1", 2)]);
        assert_eq!(
            json,
            serde_json::json!({"namespaces": [{"id": "ns_1", "name": "shop", "entity_count": 2, "parent": ""}]})
        );
    }

    #[test]
    fn tree_groups_by_domain_and_storyboard_and_keeps_unplaced_slices() {
        let domain = entity(pb::entity::Kind::Domain(pb::Domain {
            id: Some(id("retail", 1)),
            title: "Retail".into(),
            ..Default::default()
        }));
        let subdomain = entity(pb::entity::Kind::Subdomain(pb::Subdomain {
            id: Some(id("ordering", 1)),
            title: "Ordering".into(),
            domain: Some(pb::DomainRef {
                id: Some(id("retail", 1)),
            }),
            ..Default::default()
        }));
        let storyboard = entity(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id("checkout", 1)),
            title: "Checkout".into(),
            slices: vec![
                pb::SliceRef {
                    id: Some(id("place-order-slice", 1)),
                },
                pb::SliceRef {
                    id: Some(id("gone-slice", 1)),
                },
            ],
            ..Default::default()
        }));
        let mut placed_v2 = event("order.placed", 2, "Order placed v2");
        if let Some(pb::entity::Kind::Event(e)) = placed_v2.kind.as_mut() {
            e.supersedes = Some(id("order.placed", 1));
        }
        let entities = vec![
            storyboard,
            command_slice(
                "place-order-slice",
                "place-order",
                &["order.placed", "order.placed"],
            ),
            command_slice("refund-slice", "refund", &[]),
            event("order.placed", 1, "Order placed"),
            placed_v2,
            subdomain,
            domain,
        ];
        let tree = namespace_tree(NS, &entities);
        assert_eq!(
            render_tree(&tree),
            "\
shop
├── domain retail  Retail
│   └── subdomain ordering  Ordering
├── storyboard checkout  Checkout
│   ├── command_slice place-order-slice  place-order-slice title
│   │   ├── command place-order
│   │   └── event order.placed  Order placed
│   └── slice gone-slice  (not found)
└── (not in a storyboard)
    └── command_slice refund-slice  refund-slice title
        └── command refund
"
        );
        let json = serde_json::to_value(&tree).unwrap();
        assert_eq!(
            json["storyboards"][0]["children"][0]["children"][1]["version"],
            1
        );
        assert!(json["domains"][0]["children"][0].get("children").is_none());
    }

    #[test]
    fn tree_attaches_children_that_pin_an_older_version_of_their_parent() {
        let domain = |version| {
            entity(pb::entity::Kind::Domain(pb::Domain {
                id: Some(id("retail", version)),
                title: format!("Retail v{version}"),
                ..Default::default()
            }))
        };
        let subdomain = entity(pb::entity::Kind::Subdomain(pb::Subdomain {
            id: Some(id("ordering", 1)),
            title: "Ordering".into(),
            domain: Some(pb::DomainRef {
                id: Some(id("retail", 1)),
            }),
            ..Default::default()
        }));
        let storyboard = entity(pb::entity::Kind::Storyboard(pb::Storyboard {
            id: Some(id("checkout", 1)),
            title: "Checkout".into(),
            slices: vec![pb::SliceRef {
                id: Some(id("place-order-slice", 1)),
            }],
            ..Default::default()
        }));
        let mut slice_v2 = command_slice("place-order-slice", "place-order", &[]);
        if let Some(pb::entity::Kind::CommandSlice(s)) = slice_v2.kind.as_mut() {
            s.id = Some(id("place-order-slice", 2));
        }
        let entities = vec![
            domain(1),
            domain(2),
            subdomain,
            storyboard,
            command_slice("place-order-slice", "place-order", &[]),
            slice_v2,
        ];
        assert_eq!(
            render_tree(&namespace_tree(NS, &entities)),
            "\
shop
├── domain retail@2  Retail v2
│   └── subdomain ordering  Ordering
└── storyboard checkout  Checkout
    └── command_slice place-order-slice  place-order-slice title
        └── command place-order
"
        );
    }
}
