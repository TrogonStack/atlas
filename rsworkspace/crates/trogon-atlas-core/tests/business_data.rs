#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_core::{
    business_data::{self, MAX_SCHEMA_BYTES},
    connection_persistence::prepare_entity,
    connections::{self, connection_occurrences},
    refs::{outbound_refs, retarget_refs},
    schema, source_integration, transcode,
};
use trogon_atlas_proto::{
    self as pb, entity, field_type, source_field_ownership, source_selection, BusinessIdentifier,
    BusinessIdentifierRole, BusinessPropertyRelationship, CatalogText, CatalogTitle, ConnectionId,
    ConnectionRef, Entity, EntityKind, EntityRef, Event, FieldSpec, FieldType, Id, PropertyPath,
    PropertySegment, Schema, SchemaRef, SourceFieldOwnership, SourceOwnedField, SourceOwnedFields,
    SourceSelection, SourceWholeOwnerOwnership,
};

const KEPT: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

fn id(slug: &str, version: u64) -> Id {
    Id {
        namespace: "shop".into(),
        slug: slug.into(),
        version,
    }
}

fn segment(name: &str, repeated: bool) -> PropertySegment {
    PropertySegment {
        name: name.into(),
        repeated,
    }
}

fn path(segments: &[(&str, bool)]) -> PropertyPath {
    PropertyPath {
        segments: segments
            .iter()
            .map(|(name, repeated)| segment(name, *repeated))
            .collect(),
    }
}

fn string_field(name: &str, repeated: bool) -> FieldSpec {
    FieldSpec {
        name: name.into(),
        repeated,
        r#type: Some(FieldType {
            kind: Some(field_type::Kind::String(field_type::StringType {})),
        }),
        ..Default::default()
    }
}

fn object_field(name: &str, repeated: bool, fields: Vec<FieldSpec>) -> FieldSpec {
    FieldSpec {
        name: name.into(),
        repeated,
        r#type: Some(FieldType {
            kind: Some(field_type::Kind::Object(field_type::ObjectType { fields })),
        }),
        ..Default::default()
    }
}

fn schema_ref_field(name: &str, target: Id) -> FieldSpec {
    FieldSpec {
        name: name.into(),
        r#type: Some(FieldType {
            kind: Some(field_type::Kind::Ref(field_type::RefType {
                r#ref: Some(pb::TypeRef {
                    r#ref: Some(pb::type_ref::Ref::Schema(SchemaRef { id: Some(target) })),
                }),
            })),
        }),
        ..Default::default()
    }
}

fn order_fields(product: Id) -> Vec<FieldSpec> {
    vec![
        object_field("customer", false, vec![string_field("id", false)]),
        object_field("lines", true, vec![string_field("sku", false)]),
        schema_ref_field("product_ref", product),
    ]
}

fn order_schema(product: Id) -> Schema {
    Schema {
        id: Some(id("order", 1)),
        title: "Order".into(),
        fields: order_fields(product),
        ..Default::default()
    }
}

fn entity_of(schema: Schema) -> Entity {
    Entity {
        kind: Some(entity::Kind::Schema(schema)),
        ..Default::default()
    }
}

fn identifier(
    property: PropertyPath,
    role: BusinessIdentifierRole,
    connection: Option<&str>,
) -> BusinessIdentifier {
    BusinessIdentifier {
        connection_id: connection.map(|value| ConnectionId {
            value: value.into(),
        }),
        property: Some(property),
        role: role as i32,
        doc: None,
    }
}

fn relationship(
    source: PropertyPath,
    target: Id,
    target_property: Option<PropertyPath>,
    label: Option<&str>,
    connection: Option<&str>,
) -> BusinessPropertyRelationship {
    BusinessPropertyRelationship {
        connection_id: connection.map(|value| ConnectionId {
            value: value.into(),
        }),
        source: Some(source),
        target_entity: Some(SchemaRef { id: Some(target) }),
        target_property,
        label: label.map(|value| CatalogTitle {
            value: value.into(),
        }),
        doc: None,
    }
}

#[test]
fn primary_customer_id_admits_and_a_second_primary_rejects() {
    let product = id("product", 1);
    let mut schema = order_schema(product);
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        None,
    ));
    business_data::validate_entity(&entity_of(schema.clone())).expect("one primary");
    schema.identifiers.push(identifier(
        path(&[("lines", true), ("sku", false)]),
        BusinessIdentifierRole::Primary,
        None,
    ));
    let error = business_data::validate_entity(&entity_of(schema)).unwrap_err();
    assert!(error.to_string().contains("one primary"), "{error}");
}

