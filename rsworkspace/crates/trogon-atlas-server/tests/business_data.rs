//! Store-backed acceptance for native Schema business declarations.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::{Code, Request};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService, field_type};
use trogon_atlas_server::{
    auth::{Principal, PrincipalKind, Role},
    metrics::ServerMetrics,
    scope::NamespaceScope,
    service::EventModelServiceImpl,
};

fn id(namespace: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version,
    }
}

fn segment(name: &str, repeated: bool) -> pb::PropertySegment {
    pb::PropertySegment {
        name: name.into(),
        repeated,
    }
}

fn path(segments: &[(&str, bool)]) -> pb::PropertyPath {
    pb::PropertyPath {
        segments: segments
            .iter()
            .map(|(name, repeated)| segment(name, *repeated))
            .collect(),
    }
}

fn string_field(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType {
            kind: Some(field_type::Kind::String(field_type::StringType {})),
        }),
        ..Default::default()
    }
}

fn object_field(name: &str, repeated: bool, fields: Vec<pb::FieldSpec>) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        repeated,
        r#type: Some(pb::FieldType {
            kind: Some(field_type::Kind::Object(field_type::ObjectType { fields })),
        }),
        ..Default::default()
    }
}

fn schema_ref_field(name: &str, target: pb::Id) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType {
            kind: Some(field_type::Kind::Ref(field_type::RefType {
                r#ref: Some(pb::TypeRef {
                    r#ref: Some(pb::type_ref::Ref::Schema(pb::SchemaRef {
                        id: Some(target),
                    })),
                }),
            })),
        }),
        ..Default::default()
    }
}

fn schema_entity(namespace: &str, slug: &str, version: u64, schema: pb::Schema) -> pb::Entity {
    let mut schema = schema;
    schema.id = Some(id(namespace, slug, version));
    pb::Entity {
        kind: Some(pb::entity::Kind::Schema(schema)),
        ..Default::default()
    }
}

fn order_fields(namespace: &str, product_version: u64) -> Vec<pb::FieldSpec> {
    vec![
        object_field("customer", false, vec![string_field("id")]),
        object_field("lines", true, vec![string_field("sku")]),
        schema_ref_field("product_ref", id(namespace, "product", product_version)),
        string_field("order_id"),
    ]
}

fn product_schema(namespace: &str, version: u64) -> pb::Entity {
    schema_entity(
        namespace,
        "product",
        version,
        pb::Schema {
            title: "Product".into(),
            fields: vec![string_field("sku")],
            ..Default::default()
        },
    )
}

fn relationship(
    namespace: &str,
    version: u64,
    target_property: Option<pb::PropertyPath>,
    label: Option<&str>,
) -> pb::BusinessPropertyRelationship {
    pb::BusinessPropertyRelationship {
        source: Some(path(&[("product_ref", false)])),
        target_entity: Some(pb::SchemaRef {
            id: Some(id(namespace, "product", version)),
        }),
        target_property,
        label: label.map(|value| pb::CatalogTitle {
            value: value.into(),
        }),
        ..Default::default()
    }
}

async fn service() -> EventModelServiceImpl {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap()
}

async fn put(service: &EventModelServiceImpl, entity: pb::Entity) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn get(
    service: &EventModelServiceImpl,
    namespace: &str,
    slug: &str,
    version: u64,
) -> pb::Entity {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, slug, version)),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

fn schema_of(entity: &pb::Entity) -> &pb::Schema {
    match &entity.kind {
        Some(pb::entity::Kind::Schema(schema)) => schema,
        other => panic!("schema, got {other:?}"),
    }
}

