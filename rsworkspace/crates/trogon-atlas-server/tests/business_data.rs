//! Store-backed acceptance for native Schema business declarations.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use prost::Message;
use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
    Request,
};
use trogon_atlas_proto::{
    self as pb,
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::{EventModelService, EventModelServiceServer},
    field_type,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, Principal, PrincipalKind, Role, TokenRegistry},
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

fn with_branch<T>(mut request: Request<T>, branch: &str) -> Request<T> {
    request.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    request
}

fn order_targeting_sku(namespace: &str) -> pb::Entity {
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
    order
}

fn product_without_sku(namespace: &str) -> pb::Entity {
    let mut product = product_schema(namespace, 1);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut product.kind {
        schema.fields = vec![string_field("name")];
    }
    product
}

fn fan_in(namespace: &str, slug: &str, count: usize) -> pb::Entity {
    fan_in_toward(namespace, slug, "product", count)
}

fn fan_in_toward(namespace: &str, slug: &str, target: &str, count: usize) -> pb::Entity {
    schema_entity(
        namespace,
        slug,
        1,
        pb::Schema {
            title: slug.into(),
            fields: vec![string_field("product_ref")],
            property_relationships: (0..count)
                .map(|_| pb::BusinessPropertyRelationship {
                    source: Some(path(&[("product_ref", false)])),
                    target_entity: Some(pb::SchemaRef {
                        id: Some(id(namespace, target, 1)),
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
    )
}

fn labeled_relationship(
    namespace: &str,
    label: &str,
    connection_id: Option<pb::ConnectionId>,
) -> pb::BusinessPropertyRelationship {
    pb::BusinessPropertyRelationship {
        connection_id,
        source: Some(path(&[("product_ref", false)])),
        target_entity: Some(pb::SchemaRef {
            id: Some(id(namespace, "product", 1)),
        }),
        target_property: Some(path(&[("sku", false)])),
        label: Some(pb::CatalogTitle {
            value: label.into(),
        }),
        ..Default::default()
    }
}

fn relationship_by_label<'a>(
    schema: &'a pb::Schema,
    label: &str,
) -> &'a pb::BusinessPropertyRelationship {
    schema
        .property_relationships
        .iter()
        .find(|relationship| {
            relationship
                .label
                .as_ref()
                .is_some_and(|title| title.value == label)
        })
        .unwrap()
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
    assert_eq!(
        schema_of(&get(&service, namespace, "order", 1).await)
            .property_relationships
            .len(),
        0
    );
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
    assert_eq!(
        service
            .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
                registration_id: registration.id.clone(),
                mapping_id: "order-whole".into(),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
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
    let fields_mapping = service
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
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert!(fields_mapping.base.is_some());
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
    let fields_after = service
        .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
            registration_id: registration.id.clone(),
            mapping_id: "order-fields".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert_eq!(fields_after.base, fields_mapping.base);
    assert_eq!(fields_after.last_accepted_run_id, "");
    assert_eq!(
        service
            .get_source_run(Request::new(pb::GetSourceRunRequest {
                registration_id: registration.id.clone(),
                run_id: "drop-customer".into(),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let fresh_mapping = service
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
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert!(fresh_mapping.base.is_none());
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
    let fresh_after = service
        .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
            registration_id: registration.id.clone(),
            mapping_id: "fresh-title".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap();
    assert!(fresh_after.base.is_none());
    assert_eq!(fresh_after.last_accepted_run_id, "");
    assert_eq!(fresh_after.revision, fresh_mapping.revision);
    assert_eq!(
        service
            .get_source_run(Request::new(pb::GetSourceRunRequest {
                registration_id: registration.id.clone(),
                run_id: "fresh-create".into(),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

fn with_token<T>(body: T, token: &str) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    request
}

async fn authenticated_business_server() -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let registry = Arc::new(
        TokenRegistry::from_file_contents(
            toml::from_str(
                r#"
[principals.business-reader]
role = "reader"
tokens = ["business-reader-token"]

[principals.business-writer]
role = "writer"
tokens = ["business-writer-token"]
"#,
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(EventModelServiceServer::new(service)),
        BearerAuth::new(registry),
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    EventModelServiceClient::connect(format!("http://{address}"))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_reader_can_read_but_cannot_mutate_business_fields() {
    let mut client = authenticated_business_server().await;
    let namespace = "bd-auth-reader";
    let stored = client
        .put_entity(with_token(
            pb::PutEntityRequest {
                entity: Some(schema_entity(
                    namespace,
                    "order",
                    1,
                    pb::Schema {
                        title: "Order".into(),
                        aggregate_root: Some(true),
                        fields: order_fields(namespace, 1),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            "business-writer-token",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let readable = client
        .get_entity(with_token(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            },
            "business-reader-token",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(schema_of(&readable).aggregate_root, Some(true));
    let mut denied_entity = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut denied_entity.kind {
        schema.title = "Denied".into();
        schema.aggregate_root = Some(false);
    }
    let denied = client
        .put_entity(with_token(
            pb::PutEntityRequest {
                entity: Some(denied_entity.clone()),
                ..Default::default()
            },
            "business-reader-token",
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied);
    let batch_denied = client
        .batch_mutate(with_token(
            pb::BatchMutateRequest {
                ops: vec![pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                        entity: Some(denied_entity),
                        ..Default::default()
                    })),
                }],
                ..Default::default()
            },
            "business-reader-token",
        ))
        .await
        .unwrap_err();
    assert_eq!(batch_denied.code(), Code::PermissionDenied);
    let unchanged = client
        .get_entity(with_token(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            },
            "business-writer-token",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(schema_of(&unchanged).title, "Order");
    assert_eq!(schema_of(&unchanged).aggregate_root, Some(true));
    assert_eq!(
        unchanged.system.as_ref().unwrap().uid,
        stored.system.as_ref().unwrap().uid
    );
}

fn connection_subject(owner: &pb::Entity, connection_id: &str) -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(pb::EntityRef {
                    kind: pb::EntityKind::Schema as i32,
                    id: trogon_atlas_core::refs::entity_id(owner).cloned(),
                }),
                id: Some(pb::ConnectionId {
                    value: connection_id.into(),
                }),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
            },
        )),
    }
}

fn catalog_subject(owner: &pb::Entity, connection_id: &str) -> pb::CatalogSubject {
    pb::CatalogSubject {
        selection: Some(pb::catalog_subject::Selection::Connection(
            pb::ConnectionRef {
                owner: Some(pb::EntityRef {
                    kind: pb::EntityKind::Schema as i32,
                    id: trogon_atlas_core::refs::entity_id(owner).cloned(),
                }),
                id: Some(pb::ConnectionId {
                    value: connection_id.into(),
                }),
                owner_uid: owner.system.as_ref().unwrap().uid.clone(),
            },
        )),
    }
}

fn entity_subject(kind: pb::EntityKind, id: pb::Id) -> pb::ResponsibilitySubject {
    pb::ResponsibilitySubject {
        selection: Some(pb::responsibility_subject::Selection::Entity(
            pb::EntityRef {
                kind: kind as i32,
                id: Some(id),
            },
        )),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_subject_names_one_occurrence_and_not_its_equal_target_sibling() {
    let namespace = "bd-subject";
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
        schema
            .property_relationships
            .push(relationship(namespace, 1, None, Some("hasMany")));
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("belongsTo"),
        ));
    }
    let order = put(&service, order).await;
    let schema = schema_of(&order);
    let first_id = schema.property_relationships[0]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let sibling_id = schema.property_relationships[1]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let identifier_id = schema.identifiers[0]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_ne!(first_id, sibling_id);
    assert_eq!(
        schema.property_relationships[0].target_entity,
        schema.property_relationships[1].target_entity
    );
    let first = connection_subject(&order, &first_id);
    let sibling = connection_subject(&order, &sibling_id);
    let identifier = connection_subject(&order, &identifier_id);
    put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Team(pb::Team {
                id: Some(id(namespace, "fulfillment", 1)),
                title: Some(pb::DirectoryDisplayName {
                    value: "Fulfillment".into(),
                }),
                responsibilities: vec![pb::Responsibility {
                    kind: pb::ResponsibilityKind::Owner as i32,
                    subject: Some(first.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Decision(pb::Decision {
                id: Some(id(namespace, "record-key", 1)),
                title: Some(pb::CatalogTitle {
                    value: "Record key".into(),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(catalog_subject(&order, &identifier_id)),
                    ..Default::default()
                }],
                status: pb::DecisionStatus::Proposed as i32,
                context: Some(pb::CatalogText {
                    value: "The identifier occurrence is the record key.".into(),
                }),
                rationale: Some(pb::CatalogText {
                    value: "A field index is not that occurrence.".into(),
                }),
                options: vec![pb::DecisionOption {
                    title: Some(pb::CatalogTitle {
                        value: "Keep the identifier".into(),
                    }),
                    description: Some(pb::CatalogText {
                        value: "Name the connection, not the field position.".into(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let assigned = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(first.clone()),
            kind: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(assigned.assignments.len(), 1);
    assert_eq!(
        assigned.assignments[0].assignment.as_ref().unwrap().subject,
        Some(first.clone())
    );
    assert_eq!(
        service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: Some(sibling.clone()),
                kind: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .assignments
            .len(),
        0
    );
    assert_eq!(
        service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: Some(identifier),
                kind: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .assignments
            .len(),
        0
    );
    assert_eq!(
        service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: Some(entity_subject(
                    pb::EntityKind::Schema,
                    id(namespace, "product", 1),
                )),
                kind: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .assignments
            .len(),
        0
    );
    assert_eq!(
        service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: Some(entity_subject(
                    pb::EntityKind::Schema,
                    id(namespace, "order", 1),
                )),
                kind: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .assignments
            .len(),
        0
    );
    let decisions = service
        .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
            subject: Some(catalog_subject(&order, &identifier_id)),
            status: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(decisions.declarations.len(), 1);
    assert_eq!(
        service
            .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
                subject: Some(catalog_subject(&order, &first_id)),
                status: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .declarations
            .len(),
        0
    );
    assert_eq!(
        service
            .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
                subject: Some(catalog_subject(&order, &sibling_id)),
                status: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .declarations
            .len(),
        0
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
    assert!(outgoing.iter().any(|reference| {
        reference.from_field.contains("fields[")
            && reference
                .from_field
                .rsplit_once('.')
                .is_some_and(|(_, suffix)| suffix == "ref")
            && reference.business_property_status.is_none()
            && reference.connection_id.is_none()
    }));
    let bogus = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Team(pb::Team {
                    id: Some(id(namespace, "missing-occurrence", 1)),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Missing".into(),
                    }),
                    responsibilities: vec![pb::Responsibility {
                        kind: pb::ResponsibilityKind::Owner as i32,
                        subject: Some(connection_subject(
                            &order,
                            "11111111-1111-4111-8111-111111111111",
                        )),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(bogus.code(), Code::InvalidArgument, "{bogus:?}");
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Team as i32,
                id: Some(id(namespace, "missing-occurrence", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn force_delete_and_recreate_reports_the_new_target_uid_and_etag() {
    let namespace = "bd-recreate";
    let service = service().await;
    let (product, product_etag) = put_with_etag(&service, product_schema(namespace, 1)).await;
    let product_uid = product.system.as_ref().unwrap().uid.clone();
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
    let order = put(&service, order).await;
    let order_uid = order.system.as_ref().unwrap().uid.clone();
    let connection_id = schema_of(&order).property_relationships[0]
        .connection_id
        .clone();
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "product", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let recreated = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(product_schema(namespace, 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let recreated_entity = recreated.entity.unwrap();
    let recreated_uid = recreated_entity.system.as_ref().unwrap().uid.clone();
    assert_ne!(recreated_uid, product_uid);
    assert_ne!(recreated.etag, product_etag);
    let fetched = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "product", 1)),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched.entity.unwrap().system.unwrap().uid, recreated_uid);
    assert_eq!(fetched.etag, recreated.etag);
    let order_after = get(&service, namespace, "order", 1).await;
    assert_eq!(order_after.system.as_ref().unwrap().uid, order_uid);
    assert_eq!(
        schema_of(&order_after).property_relationships[0].connection_id,
        connection_id
    );
    assert_eq!(
        schema_of(&order_after).property_relationships[0]
            .target_entity
            .as_ref()
            .unwrap()
            .id,
        Some(id(namespace, "product", 1))
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
    let reference = outgoing
        .iter()
        .find(|reference| reference.from_field.contains("property_relationships["))
        .unwrap();
    assert_eq!(reference.from_uid, order_uid);
    assert_ne!(reference.from_uid, recreated_uid);
    assert_eq!(
        reference.business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::PropertyResolved as i32)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn type_reuse_impact_still_walks_past_a_schema_ref() {
    let namespace = "bd-type-reuse";
    let service = service().await;
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
            "viewer",
            1,
            pb::Schema {
                title: "Viewer".into(),
                fields: vec![schema_ref_field("product_ref", id(namespace, "product", 1))],
                ..Default::default()
            },
        ),
    )
    .await;
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "viewer", 1)),
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
    assert!(slugs.contains(&"product".to_string()), "{slugs:?}");
    assert!(slugs.contains(&"customer-record".to_string()), "{slugs:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn review_red_parallel_type_walk_still_reaches_past_a_business_target() {
    let namespace = "bd-legacy-walk";
    let service = service().await;
    put(
        &service,
        schema_entity(
            namespace,
            "d",
            1,
            pb::Schema {
                title: "D".into(),
                fields: vec![string_field("id")],
                ..Default::default()
            },
        ),
    )
    .await;
    put(
        &service,
        schema_entity(
            namespace,
            "b",
            1,
            pb::Schema {
                title: "B".into(),
                fields: vec![schema_ref_field("d_ref", id(namespace, "d", 1))],
                ..Default::default()
            },
        ),
    )
    .await;
    put(
        &service,
        schema_entity(
            namespace,
            "c",
            1,
            pb::Schema {
                title: "C".into(),
                fields: vec![schema_ref_field("b_ref", id(namespace, "b", 1))],
                ..Default::default()
            },
        ),
    )
    .await;
    let mut root = schema_entity(
        namespace,
        "root",
        1,
        pb::Schema {
            title: "Root".into(),
            fields: vec![schema_ref_field("c_ref", id(namespace, "c", 1))],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut root.kind {
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("c_ref", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(namespace, "b", 1)),
                }),
                ..Default::default()
            });
    }
    put(&service, root).await;
    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(pb::EntityRef {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "root", 1)),
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
    assert!(slugs.contains(&"b".to_string()), "{slugs:?}");
    assert!(slugs.contains(&"c".to_string()), "{slugs:?}");
    assert!(
        slugs.contains(&"d".to_string()),
        "legacy type walk did not reach d: {slugs:?}"
    );
    let through_business = impact.nodes.iter().any(|node| {
        node.entity
            .as_ref()
            .and_then(|entity| entity.id.as_ref())
            .is_some_and(|id| id.slug == "d")
            && node
                .path
                .iter()
                .any(|reference| reference.from_field.contains("property_relationships["))
    });
    assert!(
        !through_business,
        "d must be reached by the type walk, not the business edge"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_red_rebase_revalidates_an_added_source_when_the_target_property_is_gone() {
    let namespace = "bd-rebase-added";
    let branch = "bd-rebase-added";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: branch.into(),
            doc: "added source".into(),
        }))
        .await
        .unwrap();
    let stored = service
        .put_entity(with_branch(
            Request::new(pb::PutEntityRequest {
                entity: Some(order_targeting_sku(namespace)),
                ..Default::default()
            }),
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let target_property = schema_of(&stored).property_relationships[0]
        .target_property
        .clone();
    put(&service, product_without_sku(namespace)).await;
    let error = service
        .update_branch(Request::new(pb::UpdateBranchRequest {
            name: branch.into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error:?}");
    let still = service
        .get_entity(with_branch(
            Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }),
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(
        schema_of(&still).property_relationships[0].target_property,
        target_property
    );
}

fn incoming_page(
    namespace: &str,
    slug: &str,
    page_size: i32,
    page_token: &str,
) -> pb::GetReferencesRequest {
    pb::GetReferencesRequest {
        kind: pb::EntityKind::Schema as i32,
        id: Some(id(namespace, slug, 1)),
        page_size,
        page_token: page_token.into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn review_red_one_row_page_of_4097_references_stays_valid() {
    let namespace = "bd-ref-page";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    for index in 0..4 {
        put(
            &service,
            fan_in(namespace, &format!("owner-{index}"), 1_024),
        )
        .await;
    }
    let within = service
        .get_incoming_references(Request::new(incoming_page(namespace, "product", 1, "")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(within.references.len(), 1);
    assert_ne!(within.next_page_token, "");
    assert_eq!(
        within.references[0].business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32)
    );
    put(&service, fan_in(namespace, "owner-4", 1)).await;
    let page = service
        .get_incoming_references(Request::new(incoming_page(namespace, "product", 1, "")))
        .await
        .expect("page_size 1 emits one reference, so 4097 matches stay paginated")
        .into_inner();
    assert_eq!(page.references.len(), 1);
    assert_ne!(page.next_page_token, "");
    assert_eq!(
        page.references[0].business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32)
    );
    let next = service
        .get_incoming_references(Request::new(incoming_page(
            namespace,
            "product",
            1,
            &page.next_page_token,
        )))
        .await
        .expect("the following page opens no second output charge for rows it does not return")
        .into_inner();
    assert_eq!(next.references.len(), 1);
    assert_eq!(
        next.references[0].business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_red_truncated_snapshot_does_not_present_omitted_schema_referrers_as_complete() {
    let namespace = "bd-schema-omit";
    // Slug order is aaa-filler, product, zzz-referrer. A cap of 2 keeps the
    // first two and drops the only schema that points at product.
    let service = service().await.with_snapshot_cap(2);
    put(&service, product_schema(namespace, 1)).await;
    put(
        &service,
        schema_entity(
            namespace,
            "aaa-filler",
            1,
            pb::Schema {
                title: "Filler".into(),
                fields: vec![string_field("name")],
                ..Default::default()
            },
        ),
    )
    .await;
    let referrer = put(&service, fan_in(namespace, "zzz-referrer", 1)).await;
    let referrer_connection = schema_of(&referrer).property_relationships[0]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let stored = get(&service, namespace, "zzz-referrer", 1).await;
    assert!(schema_of(&stored)
        .property_relationships
        .iter()
        .any(|relationship| {
            relationship
                .connection_id
                .as_ref()
                .is_some_and(|id| id.value == referrer_connection)
        }));
    let discovered = service
        .get_incoming_references(Request::new(incoming_page(namespace, "product", 50, "")))
        .await;
    let error = discovered.expect_err(
        "a capped snapshot that dropped zzz-referrer must not answer as a complete incoming set",
    );
    assert_eq!(error.code(), Code::ResourceExhausted, "{error:?}");
    assert!(error.message().contains("omitted schemas"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn review_red_incoming_discovery_charges_every_examined_schema() {
    let namespace = "bd-examined";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    put(
        &service,
        schema_entity(
            namespace,
            "other",
            1,
            pb::Schema {
                title: "Other".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    // Ten owners point at `other`, not `product`. 10 * 1024 occurrences is
    // past the shared scan budget even though none of them is a match.
    for index in 0..10 {
        put(
            &service,
            fan_in_toward(namespace, &format!("owner-{index}"), "other", 1_024),
        )
        .await;
    }
    let error = service
        .get_incoming_references(Request::new(incoming_page(namespace, "product", 1, "")))
        .await
        .expect_err("schemas examined while scanning are part of the shared occurrence budget");
    assert_eq!(error.code(), Code::ResourceExhausted, "{error:?}");
    assert!(
        error
            .message()
            .contains("business reference occurrence budget exceeded"),
        "{error:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_accept_coherent_incoming_status_follows_the_connection_id() {
    let namespace = "bd-coherent-status";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
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
            Some("resolved"),
        ));
        schema
            .property_relationships
            .push(relationship(namespace, 1, None, Some("entity-only")));
    }
    let stored = put(&service, order).await;
    let resolved_id = relationship_by_label(schema_of(&stored), "resolved")
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let entity_only_id = relationship_by_label(schema_of(&stored), "entity-only")
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
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
    let status = |connection_id: &str| {
        incoming
            .iter()
            .find(|reference| {
                reference
                    .connection_id
                    .as_ref()
                    .is_some_and(|id| id.value == connection_id)
            })
            .unwrap()
            .business_property_status
    };
    assert_eq!(
        status(&resolved_id),
        Some(pb::BusinessPropertyReferenceStatus::PropertyResolved as i32)
    );
    assert_eq!(
        status(&entity_only_id),
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_accept_changed_source_revalidates_on_rebase_when_the_target_shape_changes() {
    let namespace = "bd-rebase-changed";
    let branch = "bd-rebase-changed";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let stored = put(&service, order_targeting_sku(namespace)).await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: branch.into(),
            doc: "changed source".into(),
        }))
        .await
        .unwrap();
    let mut edited = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut edited.kind {
        schema.title = "Branch title".into();
    }
    service
        .put_entity(with_branch(
            Request::new(pb::PutEntityRequest {
                entity: Some(edited),
                ..Default::default()
            }),
            branch,
        ))
        .await
        .unwrap();
    put(&service, product_without_sku(namespace)).await;
    let error = service
        .update_branch(Request::new(pb::UpdateBranchRequest {
            name: branch.into(),
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error:?}");
    let still = service
        .get_entity(with_branch(
            Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }),
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(schema_of(&still).title, "Branch title");
    assert_eq!(
        schema_of(&still).property_relationships[0]
            .target_property
            .as_ref()
            .unwrap()
            .segments[0]
            .name,
        "sku"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_accept_deleted_connection_id_is_not_revived_and_the_sibling_stays() {
    let namespace = "bd-revive";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
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
            .push(labeled_relationship(namespace, "drop", None));
        schema
            .property_relationships
            .push(labeled_relationship(namespace, "keep", None));
    }
    let stored = put(&service, order).await;
    let dropped_id = relationship_by_label(schema_of(&stored), "drop")
        .connection_id
        .clone();
    let kept_id = relationship_by_label(schema_of(&stored), "keep")
        .connection_id
        .clone();
    let mut without = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut without.kind {
        schema.property_relationships =
            vec![labeled_relationship(namespace, "keep", kept_id.clone())];
    }
    let remaining = put(&service, without).await;
    assert_eq!(schema_of(&remaining).property_relationships.len(), 1);
    assert_eq!(
        schema_of(&remaining).property_relationships[0].connection_id,
        kept_id
    );
    let mut revived = remaining.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut revived.kind {
        schema
            .property_relationships
            .push(labeled_relationship(namespace, "drop", dropped_id));
    }
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(revived),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error:?}");
    let fetched = get(&service, namespace, "order", 1).await;
    assert_eq!(schema_of(&fetched).property_relationships.len(), 1);
    assert_eq!(
        schema_of(&fetched).property_relationships[0].connection_id,
        kept_id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_accept_conflict_restore_renews_only_the_restored_occurrence() {
    let namespace = "bd-conflict-ids";
    let branch = "bd-conflict-ids";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
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
            .push(labeled_relationship(namespace, "restored", None));
        schema
            .property_relationships
            .push(labeled_relationship(namespace, "kept", None));
    }
    let stored = put(&service, order).await;
    let restored_id = relationship_by_label(schema_of(&stored), "restored")
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let kept_id = relationship_by_label(schema_of(&stored), "kept")
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: branch.into(),
            doc: "restore one occurrence".into(),
        }))
        .await
        .unwrap();
    let mut branched = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut branched.kind {
        schema.title = "Branch title".into();
    }
    service
        .put_entity(with_branch(
            Request::new(pb::PutEntityRequest {
                entity: Some(branched),
                ..Default::default()
            }),
            branch,
        ))
        .await
        .unwrap();
    let mut baseline = stored;
    if let Some(pb::entity::Kind::Schema(schema)) = &mut baseline.kind {
        schema.title = "Baseline title".into();
        let kept = relationship_by_label(schema, "kept").clone();
        schema.property_relationships = vec![kept];
    }
    put(&service, baseline).await;
    let merged = service
        .merge_branch(Request::new(pb::MergeBranchRequest {
            name: branch.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Conflicts as i32,
        "{merged:?}"
    );
    service
        .resolve_branch_entry(Request::new(pb::ResolveBranchEntryRequest {
            name: branch.into(),
            r#ref: Some(pb::EntityRef {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }),
            resolution: pb::resolve_branch_entry_request::Resolution::KeepOurs as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let resolved = service
        .get_entity(with_branch(
            Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }),
            branch,
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let resolved_schema = schema_of(&resolved);
    assert_eq!(resolved_schema.property_relationships.len(), 2);
    assert_eq!(
        relationship_by_label(resolved_schema, "kept")
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        kept_id
    );
    let renewed = relationship_by_label(resolved_schema, "restored")
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_ne!(renewed, restored_id);
    assert_ne!(renewed, kept_id);
    assert_ne!(renewed, "");
    let baseline = get(&service, namespace, "order", 1).await;
    assert_eq!(schema_of(&baseline).title, "Baseline title");
    assert_eq!(schema_of(&baseline).property_relationships.len(), 1);
    assert_eq!(
        schema_of(&baseline).property_relationships[0]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        kept_id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn review_accept_prepared_owner_rejects_the_next_identifier_relationship_and_oversized_schema(
) {
    let namespace = "bd-bounds";
    let service = service().await;
    let names: Vec<String> = (0..1_024).map(|index| format!("f{index}")).collect();
    let mut schema = pb::Schema {
        title: "Bounded".into(),
        fields: names.iter().map(|name| string_field(name)).collect(),
        ..Default::default()
    };
    schema.identifiers = names
        .iter()
        .enumerate()
        .map(|(index, name)| pb::BusinessIdentifier {
            property: Some(path(&[(name.as_str(), false)])),
            role: if index == 0 {
                pb::BusinessIdentifierRole::Primary as i32
            } else {
                pb::BusinessIdentifierRole::Alternate as i32
            },
            ..Default::default()
        })
        .collect();
    let stored = put(&service, schema_entity(namespace, "bounded", 1, schema)).await;
    assert_eq!(schema_of(&stored).identifiers.len(), 1_024);
    assert!(schema_of(&stored).identifiers.iter().all(|identifier| {
        identifier
            .connection_id
            .as_ref()
            .is_some_and(|id| !id.value.is_empty())
    }));
    let mut too_many = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut too_many.kind {
        schema.fields.push(string_field("f1024"));
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("f1024", false)])),
            role: pb::BusinessIdentifierRole::Alternate as i32,
            ..Default::default()
        });
    }
    let rejected = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(too_many),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected:?}");
    assert_eq!(
        schema_of(&get(&service, namespace, "bounded", 1).await)
            .identifiers
            .len(),
        1_024
    );

    put(&service, product_schema(namespace, 1)).await;
    let related = put(&service, fan_in(namespace, "related", 1_024)).await;
    assert_eq!(schema_of(&related).property_relationships.len(), 1_024);
    assert!(schema_of(&related)
        .property_relationships
        .iter()
        .all(|relationship| {
            relationship
                .connection_id
                .as_ref()
                .is_some_and(|id| !id.value.is_empty())
        }));
    let mut extra = related;
    if let Some(pb::entity::Kind::Schema(schema)) = &mut extra.kind {
        schema
            .property_relationships
            .push(labeled_relationship(namespace, "extra", None));
    }
    let rejected = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(extra),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected:?}");
    assert_eq!(
        schema_of(&get(&service, namespace, "related", 1).await)
            .property_relationships
            .len(),
        1_024
    );

    let mut huge = schema_entity(
        namespace,
        "huge",
        1,
        pb::Schema {
            title: "Huge".into(),
            fields: vec![string_field("sku")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut huge.kind {
        schema.fields[0].doc = "x".repeat(trogon_atlas_core::business_data::MAX_SCHEMA_BYTES);
    }
    let rejected = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(huge),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument, "{rejected:?}");
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "huge", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

/// `2^53 + 1`, above JavaScript's exact integer range.
const LARGE_VERSION: u64 = 9_007_199_254_740_993;
const BUSINESS_WRITER: &str = "business-writer-token";

type BusinessClient = EventModelServiceClient<Channel>;

#[derive(Clone)]
struct StoredMoment {
    entity: pb::Entity,
    etag: String,
}

struct OrderFixture {
    namespace: &'static str,
    product_id: pb::Id,
    order_uid: String,
    product: StoredMoment,
    order: StoredMoment,
    lane: StoredMoment,
    event: StoredMoment,
    row: StoredMoment,
}

#[derive(Clone, Copy)]
enum PayloadBusiness {
    Aggregate(bool),
    Identifier,
    Relationship,
    All,
}

fn order_shape(product_id: pb::Id) -> Vec<pb::FieldSpec> {
    vec![
        object_field("customer", false, vec![string_field("id")]),
        object_field("lines", true, vec![string_field("sku")]),
        schema_ref_field("product_ref", product_id),
        string_field("order_id"),
    ]
}

fn primary_on(name: &str) -> pb::BusinessIdentifier {
    pb::BusinessIdentifier {
        property: Some(path(&[(name, false)])),
        role: pb::BusinessIdentifierRole::Primary as i32,
        ..Default::default()
    }
}

fn customer_identifier() -> pb::BusinessIdentifier {
    pb::BusinessIdentifier {
        property: Some(path(&[("customer", false), ("id", false)])),
        role: pb::BusinessIdentifierRole::Primary as i32,
        ..Default::default()
    }
}

fn product_relationship(product_id: &pb::Id) -> pb::BusinessPropertyRelationship {
    pb::BusinessPropertyRelationship {
        source: Some(path(&[("product_ref", false)])),
        target_entity: Some(pb::SchemaRef {
            id: Some(product_id.clone()),
        }),
        ..Default::default()
    }
}

fn order_relationship(product_id: &pb::Id) -> pb::BusinessPropertyRelationship {
    pb::BusinessPropertyRelationship {
        source: Some(path(&[("order_id", false)])),
        target_entity: Some(pb::SchemaRef {
            id: Some(product_id.clone()),
        }),
        ..Default::default()
    }
}

fn order_identity_annotation() -> prost_types::Any {
    let annotation = pb::Uuidv5IdentityAnnotation {
        name: "order_id".into(),
        template: "orders/{order_id}".into(),
        version: 1,
        namespace: Some(pb::uuidv5_identity_annotation::Namespace::Dns(
            "orders.example".into(),
        )),
    };
    prost_types::Any {
        type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Uuidv5IdentityAnnotation"
            .into(),
        value: annotation.encode_to_vec(),
    }
}

fn id_only_fields() -> pb::Schema {
    pb::Schema {
        fields: vec![string_field("order_id")],
        ..Default::default()
    }
}

fn id_only(order_id: pb::Id) -> pb::Schema {
    let mut schema = id_only_fields();
    schema.id = Some(order_id);
    schema
}

fn business_payload(
    order_id: Option<pb::Id>,
    product_id: &pb::Id,
    business: PayloadBusiness,
) -> pb::Schema {
    let mut schema = id_only_fields();
    schema.id = order_id;
    match business {
        PayloadBusiness::Aggregate(value) => schema.aggregate_root = Some(value),
        PayloadBusiness::Identifier => schema.identifiers.push(primary_on("order_id")),
        PayloadBusiness::Relationship => {
            schema
                .property_relationships
                .push(order_relationship(product_id));
        }
        PayloadBusiness::All => {
            schema.aggregate_root = Some(true);
            schema.identifiers.push(primary_on("order_id"));
            schema
                .property_relationships
                .push(order_relationship(product_id));
        }
    }
    schema
}

fn uid_of(entity: &pb::Entity) -> &str {
    &entity.system.as_ref().unwrap().uid
}

fn returned_id(moment: &StoredMoment) -> pb::Id {
    trogon_atlas_core::refs::entity_id(&moment.entity)
        .unwrap()
        .clone()
}

fn payload_of(entity: &pb::Entity) -> pb::Schema {
    let any = trogon_atlas_core::schema::entity_schema(entity).unwrap();
    pb::Schema::decode(any.value.as_slice()).unwrap()
}

fn with_aggregate(entity: &pb::Entity, value: Option<bool>) -> pb::Entity {
    let mut entity = entity.clone();
    match &mut entity.kind {
        Some(pb::entity::Kind::Schema(schema)) => schema.aggregate_root = value,
        other => panic!("schema, got {other:?}"),
    }
    entity
}

fn with_declarations(entity: &pb::Entity, product_id: &pb::Id) -> pb::Entity {
    let mut entity = with_aggregate(entity, Some(true));
    match &mut entity.kind {
        Some(pb::entity::Kind::Schema(schema)) => {
            schema.identifiers.push(customer_identifier());
            schema
                .property_relationships
                .push(product_relationship(product_id));
        }
        other => panic!("schema, got {other:?}"),
    }
    entity
}

fn with_payload(entity: &pb::Entity, schema: &pb::Schema) -> pb::Entity {
    let packed = Some(trogon_atlas_core::schema::pack_schema(schema));
    let mut entity = entity.clone();
    match &mut entity.kind {
        Some(pb::entity::Kind::Event(event)) => event.schema = packed,
        Some(pb::entity::Kind::Command(command)) => command.schema = packed,
        Some(pb::entity::Kind::ReadModel(row)) => row.schema = packed,
        other => panic!("payload owner, got {other:?}"),
    }
    entity
}

fn pack_owner(kind: pb::EntityKind, owner: pb::Id, schema: &pb::Schema) -> pb::Entity {
    let title = owner.slug.clone();
    let packed = Some(trogon_atlas_core::schema::pack_schema(schema));
    let body = match kind {
        pb::EntityKind::Event => pb::entity::Kind::Event(pb::Event {
            id: Some(owner),
            title,
            schema: packed,
            ..Default::default()
        }),
        pb::EntityKind::Command => pb::entity::Kind::Command(pb::Command {
            id: Some(owner),
            title,
            schema: packed,
            ..Default::default()
        }),
        pb::EntityKind::ReadModel => pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(owner),
            title,
            key: vec!["order_id".into()],
            schema: packed,
            ..Default::default()
        }),
        other => panic!("payload kind {other:?}"),
    };
    pb::Entity {
        kind: Some(body),
        ..Default::default()
    }
}

fn reject_slug(kind: pb::EntityKind, business: PayloadBusiness, with_id: bool) -> String {
    let kind = match kind {
        pb::EntityKind::Event => "event",
        pb::EntityKind::Command => "command",
        pb::EntityKind::ReadModel => "read-model",
        other => panic!("payload kind {other:?}"),
    };
    let business = match business {
        PayloadBusiness::Aggregate(true) => "root-true",
        PayloadBusiness::Aggregate(false) => "root-false",
        PayloadBusiness::Identifier => "identifier",
        PayloadBusiness::Relationship => "relationship",
        PayloadBusiness::All => "all",
    };
    let id_mode = if with_id { "id" } else { "noid" };
    format!("rej-{kind}-{business}-{id_mode}")
}

fn moment_kind_filter() -> Vec<i32> {
    [
        pb::EntityKind::Event,
        pb::EntityKind::Command,
        pb::EntityKind::ReadModel,
        pb::EntityKind::Swimlane,
        pb::EntityKind::CommandSlice,
        pb::EntityKind::ReadModelSlice,
        pb::EntityKind::AutomationSlice,
        pb::EntityKind::UiSlice,
    ]
    .into_iter()
    .map(|kind| kind as i32)
    .collect()
}

fn initial_moment_keys(namespace: &str) -> Vec<(i32, String, String, u64)> {
    vec![
        (
            pb::EntityKind::Event as i32,
            namespace.to_owned(),
            "OrderPlaced".into(),
            LARGE_VERSION,
        ),
        (
            pb::EntityKind::ReadModel as i32,
            namespace.to_owned(),
            "order-row".into(),
            LARGE_VERSION,
        ),
        (
            pb::EntityKind::Swimlane as i32,
            namespace.to_owned(),
            "orders".into(),
            LARGE_VERSION,
        ),
    ]
}

async fn put_and_read(client: &mut BusinessClient, entity: pb::Entity) -> StoredMoment {
    let written = client
        .put_entity(with_token(
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
            BUSINESS_WRITER,
        ))
        .await
        .unwrap()
        .into_inner();
    let stored = written.entity.unwrap();
    let kind = trogon_atlas_core::refs::entity_kind(&stored).unwrap();
    let stored_id = trogon_atlas_core::refs::entity_id(&stored).unwrap().clone();
    assert_eq!(stored_id.version, LARGE_VERSION);
    assert_ne!(written.etag, "");
    assert_ne!(uid_of(&stored), "");
    let read = client
        .get_entity(with_token(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(stored_id),
            },
            BUSINESS_WRITER,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.etag, written.etag);
    assert_eq!(read.entity.as_ref(), Some(&stored));
    StoredMoment {
        entity: read.entity.unwrap(),
        etag: read.etag,
    }
}

async fn assert_held(client: &mut BusinessClient, before: &StoredMoment) {
    let kind = trogon_atlas_core::refs::entity_kind(&before.entity).unwrap();
    let stored_id = trogon_atlas_core::refs::entity_id(&before.entity)
        .unwrap()
        .clone();
    let read = client
        .get_entity(with_token(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(stored_id),
            },
            BUSINESS_WRITER,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read.etag, before.etag);
    assert_eq!(read.entity.as_ref(), Some(&before.entity));
}

async fn assert_absent(
    client: &mut BusinessClient,
    kind: pb::EntityKind,
    namespace: &str,
    slug: &str,
    version: u64,
) {
    let missing = client
        .get_entity(with_token(
            pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(id(namespace, slug, version)),
            },
            BUSINESS_WRITER,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        missing.code(),
        Code::NotFound,
        "{kind:?} {slug}@{version}: {missing:?}"
    );
}

async fn assert_rejected(client: &mut BusinessClient, entity: pb::Entity) {
    let kind = trogon_atlas_core::refs::entity_kind(&entity).unwrap();
    let stored_id = trogon_atlas_core::refs::entity_id(&entity).unwrap().clone();
    let error = client
        .put_entity(with_token(
            pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            },
            BUSINESS_WRITER,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        Code::InvalidArgument,
        "{kind:?} {}: {error:?}",
        stored_id.slug
    );
    assert_eq!(
        error.message(),
        "business declarations are not allowed on a payload schema",
        "{error:?}"
    );
}

async fn moment_keys(
    client: &mut BusinessClient,
    namespace: &str,
) -> Vec<(i32, String, String, u64)> {
    let listed = client
        .list_entities(with_token(
            pb::ListEntitiesRequest {
                kinds: moment_kind_filter(),
                namespaces: vec![namespace.to_owned()],
                page_size: 100,
                ..Default::default()
            },
            BUSINESS_WRITER,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.next_page_token, "");
    let mut keys = Vec::new();
    for entity in listed.entities {
        let stored_id = trogon_atlas_core::refs::entity_id(&entity).unwrap();
        let kind = trogon_atlas_core::refs::entity_kind(&entity).unwrap() as i32;
        keys.push((
            kind,
            stored_id.namespace.clone(),
            stored_id.slug.clone(),
            stored_id.version,
        ));
    }
    keys.sort();
    keys
}

fn assert_declares_order_without_copying(
    entity: &pb::Entity,
    order_id: &pb::Id,
    addressable: &pb::Schema,
) {
    let payload = payload_of(entity);
    assert_eq!(payload.id.as_ref(), Some(order_id));
    assert_eq!(payload.aggregate_root, None);
    assert_eq!(payload.identifiers.len(), 0);
    assert_eq!(payload.property_relationships.len(), 0);
    assert_eq!(payload.fields, vec![string_field("order_id")]);
    assert_eq!(
        trogon_atlas_core::schema::declared_schema_id(entity).as_ref(),
        Some(order_id)
    );
    assert_eq!(addressable.aggregate_root, Some(true));
    assert_ne!(addressable.identifiers.len(), 0);
    assert_ne!(addressable.property_relationships.len(), 0);
    assert_ne!(payload.aggregate_root, addressable.aggregate_root);
    assert_ne!(payload.identifiers, addressable.identifiers);
    assert_ne!(
        payload.property_relationships,
        addressable.property_relationships
    );
}

async fn prepare_order_fixture(client: &mut BusinessClient) -> OrderFixture {
    let namespace = "bd-order-fixture";
    let product = put_and_read(
        client,
        schema_entity(
            namespace,
            "product",
            LARGE_VERSION,
            pb::Schema {
                title: "Product".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    let product_id = returned_id(&product);
    assert_eq!(product_id, id(namespace, "product", LARGE_VERSION));
    let order = put_and_read(
        client,
        schema_entity(
            namespace,
            "order",
            LARGE_VERSION,
            pb::Schema {
                title: "Order".into(),
                fields: order_shape(product_id.clone()),
                ..Default::default()
            },
        ),
    )
    .await;
    let order_uid = uid_of(&order.entity).to_owned();
    let lane = put_and_read(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(id(namespace, "orders", LARGE_VERSION)),
                title: "orders".into(),
                stream_id: "orders-{order_id}".into(),
                state: vec![string_field("order_id")],
                metadata: vec![order_identity_annotation()],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let lane_id = returned_id(&lane);
    assert_eq!(lane_id, id(namespace, "orders", LARGE_VERSION));
    let lane_body = match &lane.entity.kind {
        Some(pb::entity::Kind::Swimlane(body)) => body,
        other => panic!("swimlane, got {other:?}"),
    };
    assert_eq!(lane_body.stream_id, "orders-{order_id}");
    assert_eq!(lane_body.state, vec![string_field("order_id")]);
    assert_eq!(lane_body.transitions.len(), 0);
    assert_eq!(lane_body.metadata.len(), 1);
    let annotation =
        pb::Uuidv5IdentityAnnotation::decode(lane_body.metadata[0].value.as_slice()).unwrap();
    assert_eq!(annotation.name, "order_id");
    let event = put_and_read(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(namespace, "OrderPlaced", LARGE_VERSION)),
                title: "OrderPlaced".into(),
                swimlane: Some(pb::SwimlaneRef { id: Some(lane_id) }),
                schema: Some(trogon_atlas_core::schema::pack_fields(vec![string_field(
                    "order_id",
                )])),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let event_payload = payload_of(&event.entity);
    assert_eq!(event_payload.fields, vec![string_field("order_id")]);
    assert_eq!(event_payload.id, None);
    assert_eq!(event_payload.aggregate_root, None);
    assert!(trogon_atlas_core::schema::declared_schema_id(&event.entity).is_none());
    let row = put_and_read(
        client,
        pb::Entity {
            kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                id: Some(id(namespace, "order-row", LARGE_VERSION)),
                title: "order-row".into(),
                key: vec!["order_id".into()],
                schema: Some(trogon_atlas_core::schema::pack_fields(vec![string_field(
                    "order_id",
                )])),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let row_body = match &row.entity.kind {
        Some(pb::entity::Kind::ReadModel(body)) => body,
        other => panic!("read model, got {other:?}"),
    };
    assert_eq!(row_body.key, vec!["order_id".to_string()]);
    assert_eq!(payload_of(&row.entity).id, None);
    assert_held(client, &order).await;
    assert_eq!(schema_of(&order.entity).aggregate_root, None);
    assert_eq!(schema_of(&order.entity).identifiers.len(), 0);
    assert_eq!(schema_of(&order.entity).property_relationships.len(), 0);
    assert_eq!(
        moment_keys(client, namespace).await,
        initial_moment_keys(namespace)
    );
    OrderFixture {
        namespace,
        product_id,
        order_uid,
        product,
        order,
        lane,
        event,
        row,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_order_fixture_keeps_business_declarations_explicit() {
    let mut client = authenticated_business_server().await;
    let mut fixture = prepare_order_fixture(&mut client).await;

    let mut flagged = fixture.product.entity.clone();
    match &mut flagged.kind {
        Some(pb::entity::Kind::Schema(schema)) => {
            schema.aggregate_root = Some(true);
            schema.identifiers.push(primary_on("sku"));
        }
        other => panic!("schema, got {other:?}"),
    }
    let product_uid = uid_of(&fixture.product.entity).to_owned();
    fixture.product = put_and_read(&mut client, flagged).await;
    assert_eq!(
        schema_of(&fixture.product.entity).aggregate_root,
        Some(true)
    );
    assert_eq!(uid_of(&fixture.product.entity), product_uid);
    assert_eq!(returned_id(&fixture.product), fixture.product_id);
    let product_identifier = schema_of(&fixture.product.entity).identifiers[0]
        .connection_id
        .clone()
        .unwrap();
    assert_ne!(product_identifier.value, "");
    assert_held(&mut client, &fixture.order).await;
    assert_held(&mut client, &fixture.lane).await;
    assert_held(&mut client, &fixture.event).await;
    assert_held(&mut client, &fixture.row).await;
    assert_eq!(schema_of(&fixture.order.entity).aggregate_root, None);
    assert_eq!(schema_of(&fixture.order.entity).identifiers.len(), 0);
    assert_eq!(
        schema_of(&fixture.order.entity).fields,
        order_shape(fixture.product_id.clone())
    );

    for value in [Some(true), Some(false), None] {
        let next = with_aggregate(&fixture.order.entity, value);
        let previous_etag = fixture.order.etag.clone();
        fixture.order = put_and_read(&mut client, next).await;
        assert_eq!(schema_of(&fixture.order.entity).aggregate_root, value);
        assert_ne!(fixture.order.etag, previous_etag);
        assert_eq!(uid_of(&fixture.order.entity), fixture.order_uid);
        assert_eq!(schema_of(&fixture.order.entity).identifiers.len(), 0);
        assert_eq!(
            schema_of(&fixture.order.entity)
                .property_relationships
                .len(),
            0
        );
        assert_held(&mut client, &fixture.lane).await;
        assert_held(&mut client, &fixture.event).await;
        assert_held(&mut client, &fixture.row).await;
        assert_eq!(
            moment_keys(&mut client, fixture.namespace).await,
            initial_moment_keys(fixture.namespace)
        );
        if value == Some(true) {
            for (kind, slug, version) in [
                (pb::EntityKind::Swimlane, "order", LARGE_VERSION),
                (pb::EntityKind::Swimlane, "order", 1),
                (pb::EntityKind::Event, "order", LARGE_VERSION),
                (pb::EntityKind::CommandSlice, "order", LARGE_VERSION),
                (pb::EntityKind::ReadModelSlice, "order-row", LARGE_VERSION),
                (pb::EntityKind::AutomationSlice, "orders", LARGE_VERSION),
                (pb::EntityKind::UiSlice, "orders", LARGE_VERSION),
            ] {
                assert_absent(&mut client, kind, fixture.namespace, slug, version).await;
            }
        }
    }

    let product_id = fixture.product_id.clone();
    let next = with_declarations(&fixture.order.entity, &product_id);
    let previous_etag = fixture.order.etag.clone();
    fixture.order = put_and_read(&mut client, next).await;
    assert_ne!(fixture.order.etag, previous_etag);
    assert_eq!(uid_of(&fixture.order.entity), fixture.order_uid);
    let addressable = schema_of(&fixture.order.entity).clone();
    assert_eq!(addressable.aggregate_root, Some(true));
    let identifier = addressable.identifiers[0].connection_id.clone().unwrap();
    let relationship = addressable.property_relationships[0]
        .connection_id
        .clone()
        .unwrap();
    assert_ne!(identifier.value, "");
    assert_ne!(relationship.value, "");
    assert_ne!(identifier.value, relationship.value);
    assert_held(&mut client, &fixture.lane).await;
    assert_held(&mut client, &fixture.event).await;
    assert_held(&mut client, &fixture.row).await;
    assert_eq!(
        moment_keys(&mut client, fixture.namespace).await,
        initial_moment_keys(fixture.namespace)
    );

    let order_id = returned_id(&fixture.order);
    assert_eq!(order_id, id(fixture.namespace, "order", LARGE_VERSION));
    let poisoned = with_payload(
        &fixture.event.entity,
        &business_payload(None, &fixture.product_id, PayloadBusiness::Aggregate(true)),
    );
    assert_rejected(&mut client, poisoned).await;
    assert_held(&mut client, &fixture.event).await;

    let event_uid = uid_of(&fixture.event.entity).to_owned();
    let cited = with_payload(&fixture.event.entity, &id_only(order_id.clone()));
    fixture.event = put_and_read(&mut client, cited).await;
    assert_eq!(uid_of(&fixture.event.entity), event_uid);
    assert_declares_order_without_copying(&fixture.event.entity, &order_id, &addressable);
    assert_held(&mut client, &fixture.lane).await;
    assert_held(&mut client, &fixture.row).await;

    let poisoned = with_payload(
        &fixture.event.entity,
        &business_payload(
            Some(order_id.clone()),
            &fixture.product_id,
            PayloadBusiness::Relationship,
        ),
    );
    assert_rejected(&mut client, poisoned).await;
    assert_held(&mut client, &fixture.event).await;

    let poisoned = with_payload(
        &fixture.row.entity,
        &business_payload(None, &fixture.product_id, PayloadBusiness::Identifier),
    );
    assert_rejected(&mut client, poisoned).await;
    assert_held(&mut client, &fixture.row).await;
    let row_uid = uid_of(&fixture.row.entity).to_owned();
    let cited = with_payload(&fixture.row.entity, &id_only(order_id.clone()));
    fixture.row = put_and_read(&mut client, cited).await;
    assert_eq!(uid_of(&fixture.row.entity), row_uid);
    assert_declares_order_without_copying(&fixture.row.entity, &order_id, &addressable);
    let row_body = match &fixture.row.entity.kind {
        Some(pb::entity::Kind::ReadModel(body)) => body,
        other => panic!("read model, got {other:?}"),
    };
    assert_eq!(row_body.key, vec!["order_id".to_string()]);
    assert_held(&mut client, &fixture.lane).await;

    let command = put_and_read(
        &mut client,
        pack_owner(
            pb::EntityKind::Command,
            id(fixture.namespace, "place-order", LARGE_VERSION),
            &id_only(order_id.clone()),
        ),
    )
    .await;
    assert_declares_order_without_copying(&command.entity, &order_id, &addressable);
    assert_held(&mut client, &fixture.lane).await;
    assert_held(&mut client, &fixture.event).await;
    assert_held(&mut client, &fixture.row).await;
    let poisoned = with_payload(
        &command.entity,
        &business_payload(None, &fixture.product_id, PayloadBusiness::All),
    );
    assert_rejected(&mut client, poisoned).await;
    assert_held(&mut client, &command).await;

    for kind in [
        pb::EntityKind::Event,
        pb::EntityKind::Command,
        pb::EntityKind::ReadModel,
    ] {
        for with_id in [false, true] {
            for business in [
                PayloadBusiness::Aggregate(true),
                PayloadBusiness::Aggregate(false),
                PayloadBusiness::Identifier,
                PayloadBusiness::Relationship,
                PayloadBusiness::All,
            ] {
                let slug = reject_slug(kind, business, with_id);
                let schema_id = with_id.then(|| order_id.clone());
                let entity = pack_owner(
                    kind,
                    id(fixture.namespace, &slug, LARGE_VERSION),
                    &business_payload(schema_id, &fixture.product_id, business),
                );
                assert_rejected(&mut client, entity).await;
                assert_absent(&mut client, kind, fixture.namespace, &slug, LARGE_VERSION).await;
            }
        }
    }
    assert_held(&mut client, &fixture.lane).await;
    assert_held(&mut client, &fixture.order).await;
    assert_eq!(schema_of(&fixture.order.entity).aggregate_root, Some(true));
    assert_eq!(
        schema_of(&fixture.order.entity).identifiers[0]
            .connection_id
            .as_ref(),
        Some(&identifier)
    );
    assert_eq!(
        schema_of(&fixture.order.entity).property_relationships[0]
            .connection_id
            .as_ref(),
        Some(&relationship)
    );
}