#[test]
fn alternate_admits_and_role_zero_same_path_and_empty_id_reject() {
    let product = id("product", 1);
    let mut schema = order_schema(product);
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        Some(KEPT),
    ));
    schema.identifiers.push(identifier(
        path(&[("lines", true), ("sku", false)]),
        BusinessIdentifierRole::Alternate,
        Some(OTHER),
    ));
    business_data::validate_entity(&entity_of(schema.clone())).expect("primary and alternate");

    schema.identifiers[1].role = BusinessIdentifierRole::Unspecified as i32;
    assert!(business_data::validate_entity(&entity_of(schema.clone())).is_err());

    schema.identifiers[1].role = BusinessIdentifierRole::Alternate as i32;
    schema.identifiers[1].property = schema.identifiers[0].property.clone();
    assert!(business_data::validate_entity(&entity_of(schema.clone())).is_err());

    schema.identifiers[1].property = Some(path(&[("lines", true), ("sku", false)]));
    schema.identifiers[1].connection_id = Some(ConnectionId {
        value: String::new(),
    });
    assert!(business_data::validate_entity(&entity_of(schema.clone())).is_err());

    schema.identifiers[1].connection_id = Some(ConnectionId {
        value: "not-a-uuid".into(),
    });
    assert!(business_data::validate_entity(&entity_of(schema)).is_err());
}

#[test]
fn repeated_flag_object_leaf_and_ref_walk_follow_the_field_spec() {
    let product = id("product", 1);
    let fields = order_fields(product);
    assert_eq!(
        business_data::resolve_path(&fields, &path(&[("lines", true), ("sku", false)]))
            .unwrap()
            .name,
        "sku"
    );
    assert!(
        business_data::resolve_path(&fields, &path(&[("lines", false), ("sku", false)])).is_err()
    );
    assert!(business_data::resolve_path(
        &fields,
        &path(&[("lines", true), ("items", false), ("sku", false)])
    )
    .is_err());
    assert!(
        business_data::resolve_path(&fields, &path(&[("product_ref", false), ("sku", false)]))
            .is_err()
    );
    let mut ambiguous = fields.clone();
    ambiguous.push(string_field("sku", false));
    ambiguous.push(string_field("sku", false));
    assert!(business_data::resolve_path(&ambiguous, &path(&[("sku", false)])).is_err());
    assert!(
        business_data::resolve_path(&ambiguous, &path(&[("customer", false), ("id", false)]))
            .is_ok()
    );
}

#[test]
fn relationship_label_and_target_shape_are_independent_of_identifier_roles() {
    let product = id("product", 1);
    let mut schema = order_schema(product.clone());
    schema.property_relationships.push(relationship(
        path(&[("product_ref", false)]),
        product.clone(),
        None,
        None,
        None,
    ));
    business_data::validate_entity(&entity_of(schema.clone())).expect("entity only");
    schema.property_relationships[0].target_property = Some(path(&[("sku", false)]));
    schema.property_relationships[0].label = Some(CatalogTitle {
        value: "hasMany".into(),
    });
    business_data::validate_entity(&entity_of(schema.clone())).expect("target path shape");
    schema.property_relationships.push(relationship(
        path(&[("product_ref", false)]),
        product,
        Some(path(&[("sku", false)])),
        Some("belongsTo"),
        Some(OTHER),
    ));
    schema.property_relationships[0].connection_id = Some(ConnectionId { value: KEPT.into() });
    business_data::validate_entity(&entity_of(schema.clone())).expect("two occurrences");
    schema.property_relationships[0].label = Some(CatalogTitle { value: "  ".into() });
    assert!(business_data::validate_entity(&entity_of(schema.clone())).is_err());
    schema.property_relationships[0].label = None;
    schema.property_relationships[0].doc = Some(CatalogText { value: " ".into() });
    assert!(business_data::validate_entity(&entity_of(schema)).is_err());
}