fn connection(entity: &pb::Entity, relationship: bool) -> String {
    let schema = schema_of(entity);
    let value = if relationship {
        schema.property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
    } else {
        schema.identifiers[0].connection_id.as_ref().unwrap()
    };
    value.value.clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn native_declarations_persist_and_references_carry_occurrence_identity() {
    let namespace = "bd-persist";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            aggregate_root: Some(true),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
        schema
            .property_relationships
            .push(relationship(namespace, 1, None, None));
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("belongsTo"),
        ));
    }
    let stored = put(&service, order).await;
    let schema = schema_of(&stored);
    assert_eq!(schema.aggregate_root, Some(true));
    assert!(schema.identifiers[0].connection_id.is_some());
    assert_ne!(
        schema.property_relationships[0].connection_id,
        schema.property_relationships[1].connection_id
    );
    let outgoing = service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "order", 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    let business: Vec<_> = outgoing
        .iter()
        .filter(|reference| reference.from_field.contains("property_relationships["))
        .collect();
    assert_eq!(business.len(), 2);
    assert!(business.iter().all(|reference| {
        reference.connection_id.is_some()
            && reference.from_uid == stored.system.as_ref().unwrap().uid
    }));
    assert_eq!(
        business
            .iter()
            .find(|reference| reference.from_field == "property_relationships[0].target_entity")
            .unwrap()
            .business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32)
    );
    assert_eq!(
        business
            .iter()
            .find(|reference| reference.from_field == "property_relationships[1].target_entity")
            .unwrap()
            .business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::PropertyResolved as i32)
    );
    assert!(outgoing.iter().any(|reference| {
        reference.from_field == "fields[2].ref" && reference.business_property_status.is_none()
    }));
    assert!(!outgoing
        .iter()
        .any(|reference| reference.from_field.contains("identifiers")));
    let incoming = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "product", 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert_eq!(
        incoming
            .iter()
            .filter(|reference| reference.from_field.contains("property_relationships["))
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_primary_rejects_and_leaves_the_stored_schema() {
    let namespace = "bd-primary";
    let service = service().await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    let stored = put(&service, order).await;
    let mut rejected = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut rejected.kind {
        schema.title = "Changed".into();
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("lines", true), ("sku", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(rejected),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        schema_of(&get(&service, namespace, "order", 1).await).title,
        "Order"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retarget_moves_both_sites_and_renews_the_relationship_id() {
    let namespace = "bd-retarget";
    let service = service().await;
    let product_v1 = put(&service, product_schema(namespace, 1)).await;
    put(&service, product_schema(namespace, 2)).await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        ));
    }
    let stored = put(&service, order).await;
    let identifier_id = connection(&stored, false);
    let relationship_id = connection(&stored, true);
    service
        .retarget_references(Request::new(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Schema as i32,
                id: schema_of(&product_v1).id.clone(),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "product", 2)),
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    let updated = get(&service, namespace, "order", 1).await;
    let schema = schema_of(&updated);
    assert_eq!(connection(&updated, false), identifier_id);
    assert_ne!(connection(&updated, true), relationship_id);
    assert_eq!(
        schema.property_relationships[0]
            .target_entity
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        2
    );
    match &schema.fields[2].r#type.as_ref().unwrap().kind {
        Some(field_type::Kind::Ref(value)) => match &value.r#ref.as_ref().unwrap().r#ref {
            Some(pb::type_ref::Ref::Schema(schema_ref)) => {
                assert_eq!(schema_ref.id.as_ref().unwrap().version, 2);
            }
            other => panic!("schema ref, got {other:?}"),
        },
        other => panic!("ref, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn self_refs_do_not_block_delete_and_external_relationships_do() {
    let namespace = "bd-delete";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut self_owner = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("sku")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut self_owner.kind {
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("sku", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(namespace, "order", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                ..Default::default()
            });
    }
    put(&service, self_owner).await;
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "order", 1)),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("product_ref")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema
            .property_relationships
            .push(relationship(namespace, 1, None, Some("hasMany")));
    }
    put(&service, order).await;
    let blocked = service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "product", 1)),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition);
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "product", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let stored_entity = get(&service, namespace, "order", 1).await;
    let stored = schema_of(&stored_entity);
    assert_eq!(
        stored.property_relationships[0]
            .target_entity
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .slug,
        "product"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unedited_source_stays_readable_and_the_next_edit_must_repair_the_path() {
    let namespace = "bd-dangle";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        ));
    }
    let stored = put(&service, order).await;
    let mut product = product_schema(namespace, 1);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut product.kind {
        schema.fields = vec![string_field("name")];
    }
    put(&service, product).await;
    let readable = get(&service, namespace, "order", 1).await;
    assert_eq!(
        schema_of(&readable).property_relationships[0]
            .target_property
            .as_ref()
            .unwrap()
            .segments[0]
            .name,
        "sku"
    );
    let outgoing = service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "order", 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references;
    assert_eq!(
        outgoing
            .iter()
            .find(|reference| reference.from_field.contains("property_relationships["))
            .unwrap()
            .business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::PropertyUnresolved as i32)
    );
    let mut edited = readable.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut edited.kind {
        schema.title = "Renamed".into();
    }
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(edited),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        schema_of(&get(&service, namespace, "order", 1).await).title,
        "Order"
    );
    let mut repaired = stored;
    if let Some(pb::entity::Kind::Schema(schema)) = &mut repaired.kind {
        schema.property_relationships.clear();
        schema.title = "Repaired".into();
    }
    put(&service, repaired).await;
    assert!(schema_of(&get(&service, namespace, "order", 1).await)
        .property_relationships
        .is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn same_batch_targets_admit_and_preview_ids_are_not_selectors() {
    let namespace = "bd-batch";
    let service = service().await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema
            .property_relationships
            .push(relationship(namespace, 1, None, Some("hasMany")));
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    let applied = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(product_schema(namespace, 1)),
                        ..Default::default()
                    })),
                },
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(order.clone()),
                        ..Default::default()
                    })),
                },
            ],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        applied.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let stored = get(&service, namespace, "order", 1).await;
    let stored_id = connection(&stored, false);
    let mut preview = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut preview.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("lines", true), ("sku", false)])),
            role: pb::BusinessIdentifierRole::Alternate as i32,
            ..Default::default()
        });
    }
    let preview = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(preview),
            validate_only: true,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let preview_schema = schema_of(&preview);
    assert_eq!(preview_schema.identifiers.len(), 2);
    let preview_id = preview_schema.identifiers[1]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_ne!(preview_id, stored_id);
    assert!(preview_schema.identifiers[1].connection_id.is_some());
    let unchanged = get(&service, namespace, "order", 1).await;
    assert_eq!(schema_of(&unchanged).identifiers.len(), 1);
    assert_eq!(connection(&unchanged, false), stored_id);
    let mut submitted = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut submitted.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            connection_id: Some(pb::ConnectionId { value: preview_id }),
            property: Some(path(&[("lines", true), ("sku", false)])),
            role: pb::BusinessIdentifierRole::Alternate as i32,
            ..Default::default()
        });
    }
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(submitted),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(
        error.message().contains("is not active in this owner"),
        "{error:?}"
    );
    let after = get(&service, namespace, "order", 1).await;
    assert_eq!(schema_of(&after).identifiers.len(), 1);
    assert_eq!(connection(&after, false), stored_id);
    let _ = order;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_connection_ids_and_payload_business_fields_publish_nothing() {
    let namespace = "bd-empty";
    let service = service().await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            connection_id: Some(pb::ConnectionId {
                value: String::new(),
            }),
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(order),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let payload = pb::Schema {
        id: Some(id(namespace, "order", 1)),
        aggregate_root: Some(true),
        ..Default::default()
    };
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(namespace, "order-placed", 1)),
                    title: "Order placed".into(),
                    schema: Some(trogon_atlas_core::schema::pack_schema(&payload)),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn impact_stops_at_a_relationship_and_an_out_of_scope_writer_cannot_mutate() {
    let namespace = "bd-impact";
    let service = service().await;
    let mut product = product_schema(namespace, 1);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut product.kind {
        schema.fields.push(schema_ref_field(
            "customer_ref",
            id(namespace, "customer-record", 1),
        ));
    }
    put(&service, product).await;
    put(
        &service,
        schema_entity(
            namespace,
            "customer-record",
            1,
            pb::Schema {
                title: "Customer".into(),
                fields: vec![string_field("id")],
                ..Default::default()
            },
        ),
    )
    .await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("product_ref")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        ));
    }
    put(&service, order).await;
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }),
            max_depth: 5,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let slugs: Vec<_> = impact
        .nodes
        .iter()
        .filter_map(|node| node.entity.as_ref()?.id.as_ref().map(|id| id.slug.clone()))
        .collect();
    assert!(slugs.contains(&"product".to_string()));
    assert!(!slugs.contains(&"customer-record".to_string()));
    let mut request = Request::new(pb::PutEntityRequest {
        entity: Some(schema_entity(
            namespace,
            "order",
            1,
            pb::Schema {
                title: "Denied".into(),
                ..Default::default()
            },
        )),
        ..Default::default()
    });
    request.extensions_mut().insert(Principal {
        name: Arc::from("scoped-writer"),
        role: Role::Writer,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: NamespaceScope::parse(&["other-namespace"]).unwrap(),
        parent: None,
    });
    let denied = service.put_entity(request).await.unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
    assert!(denied.message().contains(namespace), "{denied:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_and_missing_targets_share_one_unavailable_status() {
    let service = service().await;
    let visible = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-visible-status".into(),
            parent: "acme".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-hidden-status".into(),
            parent: "beta".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    put(
        &service,
        schema_entity(
            &hidden,
            "product",
            1,
            pb::Schema {
                title: "hidden-product-title-sentinel".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    let mut order = schema_entity(
        &visible,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("product_ref")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("product_ref", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(&hidden, "product", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                label: Some(pb::CatalogTitle {
                    value: "hasMany".into(),
                }),
                ..Default::default()
            });
    }
    put(&service, order).await;
    let mut request = Request::new(pb::GetReferencesRequest {
        kind: pb::EntityKind::Schema as i32,
        id: Some(id(&visible, "order", 1)),
        ..Default::default()
    });
    request.extensions_mut().insert(Principal {
        name: Arc::from("acme"),
        role: Role::Reader,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: Some(trogon_atlas_core::OwnerId::parse("acme").unwrap()),
    });
    let hidden_page = service
        .get_outgoing_references(request)
        .await
        .unwrap()
        .into_inner();
    let hidden_status = hidden_page
        .references
        .iter()
        .find(|reference| reference.from_field.contains("property_relationships["))
        .unwrap()
        .business_property_status;
    assert_eq!(
        hidden_status,
        Some(pb::BusinessPropertyReferenceStatus::Unavailable as i32)
    );
    assert!(!format!("{hidden_page:?}").contains("hidden-product-title-sentinel"));
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(&hidden, "product", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let missing_page = service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(&visible, "order", 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        missing_page
            .references
            .iter()
            .find(|reference| reference.from_field.contains("property_relationships["))
            .unwrap()
            .business_property_status,
        hidden_status
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_edits_keep_ids_and_a_missing_branch_does_not_read_baseline() {
    let namespace = "bd-branch";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        ));
    }
    let baseline = put(&service, order).await;
    let identifier_id = connection(&baseline, false);
    let relationship_id = connection(&baseline, true);
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "bd-feature".into(),
            doc: "business ids".into(),
        }))
        .await
        .unwrap();
    let mut request = Request::new(pb::PutEntityRequest {
        entity: Some({
            let mut edited = baseline.clone();
            if let Some(pb::entity::Kind::Schema(schema)) = &mut edited.kind {
                schema.title = "Branch title".into();
            }
            edited
        }),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "x-trogon-atlas-branch",
        tonic::metadata::MetadataValue::try_from("bd-feature").unwrap(),
    );
    let branched = service
        .put_entity(request)
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(connection(&branched, false), identifier_id);
    assert_eq!(connection(&branched, true), relationship_id);
    let mut missing = Request::new(pb::GetEntityRequest {
        kind: pb::EntityKind::Schema as i32,
        id: Some(id(namespace, "order", 1)),
    });
    missing.metadata_mut().insert(
        "x-trogon-atlas-branch",
        tonic::metadata::MetadataValue::try_from("bd-missing").unwrap(),
    );
    assert_eq!(
        service.get_entity(missing).await.unwrap_err().code(),
        Code::NotFound
    );
    service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: "bd-feature".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let landed = get(&service, namespace, "order", 1).await;
    assert_eq!(schema_of(&landed).title, "Branch title");
    assert_eq!(connection(&landed, false), identifier_id);
    assert_eq!(connection(&landed, true), relationship_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_safe_delete_ignores_self_refs_and_keeps_external_referrers() {
    let namespace = "bd-batch-delete";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut self_owner = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("sku")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut self_owner.kind {
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("sku", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(namespace, "order", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                ..Default::default()
            });
    }
    put(&service, self_owner).await;
    let deleted = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: pb::EntityKind::Schema as i32,
                    id: Some(id(namespace, "order", 1)),
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    ..Default::default()
                })),
            }],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        deleted.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("product_ref")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema
            .property_relationships
            .push(relationship(namespace, 1, None, Some("hasMany")));
    }
    put(&service, order).await;
    let blocked = service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    kind: pb::EntityKind::Schema as i32,
                    id: Some(id(namespace, "product", 1)),
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    ..Default::default()
                })),
            }],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        blocked.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(
        schema_of(&get(&service, namespace, "product", 1).await).title,
        "Product"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hidden_relationship_referrer_blocks_delete_without_naming_itself() {
    let service = service().await;
    let visible = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-visible-delete".into(),
            parent: "acme".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-hidden-delete".into(),
            parent: "beta".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    put(
        &service,
        schema_entity(
            &visible,
            "product",
            1,
            pb::Schema {
                title: "Product".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    put(
        &service,
        schema_entity(
            &hidden,
            "order",
            1,
            pb::Schema {
                title: "hidden-order-title-sentinel".into(),
                fields: vec![string_field("product_ref")],
                property_relationships: vec![pb::BusinessPropertyRelationship {
                    source: Some(path(&[("product_ref", false)])),
                    target_entity: Some(pb::SchemaRef {
                        id: Some(id(&visible, "product", 1)),
                    }),
                    label: Some(pb::CatalogTitle {
                        value: "hasMany".into(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ),
    )
    .await;
    let mut request = Request::new(pb::DeleteEntityRequest {
        kind: pb::EntityKind::Schema as i32,
        id: Some(id(&visible, "product", 1)),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        ..Default::default()
    });
    request.extensions_mut().insert(Principal {
        name: Arc::from("acme"),
        role: Role::Writer,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: Some(trogon_atlas_core::OwnerId::parse("acme").unwrap()),
    });
    let blocked = service.delete_entity(request).await.unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition);
    assert!(!format!("{blocked:?}").contains("hidden-order-title-sentinel"));
    assert_eq!(
        schema_of(&get(&service, &visible, "product", 1).await).title,
        "Product"
    );
}

fn source_key(registration: &str, key: &str) -> pb::SourceResourceKey {
    pb::SourceResourceKey {
        registration_id: registration.into(),
        category: "model".into(),
        key: key.into(),
    }
}

fn field_ownership(fields: &[pb::SourceOwnedField]) -> pb::SourceFieldOwnership {
    pb::SourceFieldOwnership {
        policy: Some(pb::source_field_ownership::Policy::Fields(
            pb::SourceOwnedFields {
                fields: fields.iter().map(|field| *field as i32).collect(),
            },
        )),
    }
}

fn observed_schema(entity: &pb::Entity, etag: &str) -> pb::SourceObservedOwner {
    pb::SourceObservedOwner {
        owner: Some(pb::EntityRef {
            kind: pb::EntityKind::Schema as i32,
            id: trogon_atlas_core::refs::entity_id(entity).cloned(),
        }),
        uid: entity.system.as_ref().unwrap().uid.clone(),
        etag: etag.into(),
    }
}

async fn put_with_etag(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
) -> (pb::Entity, String) {
    let response = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    (response.entity.unwrap(), response.etag)
}

#[tokio::test(flavor = "multi_thread")]
async fn source_preview_keeps_dangling_targets_and_rejects_broken_local_paths() {
    let service = service().await;
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-source-ns".into(),
            parent: "acme".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let (mut product, _) = put_with_etag(&service, product_schema(&namespace, 1)).await;
    let mut order = schema_entity(
        &namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: order_fields(&namespace, 1),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
        schema.property_relationships.push(relationship(
            &namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        ));
    }
    let (order, order_etag) = put_with_etag(&service, order).await;
    if let Some(pb::entity::Kind::Schema(schema)) = &mut product.kind {
        schema.fields.clear();
    }
    put(&service, product).await;
    let readable = get(&service, &namespace, "order", 1).await;
    assert_eq!(
        schema_of(&readable).property_relationships[0]
            .target_property
            .as_ref()
            .unwrap()
            .segments[0]
            .name,
        "sku"
    );
    let registration = service
        .create_source_registration(Request::new(pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "bd-source-order".into(),
                administration_namespace: namespace.clone(),
                destination_namespaces: vec![namespace.clone()],
                profile_revision: 1,
                ownership_policy_revision: "policy-v1".into(),
                deletion_policy: pb::SourceDeletionPolicy::RetainNative as i32,
                ..Default::default()
            }),
        }))
        .await
        .unwrap()
        .into_inner()
        .registration
        .unwrap();
    let resources = ["order", "order-fields", "fresh"]
        .into_iter()
        .map(|key| {
            serde_json::json!({
                "key": {"category": "model", "key": key},
                "file": "native/model",
                "pointer": {"kind": "native", "raw": "opaque"}
            })
        })
        .collect::<Vec<_>>();
    let inventory = serde_json::to_vec(&serde_json::json!({
        "profile": "supplied-native-v1",
        "profile_revision": 1,
        "registration": registration.id,
        "source_version": "opaque-A",
        "partition": {"categories": ["model"], "logical_roots": ["native"], "selected_keys": []},
        "coverage": "PARTIAL",
        "resources": resources,
        "relations": []
    }))
    .unwrap();
    let snapshot = service
        .accept_source_snapshot(Request::new(pb::AcceptSourceSnapshotRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            inventory_bytes: inventory,
            files: vec![pb::SourceFile {
                logical_name: "native/model".into(),
                media_type: "application/octet-stream".into(),
                content: b"opaque supplied bytes".to_vec(),
                content_digest: String::new(),
            }],
        }))
        .await
        .unwrap()
        .into_inner()
        .snapshot
        .unwrap();
    let whole_owner = service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "order-whole".into(),
                source: Some(source_key(&registration.id, "order")),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(
                        observed_schema(&order, &order_etag).owner.unwrap(),
                    )),
                }),
                ownership: Some(pb::SourceFieldOwnership {
                    policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                        pb::SourceWholeOwnerOwnership {},
                    )),
                }),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(observed_schema(&order, &order_etag)),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap_err();
    assert_eq!(whole_owner.code(), Code::InvalidArgument);
    assert!(
        whole_owner
            .message()
            .contains("business declarations support native authoring only"),
        "{whole_owner:?}"
    );
    service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "order-title".into(),
                source: Some(source_key(&registration.id, "order")),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(
                        observed_schema(&order, &order_etag).owner.unwrap(),
                    )),
                }),
                ownership: Some(field_ownership(&[pb::SourceOwnedField::Title])),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(observed_schema(&order, &order_etag)),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();
    let mut renamed = order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut renamed.kind {
        schema.title = "Renamed".into();
    }
    let run = service
        .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: "rename".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source_key(&registration.id, "order")),
                entity: Some(renamed),
            }],
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(run.status, pb::SourceRunStatus::Previewed as i32);
    let applied = service
        .apply_source_run(Request::new(pb::ApplySourceRunRequest {
            registration_id: registration.id.clone(),
            run_id: run.id,
            if_match: run.revision,
            preview_digest: run.preview_digest,
        }))
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(applied.status, pb::SourceRunStatus::Applied as i32);
    let renamed_entity = get(&service, &namespace, "order", 1).await;
    let renamed_schema = schema_of(&renamed_entity);
    assert_eq!(renamed_schema.title, "Renamed");
    assert_eq!(renamed_schema.aggregate_root, None);
    assert_eq!(renamed_schema.identifiers, schema_of(&order).identifiers);
    assert_eq!(
        renamed_schema.property_relationships,
        schema_of(&order).property_relationships
    );
    assert_eq!(
        renamed_entity.system.as_ref().unwrap().uid,
        order.system.as_ref().unwrap().uid
    );
    let renamed_response = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(&namespace, "order", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "order-fields".into(),
                source: Some(source_key(&registration.id, "order-fields")),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(
                        observed_schema(&renamed_entity, &renamed_response.etag)
                            .owner
                            .unwrap(),
                    )),
                }),
                ownership: Some(field_ownership(&[pb::SourceOwnedField::Fields])),
                adoption: Some(pb::SourceAdoption {
                    observed: Some(observed_schema(&renamed_entity, &renamed_response.etag)),
                    authorized_adoption: true,
                }),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();
    let mut broken_fields = renamed_entity.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut broken_fields.kind {
        schema.identifiers.clear();
        schema.property_relationships.clear();
        schema.fields.retain(|field| field.name != "customer");
    }
    let broken = service
        .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: "drop-customer".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source_key(&registration.id, "order-fields")),
                entity: Some(broken_fields),
            }],
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(broken.code(), Code::InvalidArgument);
    assert!(broken.message().contains("property segment"), "{broken:?}");
    let kept = get(&service, &namespace, "order", 1).await;
    assert_eq!(schema_of(&kept).title, "Renamed");
    assert_eq!(schema_of(&kept).identifiers, schema_of(&order).identifiers);
    assert_eq!(
        schema_of(&kept).property_relationships,
        schema_of(&order).property_relationships
    );
    service
        .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
            registration_revision: registration.revision.clone(),
            mapping: Some(pb::SourceMapping {
                id: "fresh-title".into(),
                source: Some(source_key(&registration.id, "fresh")),
                selection: Some(pb::SourceSelection {
                    selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
                        kind: pb::EntityKind::Schema as i32,
                        id: Some(id(&namespace, "fresh", 1)),
                    })),
                }),
                ownership: Some(field_ownership(&[pb::SourceOwnedField::Title])),
                ..Default::default()
            }),
        }))
        .await
        .unwrap();
    let created = service
        .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
            registration_id: registration.id.clone(),
            registration_revision: registration.revision.clone(),
            snapshot_id: snapshot.id.clone(),
            run_id: "fresh-create".into(),
            native_payloads: vec![pb::SourceNativePayload {
                source: Some(source_key(&registration.id, "fresh")),
                entity: Some(schema_entity(
                    &namespace,
                    "fresh",
                    1,
                    pb::Schema {
                        title: "Fresh".into(),
                        aggregate_root: Some(true),
                        ..Default::default()
                    },
                )),
            }],
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(created.code(), Code::InvalidArgument);
    assert!(
        created
            .message()
            .contains("business declarations support native authoring only"),
        "{created:?}"
    );
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&namespace, "fresh", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}
