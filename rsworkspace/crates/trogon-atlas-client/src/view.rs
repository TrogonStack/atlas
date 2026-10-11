//! Read-only views of one entity. The
//! builders and renderers are pure functions over the gRPC response types,
//! so the CLI's text output and any later terminal UI show the same thing.

use std::fmt::Write as _;

use anyhow::{Context as _, Result};
use prost::Message as _;
use prost_reflect::DynamicMessage;
use serde::Serialize;
use serde_json::Value;
use trogon_atlas_core::{entity_id, entity_kind, schema::entity_fields};
use trogon_atlas_proto as pb;

use crate::{client::Client, export, failure::invalid_input};

/// `<kind>/<namespace>/<slug>[@version]`; `version: None` means latest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityAddress {
    pub kind: pb::EntityKind,
    pub namespace: String,
    pub slug: String,
    pub version: Option<u64>,
}

pub fn parse_address(s: &str) -> Result<EntityAddress> {
    let mut parts = s.splitn(3, '/');
    let (Some(kind), Some(namespace), Some(rest)) = (parts.next(), parts.next(), parts.next())
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

/// The entity as the same manifest document `export` writes, both as JSON
/// and as rendered YAML.
pub async fn entity_manifest(client: &mut Client, entity: &pb::Entity) -> Result<(Value, String)> {
    let namespace = entity_id(entity)
        .map(|id| id.namespace.clone())
        .unwrap_or_default();
    let types = export::namespace_types(client, &namespace).await?;
    let (_, _, doc) = export::entity_to_manifest(entity, &types)?;
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
}

pub fn describe(
    entity: &pb::Entity,
    outgoing: &[pb::Reference],
    incoming: &[pb::Reference],
) -> Description {
    let has_schema = matches!(
        entity_kind(entity),
        Some(pb::EntityKind::Event | pb::EntityKind::Command | pb::EntityKind::ReadModel)
    );
    let fields = has_schema.then(|| {
        entity_fields(entity)
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
    let line = |r: &pb::Reference, other: Option<&pb::EntityRef>| RefLine {
        field: r.from_field.clone(),
        entity: ref_label(other),
    };
    let mut incoming: Vec<RefLine> = incoming.iter().map(|r| line(r, r.from.as_ref())).collect();
    incoming.sort_by(|a, b| (&a.entity, &a.field).cmp(&(&b.entity, &b.field)));
    Description {
        id: export::entity_label(entity),
        title: text_field(entity, "title"),
        doc: text_field(entity, "doc"),
        fields,
        outgoing: outgoing.iter().map(|r| line(r, r.to.as_ref())).collect(),
        incoming,
    }
}

pub fn render_description(d: &Description) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Id:     {}", d.id);
    let _ = writeln!(out, "Title:  {}", d.title);
    let _ = writeln!(out, "Doc:    {}", d.doc.replace('\n', "\n        "));
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
            value
                .as_message()?
                .get_field_by_name(name)?
                .as_str()
                .map(str::to_owned)
        });
    text.unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use trogon_atlas_core::schema::pack_fields;

    use super::*;

    const NS: &str = "shop";

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
    fn parse_address_rejects_malformed_input_as_validation() {
        for (input, needle) in [
            ("event/shop", "expected <kind>/<namespace>/<slug>"),
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
        let d = describe(&placed, &outgoing, &incoming);
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
    fn describe_omits_fields_for_kinds_without_a_schema() {
        let slice = command_slice("place-order-slice", "place-order", &[]);
        let d = describe(&slice, &[], &[]);
        assert_eq!(d.fields, None);
        assert!(serde_json::to_value(&d).unwrap().get("fields").is_none());
        assert!(
            render_description(&d).ends_with("References:\n  <none>\nReferenced by:\n  <none>\n")
        );
    }
}