#[test]
fn payload_schema_rejects_business_declarations_and_admits_an_id_only_body() {
    let mut payload = Schema {
        id: Some(id("order", 1)),
        ..Default::default()
    };
    let event = Entity {
        kind: Some(entity::Kind::Event(Event {
            id: Some(id("order-placed", 1)),
            title: "Order placed".into(),
            schema: Some(schema::pack_schema(&payload)),
            ..Default::default()
        })),
        ..Default::default()
    };
    business_data::validate_entity(&event).expect("id only payload");
    payload.aggregate_root = Some(true);
    let rejected = Entity {
        kind: Some(entity::Kind::Event(Event {
            id: Some(id("order-placed", 1)),
            schema: Some(schema::pack_schema(&payload)),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert!(business_data::validate_entity(&rejected).is_err());
}

#[test]
fn omitted_ids_are_assigned_and_wording_keeps_them() {
    let product = id("product", 1);
    let mut schema = order_schema(product.clone());
    schema.aggregate_root = Some(true);
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        None,
    ));
    schema.property_relationships.push(relationship(
        path(&[("product_ref", false)]),
        product,
        Some(path(&[("sku", false)])),
        Some("hasMany"),
        None,
    ));
    let prepared = prepare_entity(None, &entity_of(schema)).unwrap();
    let occurrences = connection_occurrences(&prepared).unwrap();
    assert_eq!(occurrences.len(), 2);
    assert!(occurrences.iter().all(|item| item.id.is_some()));
    assert!(occurrences.iter().any(|item| item.target.is_none()));
    assert!(occurrences.iter().any(|item| {
        item.target
            .as_ref()
            .and_then(|target| target.id.as_ref())
            .map(|id| id.slug.as_str())
            == Some("product")
    }));
    let mut wording = prepared.clone();
    if let Some(entity::Kind::Schema(schema)) = &mut wording.kind {
        schema.identifiers[0].doc = Some(CatalogText {
            value: "Customer id".into(),
        });
        schema.property_relationships[0].label = Some(CatalogTitle {
            value: "belongsTo".into(),
        });
    }
    let kept = prepare_entity(Some(&prepared), &wording).unwrap();
    let before = connection_occurrences(&prepared).unwrap();
    let after = connection_occurrences(&kept).unwrap();
    assert_eq!(
        before.iter().filter_map(|item| item.id).collect::<Vec<_>>(),
        after.iter().filter_map(|item| item.id).collect::<Vec<_>>()
    );
}

#[test]
fn role_or_target_change_with_a_kept_id_rejects_and_omission_assigns_a_new_one() {
    let product = id("product", 1);
    let mut schema = order_schema(product.clone());
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        None,
    ));
    schema.property_relationships.push(relationship(
        path(&[("product_ref", false)]),
        product,
        None,
        Some("hasMany"),
        None,
    ));
    let prepared = prepare_entity(None, &entity_of(schema)).unwrap();
    let mut changed = prepared.clone();
    if let Some(entity::Kind::Schema(schema)) = &mut changed.kind {
        schema.identifiers[0].role = BusinessIdentifierRole::Alternate as i32;
    }
    assert!(prepare_entity(Some(&prepared), &changed).is_err());
    let mut retargeted = prepared.clone();
    if let Some(entity::Kind::Schema(schema)) = &mut retargeted.kind {
        schema.property_relationships[0].target_entity = Some(SchemaRef {
            id: Some(id("product", 2)),
        });
    }
    assert!(prepare_entity(Some(&prepared), &retargeted).is_err());
    if let Some(entity::Kind::Schema(schema)) = &mut retargeted.kind {
        schema.property_relationships[0].connection_id = None;
    }
    let renewed = prepare_entity(Some(&prepared), &retargeted).unwrap();
    let old = connection_occurrences(&prepared)
        .unwrap()
        .into_iter()
        .find(|item| item.target.is_some())
        .unwrap()
        .id;
    let new = connection_occurrences(&renewed)
        .unwrap()
        .into_iter()
        .find(|item| item.target.is_some())
        .unwrap()
        .id;
    assert_ne!(old, new);
}

#[test]
fn generic_retarget_moves_the_field_ref_and_the_relationship_and_renews_only_that_id() {
    let from = id("product", 1);
    let to = id("product", 2);
    let mut schema = order_schema(from.clone());
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        None,
    ));
    schema.property_relationships.push(relationship(
        path(&[("product_ref", false)]),
        from.clone(),
        Some(path(&[("sku", false)])),
        Some("hasMany"),
        None,
    ));
    let prepared = prepare_entity(None, &entity_of(schema)).unwrap();
    let identifier_id = connection_occurrences(&prepared)
        .unwrap()
        .into_iter()
        .find(|item| item.target.is_none())
        .unwrap()
        .id;
    let mut retargeted = prepared.clone();
    let rewritten = retarget_refs(&mut retargeted, EntityKind::Schema, &from, &to);
    assert!(rewritten
        .iter()
        .any(|field| field.contains("product_ref") || field.contains("fields[2]")));
    assert!(rewritten
        .iter()
        .any(|field| field == "property_relationships[0].target_entity"));
    connections::renew_retargeted_connection_ids(&prepared, &mut retargeted).unwrap();
    let schema = match &retargeted.kind {
        Some(entity::Kind::Schema(schema)) => schema,
        _ => panic!("schema"),
    };
    let field_target = match &schema.fields[2].r#type.as_ref().unwrap().kind {
        Some(field_type::Kind::Ref(value)) => match &value.r#ref.as_ref().unwrap().r#ref {
            Some(pb::type_ref::Ref::Schema(schema_ref)) => schema_ref.id.clone().unwrap(),
            other => panic!("schema ref, got {other:?}"),
        },
        other => panic!("ref field, got {other:?}"),
    };
    assert_eq!(field_target, to);
    assert_eq!(
        schema.property_relationships[0]
            .target_entity
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap(),
        &to
    );
    assert!(schema.property_relationships[0].connection_id.is_none());
    assert_eq!(
        schema.identifiers[0]
            .connection_id
            .as_ref()
            .map(|value| value.value.clone()),
        identifier_id.as_ref().map(ToString::to_string)
    );
    let refs = outbound_refs(&retargeted);
    assert!(refs
        .iter()
        .any(|item| item.from_field.starts_with("fields[") && item.to_id == to));
    assert!(refs.iter().any(
        |item| item.from_field == "property_relationships[0].target_entity" && item.to_id == to
    ));
    assert!(
        refs.iter()
            .position(|item| item.from_field.starts_with("fields["))
            < refs
                .iter()
                .position(|item| item.from_field.starts_with("property_relationships"))
    );
}

#[test]
fn local_source_paths_reject_when_broken_and_ignore_a_dangling_target_property() {
    let product = id("product", 1);
    let mut schema = order_schema(product.clone());
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        Some(KEPT),
    ));
    schema.property_relationships.push(relationship(
        path(&[("product_ref", false)]),
        product,
        Some(path(&[("missing", false)])),
        Some("hasMany"),
        Some(OTHER),
    ));
    let entity = entity_of(schema);
    business_data::reject_source_local_paths(&entity).expect("target property is not a local path");
    let mut broken = entity.clone();
    if let Some(entity::Kind::Schema(schema)) = &mut broken.kind {
        schema.fields.retain(|field| field.name != "customer");
    }
    assert!(business_data::reject_source_local_paths(&broken).is_err());
}

#[test]
fn source_cannot_own_business_declarations_or_select_their_occurrences() {
    let product = id("product", 1);
    let mut schema = order_schema(product.clone());
    schema.aggregate_root = Some(false);
    schema.identifiers.push(identifier(
        path(&[("customer", false), ("id", false)]),
        BusinessIdentifierRole::Primary,
        Some(KEPT),
    ));
    let entity = entity_of(schema);
    let whole = SourceFieldOwnership {
        policy: Some(source_field_ownership::Policy::WholeOwner(
            SourceWholeOwnerOwnership {},
        )),
    };
    assert_eq!(
        source_integration::validate_ownership(&entity, &whole),
        Err(source_integration::SourceError::UnsupportedUnit)
    );
    let selection = SourceSelection {
        selection: Some(source_selection::Selection::Connection(ConnectionRef {
            owner: Some(EntityRef {
                kind: EntityKind::Schema as i32,
                id: Some(id("order", 1)),
            }),
            id: Some(ConnectionId { value: KEPT.into() }),
            owner_uid: "owner".into(),
        })),
    };
    assert_eq!(
        source_integration::validate_selection_ownership(
            &selection,
            &SourceFieldOwnership {
                policy: Some(source_field_ownership::Policy::None(
                    pb::SourceNoOwnership {}
                )),
            }
        ),
        Err(source_integration::SourceError::UnsupportedUnit)
    );
    let mut incoming = entity.clone();
    if let Some(entity::Kind::Schema(schema)) = &mut incoming.kind {
        schema.title = "Renamed".into();
        schema.aggregate_root = None;
        schema.identifiers.clear();
    }
    let plan = source_integration::plan_merge(
        &entity,
        &entity,
        &incoming,
        &entity,
        &SourceFieldOwnership {
            policy: Some(source_field_ownership::Policy::Fields(SourceOwnedFields {
                fields: vec![SourceOwnedField::Title as i32],
            })),
        },
        &mut source_integration::RunBudget::new(source_integration::profile_budgets()).unwrap(),
    )
    .unwrap();
    let proposed = match plan.proposed.kind {
        Some(entity::Kind::Schema(schema)) => schema,
        _ => panic!("schema"),
    };
    assert_eq!(proposed.title, "Renamed");
    assert_eq!(proposed.aggregate_root, Some(false));
    assert_eq!(proposed.identifiers.len(), 1);
}

#[test]
fn bounds_reject_the_next_identifier_segment_name_and_encoded_schema() {
    let mut schema = Schema {
        id: Some(id("wide", 1)),
        fields: (0..1_025)
            .map(|index| string_field(&format!("f{index}"), false))
            .collect(),
        ..Default::default()
    };
    schema.identifiers = (0..1_024)
        .map(|index| {
            identifier(
                path(&[(&format!("f{index}"), false)]),
                if index == 0 {
                    BusinessIdentifierRole::Primary
                } else {
                    BusinessIdentifierRole::Alternate
                },
                None,
            )
        })
        .collect();
    business_data::validate_entity(&entity_of(schema.clone())).expect("1024 identifiers");
    schema.identifiers.push(identifier(
        path(&[("f1024", false)]),
        BusinessIdentifierRole::Alternate,
        None,
    ));
    assert!(business_data::validate_entity(&entity_of(schema)).is_err());

    fn nest(depth: usize) -> FieldSpec {
        if depth == 0 {
            string_field("leaf", false)
        } else {
            object_field(&format!("n{depth}"), false, vec![nest(depth - 1)])
        }
    }
    fn deep_path(depth: usize) -> PropertyPath {
        let mut segments = Vec::new();
        for index in (1..=depth).rev() {
            segments.push(segment(&format!("n{index}"), false));
        }
        segments.push(segment("leaf", false));
        PropertyPath { segments }
    }
    assert!(business_data::resolve_path(&[nest(31)], &deep_path(31)).is_ok());
    assert!(business_data::resolve_path(&[nest(32)], &deep_path(32)).is_err());

    let wide_name = "a".repeat(256);
    let fields = vec![string_field(&wide_name, false)];
    assert!(business_data::resolve_path(&fields, &path(&[(&wide_name, false)])).is_ok());
    let too_wide = format!("{wide_name}b");
    let fields = vec![string_field(&too_wide, false)];
    assert!(business_data::resolve_path(&fields, &path(&[(&too_wide, false)])).is_err());

    let mut huge = order_schema(id("product", 1));
    huge.fields[0].doc = "x".repeat(MAX_SCHEMA_BYTES);
    assert!(prepare_entity(None, &entity_of(huge)).is_err());
}

#[test]
fn optional_aggregate_root_and_large_versions_round_trip_in_json() {
    let present = Schema {
        id: Some(id("order", 9_007_199_254_740_993)),
        aggregate_root: Some(false),
        ..Default::default()
    };
    let json = transcode::message_json_value(schema::SCHEMA_MESSAGE, &present).unwrap();
    assert_eq!(json["aggregateRoot"], false);
    assert_eq!(
        transcode::message_from_json_value::<Schema>(schema::SCHEMA_MESSAGE, &json)
            .unwrap()
            .id
            .unwrap()
            .version,
        9_007_199_254_740_993
    );
    let unset = Schema {
        id: Some(id("order", 1)),
        ..Default::default()
    };
    let json = transcode::message_json_value(schema::SCHEMA_MESSAGE, &unset).unwrap();
    assert!(json.get("aggregateRoot").is_none());
    assert_eq!(pb::CONTRACT_REVISION, pb::MIN_SERVER_CONTRACT_REVISION);
    assert_eq!(pb::CONTRACT_REVISION, pb::MIN_CLIENT_CONTRACT_REVISION);
    assert_eq!(pb::CONTRACT_REVISION, 19);
}

#[test]
fn a_named_field_does_not_become_an_identifier_occurrence() {
    let mut schema = order_schema(id("product", 1));
    schema.fields.push(string_field("order_id", false));
    let entity = entity_of(schema);
    business_data::validate_entity(&entity).unwrap();
    assert!(
        connection_occurrences(&prepare_entity(None, &entity).unwrap())
            .unwrap()
            .is_empty()
    );
}
