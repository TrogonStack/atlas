//! Store-backed acceptance for native Schema business declarations.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use prost::Message;
use tokio::{net::TcpListener, sync::broadcast};
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
    Request, Status,
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
use trogon_atlas_store::{
    store::{
        BranchDeltaEntry, BranchInfo, BranchLandOp, ChangeRecord, ChangeStreamStats, ListFilter,
        MutationOp, MutationOutcome, Store, Written,
    },
    ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef, EntityRevisionRecord,
    NamespaceRecord, NatsStore, RevisionPage, StoreError, StoredEntity, WriteContext,
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
    let (target, etag) = read_stored(
        &service,
        pb::EntityKind::Schema,
        id(namespace, "product", 1),
    )
    .await;
    let refused = service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "product", 1)),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    let (after, after_etag) = read_stored(
        &service,
        pb::EntityKind::Schema,
        id(namespace, "product", 1),
    )
    .await;
    assert_eq!(after, target);
    assert_eq!(after_etag, etag);
}

#[tokio::test(flavor = "multi_thread")]
async fn assigned_business_ids_cannot_grow_a_schema_past_its_byte_ceiling() {
    let namespace = "bd-assigned-size";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut proposed = fan_in(namespace, "oversized-after-ids", 1);
    let pb::entity::Kind::Schema(schema) = proposed.kind.as_mut().unwrap() else {
        panic!("schema")
    };
    let ceiling = trogon_atlas_core::business_data::MAX_SCHEMA_BYTES;
    schema.fields[0].doc = "x".repeat(ceiling - 512);
    let length = schema.encoded_len();
    schema.fields[0].doc.push_str(&"x".repeat(ceiling - length));
    assert_eq!(schema.encoded_len(), ceiling);
    assert!(schema.property_relationships[0].connection_id.is_none());
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(proposed),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(
        error.message().contains("1048576") || error.message().contains("1 MiB"),
        "{error}"
    );
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "oversized-after-ids", 1))
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
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

fn packed_annotation(type_name: &str, bytes: Vec<u8>) -> prost_types::Any {
    prost_types::Any {
        type_url: format!("type.googleapis.com/trogonatlas.eventmodel.v1alpha1.{type_name}"),
        value: bytes,
    }
}

fn uuid_field(name: &str) -> pb::FieldSpec {
    pb::FieldSpec {
        name: name.into(),
        r#type: Some(pb::FieldType {
            kind: Some(field_type::Kind::Uuid(field_type::UuidType { version: 7 })),
        }),
        metadata: vec![
            packed_annotation(
                "DerivedFieldAnnotation",
                pb::DerivedFieldAnnotation {
                    doc: "runtime uuid".into(),
                    kind: pb::derived_field_annotation::Kind::Uuid as i32,
                }
                .encode_to_vec(),
            ),
            packed_annotation(
                "DomainTagAnnotation",
                pb::DomainTagAnnotation {
                    tag: "shop.order-id".into(),
                }
                .encode_to_vec(),
            ),
            packed_annotation(
                "PersonalDataAnnotation",
                pb::PersonalDataAnnotation {
                    doc: "customer token".into(),
                    erasure: pb::personal_data_annotation::Erasure::CryptoShredding as i32,
                    subject_field: "order_id".into(),
                }
                .encode_to_vec(),
            ),
        ],
        ..Default::default()
    }
}

async fn expect_unchanged(
    service: &EventModelServiceImpl,
    namespace: &str,
    slug: &str,
    version: u64,
    proposed: pb::Entity,
    fragment: &str,
) {
    let stored = get(service, namespace, slug, version).await;
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(proposed),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    assert!(error.message().contains(fragment), "{error}");
    assert_eq!(get(service, namespace, slug, version).await, stored);
}

async fn expect_unpublished(
    service: &EventModelServiceImpl,
    namespace: &str,
    slug: &str,
    proposed: pb::Entity,
    code: Code,
    fragment: &str,
) {
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(proposed),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), code, "{error}");
    assert!(error.message().contains(fragment), "{error}");
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, slug, 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}

fn order_with(
    namespace: &str,
    identifiers: Vec<pb::BusinessIdentifier>,
    relationships: Vec<pb::BusinessPropertyRelationship>,
) -> pb::Entity {
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
        schema.identifiers = identifiers;
        schema.property_relationships = relationships;
    }
    order
}

#[tokio::test(flavor = "multi_thread")]
async fn public_identifier_and_relationship_rules_reject_without_publishing() {
    let namespace = "bd-rules";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let stored = put(
        &service,
        order_with(
            namespace,
            vec![
                pb::BusinessIdentifier {
                    property: Some(path(&[("customer", false), ("id", false)])),
                    role: pb::BusinessIdentifierRole::Primary as i32,
                    ..Default::default()
                },
                pb::BusinessIdentifier {
                    property: Some(path(&[("lines", true), ("sku", false)])),
                    role: pb::BusinessIdentifierRole::Alternate as i32,
                    ..Default::default()
                },
            ],
            vec![relationship(namespace, 1, None, None)],
        ),
    )
    .await;
    let schema = schema_of(&stored);
    assert!(schema.identifiers[0].connection_id.is_some());
    assert!(schema.identifiers[1].connection_id.is_some());
    assert_ne!(
        schema.identifiers[0].connection_id,
        schema.identifiers[1].connection_id
    );
    assert!(schema.property_relationships[0].target_property.is_none());
    assert!(schema.property_relationships[0].label.is_none());
    let primary_id = connection(&stored, false);
    let alternate_id = schema.identifiers[1]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let relationship_id = connection(&stored, true);
    let mut product = get(&service, namespace, "product", 1).await;
    if let Some(pb::entity::Kind::Schema(body)) = &mut product.kind {
        body.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("sku", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    put(&service, product).await;
    let order = get(&service, namespace, "order", 1).await;
    let schema = schema_of(&order);
    assert!(schema.property_relationships[0].target_property.is_none());
    assert_eq!(schema.identifiers.len(), 2);
    assert_eq!(connection(&order, false), primary_id);
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
            .find(|reference| reference.from_field == "property_relationships[0].target_entity")
            .unwrap()
            .business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32)
    );
    assert!(!outgoing
        .iter()
        .any(|reference| reference.from_field.contains("identifiers")));

    let mut documented = order.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut documented.kind {
        body.identifiers[0].doc = Some(pb::CatalogText {
            value: "Customer id".into(),
        });
        body.property_relationships[0].label = Some(pb::CatalogTitle {
            value: "hasMany".into(),
        });
        body.property_relationships[0].doc = Some(pb::CatalogText {
            value: "Line product".into(),
        });
    }
    let documented = put(&service, documented).await;
    assert_eq!(connection(&documented, false), primary_id);
    assert_eq!(connection(&documented, true), relationship_id);
    assert_eq!(
        schema_of(&documented).property_relationships[0]
            .label
            .as_ref()
            .unwrap()
            .value,
        "hasMany"
    );
    let mut second = documented.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut second.kind {
        body.property_relationships
            .push(relationship(namespace, 1, None, Some("belongsTo")));
    }
    let second = put(&service, second).await;
    assert_eq!(connection(&second, true), relationship_id);
    assert_ne!(
        schema_of(&second).property_relationships[1].connection_id,
        schema_of(&second).property_relationships[0].connection_id
    );
    let belongs_to = schema_of(&second).property_relationships[1]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    let mut reordered = second.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut reordered.kind {
        body.fields.reverse();
    }
    let reordered = put(&service, reordered).await;
    assert_eq!(connection(&reordered, false), primary_id);
    assert_eq!(
        schema_of(&reordered).identifiers[1]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        alternate_id
    );
    assert_eq!(connection(&reordered, true), relationship_id);
    assert_eq!(
        schema_of(&reordered).identifiers[0]
            .property
            .as_ref()
            .unwrap()
            .segments
            .iter()
            .map(|segment| segment.name.as_str())
            .collect::<Vec<_>>(),
        vec!["customer", "id"]
    );

    let mut blank_label = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut blank_label.kind {
        body.property_relationships[0].label = Some(pb::CatalogTitle { value: "  ".into() });
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        blank_label,
        "catalog text blank or byte limit exceeded",
    )
    .await;
    let mut blank_doc = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut blank_doc.kind {
        body.property_relationships[0].doc = Some(pb::CatalogText { value: " ".into() });
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        blank_doc,
        "catalog text blank or byte limit exceeded",
    )
    .await;
    let mut kept_target = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut kept_target.kind {
        body.property_relationships[0].target_property = Some(path(&[("sku", false)]));
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        kept_target,
        "changed role or endpoint",
    )
    .await;
    let mut kept_role = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut kept_role.kind {
        body.identifiers[0].role = pb::BusinessIdentifierRole::Alternate as i32;
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        kept_role,
        "changed role or endpoint",
    )
    .await;
    let mut kept_path = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut kept_path.kind {
        body.identifiers[1].property = Some(path(&[("order_id", false)]));
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        kept_path,
        "changed role or endpoint",
    )
    .await;
    let mut same_path = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut same_path.kind {
        body.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Alternate as i32,
            ..Default::default()
        });
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        same_path,
        "identifier property path is declared more than once",
    )
    .await;
    let mut repeated = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut repeated.kind {
        body.identifiers[1].connection_id = None;
        body.identifiers[1].property = Some(path(&[("lines", false), ("sku", false)]));
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        repeated,
        "property segment lines repeated flag does not match the field",
    )
    .await;
    let mut items = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut items.kind {
        body.identifiers[1].connection_id = None;
        body.identifiers[1].property =
            Some(path(&[("lines", true), ("items", false), ("sku", false)]));
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        items,
        "property segment items is missing or ambiguous",
    )
    .await;
    let mut through_ref = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut through_ref.kind {
        body.identifiers[1].connection_id = None;
        body.identifiers[1].property = Some(path(&[("product_ref", false), ("sku", false)]));
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        through_ref,
        "property segment product_ref is not an inline object",
    )
    .await;

    let mut renewed = reordered.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut renewed.kind {
        body.property_relationships[0].connection_id = None;
        body.property_relationships[0].target_property = Some(path(&[("sku", false)]));
    }
    let renewed = put(&service, renewed).await;
    let renewed_schema = schema_of(&renewed);
    let renewed_relationship = renewed_schema.property_relationships[0]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_ne!(renewed_relationship, relationship_id);
    assert_eq!(
        renewed_schema.property_relationships[1]
            .connection_id
            .as_ref()
            .unwrap()
            .value,
        belongs_to
    );
    assert_eq!(connection(&renewed, false), primary_id);
    assert_eq!(
        service
            .get_outgoing_references(Request::new(pb::GetReferencesRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
            .references
            .iter()
            .find(|reference| reference.from_field == "property_relationships[0].target_entity")
            .unwrap()
            .business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::PropertyResolved as i32)
    );
    let old_relationship = connection_subject(&renewed, &relationship_id);
    let new_relationship = connection_subject(&renewed, &renewed_relationship);
    let missing = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Team(pb::Team {
                    id: Some(id(namespace, "old-relationship", 1)),
                    title: Some(pb::DirectoryDisplayName {
                        value: "Old relationship".into(),
                    }),
                    responsibilities: vec![pb::Responsibility {
                        kind: pb::ResponsibilityKind::Owner as i32,
                        subject: Some(old_relationship.clone()),
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
    assert_eq!(missing.code(), Code::InvalidArgument, "{missing}");
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Team as i32,
                id: Some(id(namespace, "old-relationship", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Team(pb::Team {
                id: Some(id(namespace, "new-relationship", 1)),
                title: Some(pb::DirectoryDisplayName {
                    value: "New relationship".into(),
                }),
                responsibilities: vec![pb::Responsibility {
                    kind: pb::ResponsibilityKind::Owner as i32,
                    subject: Some(new_relationship.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        service
            .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
                subject: Some(old_relationship),
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
                subject: Some(new_relationship),
                kind: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .assignments
            .len(),
        1
    );

    let mut renewed_identifier = renewed.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut renewed_identifier.kind {
        body.identifiers[1].connection_id = None;
        body.identifiers[1].property = Some(path(&[("order_id", false)]));
    }
    let renewed_identifier = put(&service, renewed_identifier).await;
    let new_alternate = schema_of(&renewed_identifier).identifiers[1]
        .connection_id
        .as_ref()
        .unwrap()
        .value
        .clone();
    assert_ne!(new_alternate, alternate_id);
    assert_eq!(connection(&renewed_identifier, false), primary_id);
    let old_identifier = catalog_subject(&renewed_identifier, &alternate_id);
    let new_identifier = catalog_subject(&renewed_identifier, &new_alternate);
    let missing_decision = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Decision(pb::Decision {
                    id: Some(id(namespace, "old-identifier", 1)),
                    title: Some(pb::CatalogTitle {
                        value: "Old identifier".into(),
                    }),
                    subjects: vec![pb::SubjectAssociation {
                        subject: Some(old_identifier.clone()),
                        ..Default::default()
                    }],
                    status: pb::DecisionStatus::Proposed as i32,
                    context: Some(pb::CatalogText {
                        value: "The previous occurrence.".into(),
                    }),
                    rationale: Some(pb::CatalogText {
                        value: "A replaced path is not that occurrence.".into(),
                    }),
                    options: vec![pb::DecisionOption {
                        title: Some(pb::CatalogTitle {
                            value: "Keep the new id".into(),
                        }),
                        description: Some(pb::CatalogText {
                            value: "The old connection ref does not resolve.".into(),
                        }),
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
    assert_eq!(
        missing_decision.code(),
        Code::InvalidArgument,
        "{missing_decision}"
    );
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Decision as i32,
                id: Some(id(namespace, "old-identifier", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Decision(pb::Decision {
                id: Some(id(namespace, "new-identifier", 1)),
                title: Some(pb::CatalogTitle {
                    value: "New identifier".into(),
                }),
                subjects: vec![pb::SubjectAssociation {
                    subject: Some(new_identifier.clone()),
                    ..Default::default()
                }],
                status: pb::DecisionStatus::Proposed as i32,
                context: Some(pb::CatalogText {
                    value: "The replacement occurrence.".into(),
                }),
                rationale: Some(pb::CatalogText {
                    value: "The new connection ref names that path.".into(),
                }),
                options: vec![pb::DecisionOption {
                    title: Some(pb::CatalogTitle {
                        value: "Use the new id".into(),
                    }),
                    description: Some(pb::CatalogText {
                        value: "The omitted id was assigned by the server.".into(),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        service
            .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
                subject: Some(old_identifier),
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
                subject: Some(new_identifier),
                status: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .declarations
            .len(),
        1
    );

    let mut ambiguous = schema_entity(
        namespace,
        "ambiguous",
        1,
        pb::Schema {
            title: "Ambiguous".into(),
            fields: vec![
                object_field("customer", false, vec![string_field("id")]),
                string_field("sku"),
                string_field("sku"),
            ],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(body)) = &mut ambiguous.kind {
        body.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    let ambiguous = put(&service, ambiguous).await;
    let mut rejected_sku = ambiguous.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut rejected_sku.kind {
        body.title = "Changed".into();
        body.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("sku", false)])),
            role: pb::BusinessIdentifierRole::Alternate as i32,
            ..Default::default()
        });
    }
    expect_unchanged(
        &service,
        namespace,
        "ambiguous",
        1,
        rejected_sku,
        "property segment sku is missing or ambiguous",
    )
    .await;
    let kept_entity = get(&service, namespace, "ambiguous", 1).await;
    let kept = schema_of(&kept_entity);
    assert_eq!(kept.title, "Ambiguous");
    assert_eq!(kept.fields.len(), 3);
    assert_eq!(kept.identifiers.len(), 1);

    let traps = put(
        &service,
        schema_entity(
            namespace,
            "traps",
            1,
            pb::Schema {
                title: "Traps".into(),
                fields: vec![uuid_field("subject_token"), string_field("order_id")],
                ..Default::default()
            },
        ),
    )
    .await;
    let traps = schema_of(&traps);
    assert_eq!(traps.identifiers.len(), 0);
    assert_eq!(traps.property_relationships.len(), 0);
    assert_eq!(traps.aggregate_root, None);
    assert_eq!(traps.fields[0].metadata.len(), 3);
    let plain_entity = put(
        &service,
        schema_entity(
            namespace,
            "plain",
            1,
            pb::Schema {
                title: "Plain".into(),
                fields: vec![string_field("id")],
                ..Default::default()
            },
        ),
    )
    .await;
    let plain = schema_of(&plain_entity);
    assert_eq!(plain.identifiers.len(), 0);

    for (slug, role, fragment) in [
        (
            "role-zero",
            0,
            "identifier role must be primary or alternate",
        ),
        (
            "role-unknown",
            99,
            "identifier role must be primary or alternate",
        ),
    ] {
        expect_unpublished(
            &service,
            namespace,
            slug,
            order_with_role(namespace, slug, role),
            Code::InvalidArgument,
            fragment,
        )
        .await;
    }
    expect_unpublished(
        &service,
        namespace,
        "empty-id",
        order_with_connection(namespace, "empty-id", ""),
        Code::InvalidArgument,
        "connection ID must be a non-nil lowercase hyphenated UUID",
    )
    .await;
    expect_unpublished(
        &service,
        namespace,
        "bad-id",
        order_with_connection(namespace, "bad-id", "not-a-uuid"),
        Code::InvalidArgument,
        "connection ID must be a non-nil lowercase hyphenated UUID",
    )
    .await;
    let shared = "11111111-1111-7111-8111-111111111111";
    let mut shared_owner = order_with(
        namespace,
        vec![pb::BusinessIdentifier {
            connection_id: Some(pb::ConnectionId {
                value: shared.into(),
            }),
            property: Some(path(&[("customer", false), ("id", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        }],
        vec![relationship(namespace, 1, None, None)],
    );
    if let Some(pb::entity::Kind::Schema(body)) = &mut shared_owner.kind {
        body.id = Some(id(namespace, "shared-id", 1));
        body.property_relationships[0].connection_id = Some(pb::ConnectionId {
            value: shared.into(),
        });
    }
    expect_unpublished(
        &service,
        namespace,
        "shared-id",
        shared_owner,
        Code::InvalidArgument,
        "duplicate connection ID",
    )
    .await;

    for (kind, slug) in [
        (pb::EntityKind::Event, "placed"),
        (pb::EntityKind::Command, "place"),
        (pb::EntityKind::Swimlane, "orders"),
    ] {
        let owner_id = id(namespace, slug, 1);
        let owner = match kind {
            pb::EntityKind::Event => pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(owner_id.clone()),
                    title: slug.into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            pb::EntityKind::Command => pb::Entity {
                kind: Some(pb::entity::Kind::Command(pb::Command {
                    id: Some(owner_id.clone()),
                    title: slug.into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            pb::EntityKind::Swimlane => pb::Entity {
                kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                    id: Some(owner_id.clone()),
                    title: slug.into(),
                    stream_id: "orders-{order_id}".into(),
                    ..Default::default()
                })),
                ..Default::default()
            },
            _ => unreachable!(),
        };
        put(&service, owner).await;
        let mut bad = schema_entity(
            namespace,
            &format!("bad-{slug}"),
            1,
            pb::Schema {
                title: "Bad target".into(),
                fields: vec![string_field("product_ref")],
                ..Default::default()
            },
        );
        if let Some(pb::entity::Kind::Schema(body)) = &mut bad.kind {
            body.property_relationships
                .push(pb::BusinessPropertyRelationship {
                    source: Some(path(&[("product_ref", false)])),
                    target_entity: Some(pb::SchemaRef { id: Some(owner_id) }),
                    ..Default::default()
                });
        }
        expect_unpublished(
            &service,
            namespace,
            &format!("bad-{slug}"),
            bad,
            Code::NotFound,
            "entity not found",
        )
        .await;
    }

    let mut self_owner = schema_entity(
        namespace,
        "self-product",
        1,
        pb::Schema {
            title: "Self".into(),
            fields: vec![string_field("sku")],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(body)) = &mut self_owner.kind {
        body.property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("sku", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(namespace, "self-product", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                ..Default::default()
            });
    }
    put(&service, self_owner).await;
    let status = service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "self-product", 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
        .into_iter()
        .find(|reference| reference.from_field == "property_relationships[0].target_entity")
        .unwrap()
        .business_property_status;
    assert_eq!(
        status,
        Some(pb::BusinessPropertyReferenceStatus::PropertyResolved as i32)
    );
}

fn order_with_role(namespace: &str, slug: &str, role: i32) -> pb::Entity {
    let mut entity = order_with(
        namespace,
        vec![pb::BusinessIdentifier {
            property: Some(path(&[("customer", false), ("id", false)])),
            role,
            ..Default::default()
        }],
        Vec::new(),
    );
    if let Some(pb::entity::Kind::Schema(body)) = &mut entity.kind {
        body.id = Some(id(namespace, slug, 1));
    }
    entity
}

fn order_with_connection(namespace: &str, slug: &str, connection_id: &str) -> pb::Entity {
    let mut entity = order_with_role(namespace, slug, pb::BusinessIdentifierRole::Primary as i32);
    if let Some(pb::entity::Kind::Schema(body)) = &mut entity.kind {
        body.identifiers[0].connection_id = Some(pb::ConnectionId {
            value: connection_id.into(),
        });
    }
    entity
}

fn nested_field(depth: usize, leaf: &str) -> pb::FieldSpec {
    let mut field = string_field(leaf);
    for level in 1..=depth {
        field = object_field(&format!("n{level}"), false, vec![field]);
    }
    field
}

fn nested_path(depth: usize, leaf: &str) -> pb::PropertyPath {
    let mut segments = Vec::with_capacity(depth + 1);
    for level in (1..=depth).rev() {
        segments.push(segment(&format!("n{level}"), false));
    }
    segments.push(segment(leaf, false));
    pb::PropertyPath { segments }
}

async fn read_stored(
    service: &EventModelServiceImpl,
    kind: pb::EntityKind,
    entity_id: pb::Id,
) -> (pb::Entity, String) {
    let response = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: kind as i32,
            id: Some(entity_id),
        }))
        .await
        .unwrap()
        .into_inner();
    (response.entity.unwrap(), response.etag)
}

async fn outgoing_of(
    service: &EventModelServiceImpl,
    namespace: &str,
    slug: &str,
) -> Vec<pb::Reference> {
    service
        .get_outgoing_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, slug, 1)),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .references
}

fn bound_writer<T>(body: T) -> Request<T> {
    tenant_request(body, "business-tenant", Role::Writer, "acme")
}

fn tenant_request<T>(body: T, name: &str, role: Role, parent: &str) -> Request<T> {
    let mut request = Request::new(body);
    request.extensions_mut().insert(Principal {
        name: Arc::from(name),
        role,
        kind: PrincipalKind::User,
        is_anonymous: false,
        namespaces: NamespaceScope::unrestricted(),
        parent: Some(trogon_atlas_core::OwnerId::parse(parent).unwrap()),
    });
    request
}

fn relationship_statuses(references: &[pb::Reference]) -> Vec<i32> {
    references
        .iter()
        .filter(|reference| reference.from_field.contains("property_relationships["))
        .map(|reference| reference.business_property_status.unwrap())
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum AuthorizerFailure {
    Unavailable,
    TimedOut,
}

#[derive(Debug)]
struct FailingAuthorizer {
    kind: AuthorizerFailure,
}

impl FailingAuthorizer {
    fn status(&self) -> Status {
        match self.kind {
            AuthorizerFailure::Unavailable => Status::unavailable(
                "authorization service unavailable (CheckPermission): connection refused",
            ),
            AuthorizerFailure::TimedOut => {
                Status::deadline_exceeded("authorization service timed out")
            }
        }
    }
}

#[async_trait]
impl trogon_atlas_server::ownership::NamespaceAuthorizer for FailingAuthorizer {
    async fn admits(
        &self,
        visibility: &trogon_atlas_server::ownership::Visibility,
        _namespace: &trogon_atlas_core::NamespaceId,
    ) -> Result<bool, Status> {
        if visibility.is_unrestricted() {
            return Ok(true);
        }
        Err(self.status())
    }

    async fn lens(
        &self,
        visibility: &trogon_atlas_server::ownership::Visibility,
    ) -> Result<trogon_atlas_server::ownership::Lens, Status> {
        if visibility.is_unrestricted() {
            return Ok(trogon_atlas_server::ownership::Lens::Everything);
        }
        Err(self.status())
    }

    async fn may_write(
        &self,
        visibility: &trogon_atlas_server::ownership::Visibility,
        _namespace: &trogon_atlas_core::NamespaceId,
    ) -> Result<trogon_atlas_server::ownership::WriteVerdict, Status> {
        if visibility.is_unrestricted() {
            return Ok(trogon_atlas_server::ownership::WriteVerdict::Allowed);
        }
        Err(self.status())
    }

    async fn on_registered(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        Ok(())
    }

    async fn on_moved(
        &self,
        _record: &NamespaceRecord,
        _from: &trogon_atlas_core::OwnerId,
    ) -> Result<(), Status> {
        Ok(())
    }

    async fn on_released(&self, _record: &NamespaceRecord) -> Result<(), Status> {
        Ok(())
    }
}

struct BusinessReadFailure {
    inner: NatsStore,
    failure: fn() -> StoreError,
    armed: Arc<AtomicBool>,
    target: pb::Id,
}

impl BusinessReadFailure {
    async fn new(namespace: &str, failure: fn() -> StoreError) -> (Self, Arc<AtomicBool>) {
        let armed = Arc::new(AtomicBool::new(false));
        let store = Self {
            inner: trogon_atlas_testsupport::shared().await.store().await,
            failure,
            armed: Arc::clone(&armed),
            target: id(namespace, "product", 1),
        };
        (store, armed)
    }
}

#[async_trait]
impl Store for BusinessReadFailure {
    async fn register_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        created_by: &str,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner
            .register_namespace(name, parent, created_by)
            .await
    }

    async fn adopt_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        tenure: &trogon_atlas_store::NamespaceTenure,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner.adopt_namespace(name, parent, tenure).await
    }

    async fn restore_namespace(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner.restore_namespace(record).await
    }

    async fn release_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.release_namespace(id).await
    }

    async fn promote_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        self.inner.promote_namespace(id).await
    }

    async fn get_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        self.inner.get_namespace(id).await
    }

    async fn resolve_namespace(
        &self,
        parent: &trogon_atlas_core::OwnerId,
        name: &trogon_atlas_core::NamespaceName,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_core::NamespaceId>> {
        self.inner.resolve_namespace(parent, name).await
    }

    async fn list_namespaces(
        &self,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::NamespaceRecord>> {
        self.inner.list_namespaces().await
    }

    async fn move_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
        new_parent: &trogon_atlas_core::OwnerId,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceRecord> {
        self.inner.move_namespace(id, new_parent).await
    }

    async fn get(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<StoredEntity> {
        if id == &self.target && self.armed.load(Ordering::SeqCst) {
            return Err((self.failure)());
        }
        self.inner.get(kind, id, branch).await
    }

    async fn batch_get(
        &self,
        keys: &[(pb::EntityKind, pb::Id)],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<Option<StoredEntity>>> {
        self.inner.batch_get(keys, branch).await
    }

    async fn create(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner.create(kind, entity, ctx).await
    }

    async fn put(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner.put(kind, entity, force, ctx).await
    }

    async fn update(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        expected_etag: &str,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner
            .update(kind, entity, expected_etag, force, ctx)
            .await
    }

    async fn delete(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        expected_etag: Option<&str>,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete(kind, id, expected_etag, ctx).await
    }

    async fn referenced_targets(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        self.inner.referenced_targets(targets, branch).await
    }

    async fn referenced_targets_with_overlay(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
        overlay: &[(pb::EntityKey, Option<pb::Entity>)],
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        self.inner
            .referenced_targets_with_overlay(targets, branch, overlay)
            .await
    }

    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        self.inner.list(filter, limit, branch).await
    }

    async fn record_change(
        &self,
        kind: ChangeKind,
        entity_ref: &pb::EntityRef,
        changeset: Option<ChangesetRef<'_>>,
    ) -> trogon_atlas_store::StoreResult<u64> {
        self.inner.record_change(kind, entity_ref, changeset).await
    }

    async fn read_changes(
        &self,
        since: u64,
        limit: usize,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangeRecord>> {
        self.inner.read_changes(since, limit).await
    }

    async fn current_change_seq(&self) -> trogon_atlas_store::StoreResult<u64> {
        self.inner.current_change_seq().await
    }

    async fn batch_apply(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        self.inner.batch_apply(ops, ctx).await
    }

    fn subscribe(&self) -> broadcast::Receiver<ChangeRecord> {
        self.inner.subscribe()
    }

    async fn prune_changes(&self, before_seq: u64) -> trogon_atlas_store::StoreResult<()> {
        self.inner.prune_changes(before_seq).await
    }

    async fn change_stream_stats(
        &self,
    ) -> trogon_atlas_store::StoreResult<Option<ChangeStreamStats>> {
        self.inner.change_stream_stats().await
    }

    async fn create_branch(
        &self,
        name: &str,
        doc: &str,
    ) -> trogon_atlas_store::StoreResult<BranchInfo> {
        self.inner.create_branch(name, doc).await
    }

    async fn set_branch_fork_point(
        &self,
        name: &str,
        fork_changeset_id: &str,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner
            .set_branch_fork_point(name, fork_changeset_id)
            .await
    }

    async fn list_branches(&self) -> trogon_atlas_store::StoreResult<Vec<BranchInfo>> {
        self.inner.list_branches().await
    }

    async fn delete_branch(&self, name: &str) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete_branch(name).await
    }

    async fn list_branch_deltas(
        &self,
        branch: &str,
    ) -> trogon_atlas_store::StoreResult<Vec<BranchDeltaEntry>> {
        self.inner.list_branch_deltas(branch).await
    }

    async fn land_branch_merge(
        &self,
        branch: &str,
        ops: &[BranchLandOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        self.inner.land_branch_merge(branch, ops, ctx).await
    }

    async fn rebase_branch_deltas(
        &self,
        branch: &str,
        rebases: &[(pb::EntityKind, pb::Id, Option<(pb::Entity, String)>)],
    ) -> trogon_atlas_store::StoreResult<u32> {
        self.inner.rebase_branch_deltas(branch, rebases).await
    }

    async fn resolve_branch_entry(
        &self,
        branch: &str,
        kind: pb::EntityKind,
        id: &pb::Id,
        take_theirs: bool,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner
            .resolve_branch_entry(branch, kind, id, take_theirs)
            .await
    }

    async fn append_changeset(
        &self,
        record: &ChangesetRecord,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.append_changeset(record).await
    }

    async fn get_changeset(&self, id: &str) -> trogon_atlas_store::StoreResult<ChangesetRecord> {
        self.inner.get_changeset(id).await
    }

    async fn list_changesets(
        &self,
        page: ChangesetPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangesetRecord>> {
        self.inner.list_changesets(page).await
    }

    async fn list_entity_revisions(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        page: RevisionPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<EntityRevisionRecord>> {
        self.inner.list_entity_revisions(kind, id, page).await
    }

    async fn get_entity_revision(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        changeset_id: &str,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<EntityRevisionRecord> {
        self.inner
            .get_entity_revision(kind, id, changeset_id, branch)
            .await
    }
}

fn identifier(
    property: pb::PropertyPath,
    role: pb::BusinessIdentifierRole,
) -> pb::BusinessIdentifier {
    pb::BusinessIdentifier {
        property: Some(property),
        role: role as i32,
        ..Default::default()
    }
}

fn occurrence_ids(values: &[impl OccurrenceId]) -> Vec<String> {
    values.iter().map(OccurrenceId::occurrence_id).collect()
}

trait OccurrenceId {
    fn occurrence_id(&self) -> String;
}

impl OccurrenceId for pb::BusinessIdentifier {
    fn occurrence_id(&self) -> String {
        self.connection_id.as_ref().unwrap().value.clone()
    }
}

impl OccurrenceId for pb::BusinessPropertyRelationship {
    fn occurrence_id(&self) -> String {
        self.connection_id.as_ref().unwrap().value.clone()
    }
}

async fn rejected_put(
    service: &EventModelServiceImpl,
    proposed: pb::Entity,
    if_match: &str,
    code: Code,
    fragment: &str,
) {
    let kind = trogon_atlas_core::refs::entity_kind(&proposed).unwrap();
    let entity_id = trogon_atlas_core::refs::entity_id(&proposed)
        .unwrap()
        .clone();
    let (before, etag) = read_stored(service, kind, entity_id.clone()).await;
    let error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(proposed),
            if_match: if_match.into(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), code, "{error}");
    assert!(error.message().contains(fragment), "{error}");
    let (after, after_etag) = read_stored(service, kind, entity_id).await;
    assert_eq!(after.encode_to_vec(), before.encode_to_vec());
    assert_eq!(after_etag, etag);
}

fn fulfillment(namespace: &str, subject: pb::ResponsibilitySubject) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Team(pb::Team {
            id: Some(id(namespace, "fulfillment", 1)),
            title: Some(pb::DirectoryDisplayName {
                value: "Fulfillment".into(),
            }),
            responsibilities: vec![pb::Responsibility {
                kind: pb::ResponsibilityKind::Owner as i32,
                subject: Some(subject),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn record_key(namespace: &str, subject: pb::CatalogSubject) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Decision(pb::Decision {
            id: Some(id(namespace, "record-key", 1)),
            title: Some(pb::CatalogTitle {
                value: "Record key".into(),
            }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(subject),
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
    }
}

fn order_shell(namespace: &str) -> pb::Entity {
    schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![string_field("product_ref")],
            ..Default::default()
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_path_caps_resolve_and_refuse_the_next_byte() {
    let namespace = "bd-path-cap";
    let service = service().await;
    let name256 = "s".repeat(256);
    let name257 = "t".repeat(257);
    let mut product = product_schema(namespace, 1);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut product.kind {
        schema.fields = vec![
            nested_field(31, "leaf"),
            string_field(&name256),
            string_field(&name257),
        ];
    }
    let (product, _) = put_with_etag(&service, product).await;
    assert_eq!(schema_of(&product).fields[2].name.len(), 257);

    let mut deep = product.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut deep.kind {
        schema.identifiers.push(identifier(
            nested_path(31, "leaf"),
            pb::BusinessIdentifierRole::Primary,
        ));
    }
    let (deep, _) = put_with_etag(&service, deep).await;
    assert_eq!(
        schema_of(&deep).identifiers[0]
            .property
            .as_ref()
            .unwrap()
            .segments
            .len(),
        32
    );
    let deep_id = schema_of(&deep).identifiers[0].occurrence_id();

    let mut wide = deep.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut wide.kind {
        schema.identifiers.push(identifier(
            pb::PropertyPath {
                segments: vec![segment(&name256, false)],
            },
            pb::BusinessIdentifierRole::Alternate,
        ));
    }
    let (wide, wide_etag) = put_with_etag(&service, wide).await;
    assert_eq!(schema_of(&wide).identifiers[0].occurrence_id(), deep_id);
    assert_eq!(
        schema_of(&wide).identifiers[1]
            .property
            .as_ref()
            .unwrap()
            .segments[0]
            .name
            .len(),
        256
    );
    let wide_bytes = wide.encode_to_vec();

    let mut too_wide = wide.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut too_wide.kind {
        schema.identifiers.push(identifier(
            pb::PropertyPath {
                segments: vec![segment(&name257, false)],
            },
            pb::BusinessIdentifierRole::Alternate,
        ));
    }
    rejected_put(
        &service,
        too_wide,
        "",
        Code::InvalidArgument,
        "property segment name must be 1 to 256 UTF-8 bytes",
    )
    .await;

    let mut too_deep = wide.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut too_deep.kind {
        schema.identifiers.push(identifier(
            nested_path(32, "leaf"),
            pb::BusinessIdentifierRole::Alternate,
        ));
    }
    rejected_put(
        &service,
        too_deep,
        "",
        Code::InvalidArgument,
        "property path must contain 1 to 32 segments",
    )
    .await;
    let (stored_product, stored_etag) = read_stored(
        &service,
        pb::EntityKind::Schema,
        id(namespace, "product", 1),
    )
    .await;
    assert_eq!(stored_product.encode_to_vec(), wide_bytes);
    assert_eq!(stored_etag, wide_etag);
    assert_eq!(schema_of(&stored_product).fields[2].name.len(), 257);
    assert_eq!(schema_of(&stored_product).identifiers.len(), 2);

    let mut order = order_shell(namespace);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(nested_path(31, "leaf")),
            Some("hasMany"),
        ));
    }
    let (order, _) = put_with_etag(&service, order).await;
    let mut resolved = order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut resolved.kind {
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(pb::PropertyPath {
                segments: vec![segment(&name256, false)],
            }),
            Some("belongsTo"),
        ));
    }
    let (resolved, resolved_etag) = put_with_etag(&service, resolved).await;
    let resolved_bytes = resolved.encode_to_vec();
    let outgoing = outgoing_of(&service, namespace, "order").await;
    assert_eq!(
        relationship_statuses(&outgoing),
        vec![
            pb::BusinessPropertyReferenceStatus::PropertyResolved as i32,
            pb::BusinessPropertyReferenceStatus::PropertyResolved as i32,
        ]
    );
    assert_eq!(
        schema_of(&resolved).property_relationships[0]
            .target_property
            .as_ref()
            .unwrap()
            .segments
            .len(),
        32
    );

    let mut longer = resolved.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut longer.kind {
        schema.property_relationships[0].target_property = Some(nested_path(32, "leaf"));
    }
    rejected_put(
        &service,
        longer,
        "",
        Code::InvalidArgument,
        "target property path must contain 1 to 32 segments",
    )
    .await;

    let mut wider = resolved.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut wider.kind {
        schema.property_relationships.push(relationship(
            namespace,
            1,
            Some(pb::PropertyPath {
                segments: vec![segment(&name257, false)],
            }),
            None,
        ));
    }
    rejected_put(
        &service,
        wider,
        "",
        Code::InvalidArgument,
        "target property segment name must be 1 to 256 UTF-8 bytes",
    )
    .await;
    let (stored_order, order_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
    assert_eq!(stored_order.encode_to_vec(), resolved_bytes);
    assert_eq!(order_etag, resolved_etag);
    assert_eq!(outgoing_of(&service, namespace, "order").await, outgoing);
    let (product_after, product_etag) = read_stored(
        &service,
        pb::EntityKind::Schema,
        id(namespace, "product", 1),
    )
    .await;
    assert_eq!(product_after.encode_to_vec(), wide_bytes);
    assert_eq!(product_etag, wide_etag);
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_collection_reorder_keeps_occurrence_ids() {
    let namespace = "bd-reorder";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = order_with(
        namespace,
        vec![
            identifier(
                path(&[("customer", false), ("id", false)]),
                pb::BusinessIdentifierRole::Primary,
            ),
            identifier(
                path(&[("lines", true), ("sku", false)]),
                pb::BusinessIdentifierRole::Alternate,
            ),
        ],
        vec![
            relationship(namespace, 1, Some(path(&[("sku", false)])), Some("hasMany")),
            relationship(namespace, 1, Some(path(&[("sku", false)])), Some("hasMany")),
        ],
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.id = Some(id(namespace, "order", 1));
    }
    let stored = put(&service, order).await;
    let uid = stored.system.as_ref().unwrap().uid.clone();
    let schema = schema_of(&stored);
    let identifier_ids = occurrence_ids(&schema.identifiers);
    let relationship_ids = occurrence_ids(&schema.property_relationships);
    assert_ne!(identifier_ids[0], identifier_ids[1]);
    assert_ne!(relationship_ids[0], relationship_ids[1]);
    assert_eq!(
        schema.property_relationships[0].label,
        schema.property_relationships[1].label
    );

    let mut reordered = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut reordered.kind {
        schema.identifiers.reverse();
        schema.property_relationships.reverse();
    }
    let (after, _) = put_with_etag(&service, reordered).await;
    assert_eq!(after.system.as_ref().unwrap().uid, uid);
    assert_eq!(
        occurrence_ids(&schema_of(&after).identifiers),
        vec![identifier_ids[1].clone(), identifier_ids[0].clone()]
    );
    assert_eq!(
        occurrence_ids(&schema_of(&after).property_relationships),
        vec![relationship_ids[1].clone(), relationship_ids[0].clone()]
    );
    assert_eq!(
        schema_of(&after).identifiers[0].property,
        Some(path(&[("lines", true), ("sku", false)]))
    );
    assert_eq!(
        schema_of(&after).identifiers[1].property,
        Some(path(&[("customer", false), ("id", false)]))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_owner_recreate_scopes_reused_ids() {
    let namespace = "bd-owner-uid";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = order_with(
        namespace,
        vec![identifier(
            path(&[("customer", false), ("id", false)]),
            pb::BusinessIdentifierRole::Primary,
        )],
        vec![relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        )],
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.id = Some(id(namespace, "order", 1));
    }
    let order = put(&service, order).await;
    let old_uid = order.system.as_ref().unwrap().uid.clone();
    let identifier_id = schema_of(&order).identifiers[0].occurrence_id();
    let relationship_id = schema_of(&order).property_relationships[0].occurrence_id();
    let old_responsibility = connection_subject(&order, &relationship_id);
    let old_catalog = catalog_subject(&order, &identifier_id);
    let (team, team_etag) =
        put_with_etag(&service, fulfillment(namespace, old_responsibility.clone())).await;
    let team_bytes = team.encode_to_vec();
    let (decision, decision_etag) =
        put_with_etag(&service, record_key(namespace, old_catalog.clone())).await;
    let decision_bytes = decision.encode_to_vec();
    let assigned = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(old_responsibility.clone()),
            kind: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(assigned.assignments.len(), 1);
    assert_eq!(
        assigned.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    let decided = service
        .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
            subject: Some(old_catalog.clone()),
            status: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(decided.declarations.len(), 1);
    assert_eq!(
        decided.subject_state,
        pb::CatalogResolutionState::Resolved as i32
    );

    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, "order", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let recreated = put(&service, order.clone()).await;
    let new_uid = recreated.system.as_ref().unwrap().uid.clone();
    assert_ne!(new_uid, old_uid);
    assert_eq!(
        schema_of(&recreated).identifiers[0].occurrence_id(),
        identifier_id
    );
    assert_eq!(
        schema_of(&recreated).property_relationships[0].occurrence_id(),
        relationship_id
    );

    let stale_team = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(old_responsibility.clone()),
            kind: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(stale_team.assignments.len(), 1);
    assert_eq!(
        stale_team.subject_state,
        pb::ResponsibilityResolutionState::Unresolved as i32
    );
    let stale_decision = service
        .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
            subject: Some(old_catalog.clone()),
            status: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(stale_decision.declarations.len(), 1);
    assert_eq!(
        stale_decision.subject_state,
        pb::CatalogResolutionState::Unresolved as i32
    );
    let fresh_responsibility = connection_subject(&recreated, &relationship_id);
    let fresh_team = service
        .list_subject_assignments(Request::new(pb::ListSubjectAssignmentsRequest {
            subject: Some(fresh_responsibility),
            kind: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fresh_team.assignments.len(), 0);
    assert_eq!(
        fresh_team.subject_state,
        pb::ResponsibilityResolutionState::Resolved as i32
    );
    let fresh_catalog = catalog_subject(&recreated, &identifier_id);
    let fresh_decision = service
        .list_subject_decisions(Request::new(pb::ListSubjectDecisionsRequest {
            subject: Some(fresh_catalog),
            status: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fresh_decision.declarations.len(), 0);
    assert_eq!(
        fresh_decision.subject_state,
        pb::CatalogResolutionState::Resolved as i32
    );

    let mut renamed_team = team.clone();
    if let Some(pb::entity::Kind::Team(body)) = &mut renamed_team.kind {
        body.title = Some(pb::DirectoryDisplayName {
            value: "Renamed fulfillment".into(),
        });
    }
    rejected_put(
        &service,
        renamed_team,
        "",
        Code::InvalidArgument,
        "RESPONSIBILITY_REFERENCE_UNRESOLVED",
    )
    .await;
    let (team_after, team_etag_after) = read_stored(
        &service,
        pb::EntityKind::Team,
        id(namespace, "fulfillment", 1),
    )
    .await;
    assert_eq!(team_after.encode_to_vec(), team_bytes);
    assert_eq!(team_etag_after, team_etag);

    let mut renamed_decision = decision.clone();
    if let Some(pb::entity::Kind::Decision(body)) = &mut renamed_decision.kind {
        body.title = Some(pb::CatalogTitle {
            value: "Renamed key".into(),
        });
    }
    rejected_put(
        &service,
        renamed_decision,
        "",
        Code::InvalidArgument,
        "CATALOG_REFERENCE_UNRESOLVED",
    )
    .await;
    let (decision_after, decision_etag_after) = read_stored(
        &service,
        pb::EntityKind::Decision,
        id(namespace, "record-key", 1),
    )
    .await;
    assert_eq!(decision_after.encode_to_vec(), decision_bytes);
    assert_eq!(decision_etag_after, decision_etag);

    let mut late_team = team.clone();
    if let Some(pb::entity::Kind::Team(body)) = &mut late_team.kind {
        body.id = Some(id(namespace, "late-team", 1));
        body.title = Some(pb::DirectoryDisplayName {
            value: "Late".into(),
        });
    }
    let late_team_error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(late_team),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(
        late_team_error.code(),
        Code::InvalidArgument,
        "{late_team_error}"
    );
    assert!(
        late_team_error
            .message()
            .contains("RESPONSIBILITY_REFERENCE_UNRESOLVED"),
        "{late_team_error}"
    );
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Team as i32,
                id: Some(id(namespace, "late-team", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );

    let mut late_decision = decision.clone();
    if let Some(pb::entity::Kind::Decision(body)) = &mut late_decision.kind {
        body.id = Some(id(namespace, "late-decision", 1));
        body.title = Some(pb::CatalogTitle {
            value: "Late key".into(),
        });
    }
    let late_decision_error = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(late_decision),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(
        late_decision_error.code(),
        Code::InvalidArgument,
        "{late_decision_error}"
    );
    assert!(
        late_decision_error
            .message()
            .contains("CATALOG_REFERENCE_UNRESOLVED"),
        "{late_decision_error}"
    );
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Decision as i32,
                id: Some(id(namespace, "late-decision", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );

    let reference = outgoing_of(&service, namespace, "order")
        .await
        .into_iter()
        .find(|reference| reference.from_field.contains("property_relationships["))
        .unwrap();
    assert_eq!(reference.from_uid, new_uid);
    assert_eq!(
        reference.connection_id.as_ref().unwrap().value,
        relationship_id
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_stale_match_and_receipt_leave_the_index() {
    let namespace = "bd-stale";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = order_with(
        namespace,
        vec![identifier(
            path(&[("customer", false), ("id", false)]),
            pb::BusinessIdentifierRole::Primary,
        )],
        vec![relationship(
            namespace,
            1,
            Some(path(&[("sku", false)])),
            Some("hasMany"),
        )],
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.id = Some(id(namespace, "order", 1));
    }
    let (order, first_etag) = put_with_etag(&service, order).await;
    let uid = order.system.as_ref().unwrap().uid.clone();
    let identifier_id = schema_of(&order).identifiers[0].occurrence_id();
    let relationship_id = schema_of(&order).property_relationships[0].occurrence_id();

    let mut renamed = order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut renamed.kind {
        schema.title = "Renamed".into();
    }
    let renamed = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(renamed),
            if_match: first_etag.clone(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    let current = renamed.entity.unwrap();
    assert_ne!(renamed.etag, first_etag);
    assert_eq!(current.system.as_ref().unwrap().uid, uid);
    assert_eq!(
        schema_of(&current).identifiers[0].occurrence_id(),
        identifier_id
    );
    assert_eq!(
        schema_of(&current).property_relationships[0].occurrence_id(),
        relationship_id
    );
    let current_etag = renamed.etag;
    let outgoing = outgoing_of(&service, namespace, "order").await;

    let mut stale = current.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut stale.kind {
        schema.title = "Stale".into();
    }
    rejected_put(&service, stale, &first_etag, Code::Aborted, "etag mismatch").await;
    assert_eq!(outgoing_of(&service, namespace, "order").await, outgoing);
    let (unchanged, unchanged_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
    assert_eq!(unchanged.encode_to_vec(), current.encode_to_vec());
    assert_eq!(unchanged_etag, current_etag);

    let mut receipt_body = current.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut receipt_body.kind {
        schema.title = "Receipt A".into();
    }
    let first_receipt = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(receipt_body.clone()),
            if_match: current_etag.clone(),
            operation_id: "business-receipt-a".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!first_receipt.operation_receipt.as_ref().unwrap().replayed);
    let mut second_body = first_receipt.entity.unwrap();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut second_body.kind {
        schema.title = "Receipt B".into();
    }
    let second_receipt = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(second_body),
            if_match: first_receipt.etag.clone(),
            operation_id: "business-receipt-b".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!second_receipt.operation_receipt.as_ref().unwrap().replayed);
    assert_eq!(
        schema_of(second_receipt.entity.as_ref().unwrap()).title,
        "Receipt B"
    );
    let later_outgoing = outgoing_of(&service, namespace, "order").await;

    let replay = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(receipt_body),
            if_match: current_etag,
            operation_id: "business-receipt-a".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(replay.operation_receipt.as_ref().unwrap().replayed);
    assert_eq!(
        schema_of(replay.entity.as_ref().unwrap()).title,
        "Receipt B"
    );
    assert_eq!(replay.etag, second_receipt.etag);
    let (after_replay, replay_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
    assert_eq!(
        after_replay.encode_to_vec(),
        second_receipt.entity.as_ref().unwrap().encode_to_vec()
    );
    assert_eq!(replay_etag, second_receipt.etag);
    assert_eq!(
        outgoing_of(&service, namespace, "order").await,
        later_outgoing
    );

    let mut drifted = second_receipt.entity.clone().unwrap();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut drifted.kind {
        schema.title = "Drifted".into();
    }
    let reused = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(drifted),
            if_match: first_receipt.etag,
            operation_id: "business-receipt-a".into(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(reused.code(), Code::AlreadyExists, "{reused}");
    assert!(reused.message().contains("different digest"), "{reused}");
    let (final_entity, final_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
    assert_eq!(
        final_entity.encode_to_vec(),
        second_receipt.entity.unwrap().encode_to_vec()
    );
    assert_eq!(final_etag, second_receipt.etag);
    assert_eq!(schema_of(&final_entity).title, "Receipt B");
    assert_eq!(final_entity.system.as_ref().unwrap().uid, uid);
    assert_eq!(
        schema_of(&final_entity).identifiers[0].occurrence_id(),
        identifier_id
    );
    assert_eq!(
        schema_of(&final_entity).property_relationships[0].occurrence_id(),
        relationship_id
    );
    assert_eq!(
        outgoing_of(&service, namespace, "order").await,
        later_outgoing
    );
}

#[derive(Clone, Copy)]
enum InjectedRead {
    Unavailable,
    Backend,
    Corrupt,
}

fn injected(kind: InjectedRead) -> (fn() -> StoreError, Code, &'static str) {
    match kind {
        InjectedRead::Unavailable => (
            || StoreError::Unavailable("injected dependency outage".into()),
            Code::Unavailable,
            "injected dependency outage",
        ),
        InjectedRead::Backend => (
            || StoreError::Backend("injected dependency failure".into()),
            Code::Internal,
            "storage error",
        ),
        InjectedRead::Corrupt => (
            || StoreError::CorruptEntry {
                key: "product".into(),
                reason: "injected decode failure".into(),
            },
            Code::Internal,
            "storage error",
        ),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_store_and_decode_failures_publish_nothing() {
    for kind in [
        InjectedRead::Unavailable,
        InjectedRead::Backend,
        InjectedRead::Corrupt,
    ] {
        let (failure, code, message) = injected(kind);
        let namespace = "bd-store-down";
        let (backend, armed) = BusinessReadFailure::new(namespace, failure).await;
        let service =
            EventModelServiceImpl::try_new(Arc::new(backend), ServerMetrics::default()).unwrap();
        put(&service, product_schema(namespace, 1)).await;
        let mut order = order_shell(namespace);
        if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
            schema.property_relationships.push(relationship(
                namespace,
                1,
                Some(path(&[("sku", false)])),
                Some("hasMany"),
            ));
        }
        let (order, etag) = put_with_etag(&service, order).await;
        let bytes = order.encode_to_vec();
        let outgoing = outgoing_of(&service, namespace, "order").await;
        assert_eq!(
            relationship_statuses(&outgoing),
            vec![pb::BusinessPropertyReferenceStatus::PropertyResolved as i32]
        );

        armed.store(true, Ordering::SeqCst);
        let read_error = service
            .get_outgoing_references(Request::new(pb::GetReferencesRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(read_error.code(), code, "{read_error}");
        assert_eq!(read_error.message(), message, "{read_error}");
        if message == "storage error" {
            assert!(!read_error.message().contains("injected"), "{read_error}");
        }
        let (during, during_etag) =
            read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
        assert_eq!(during.encode_to_vec(), bytes);
        assert_eq!(during_etag, etag);

        let mut edited = during.clone();
        if let Some(pb::entity::Kind::Schema(schema)) = &mut edited.kind {
            schema.title = "Blocked".into();
        }
        let write_error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(edited),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(write_error.code(), code, "{write_error}");
        assert_eq!(write_error.message(), message, "{write_error}");

        let mut shadow = order_shell(namespace);
        if let Some(pb::entity::Kind::Schema(schema)) = &mut shadow.kind {
            schema.id = Some(id(namespace, "shadow", 1));
            schema.title = "Shadow".into();
            schema.property_relationships.push(relationship(
                namespace,
                1,
                Some(path(&[("sku", false)])),
                None,
            ));
        }
        let shadow_error = service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(shadow),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(shadow_error.code(), code, "{shadow_error}");
        assert_eq!(
            service
                .get_entity(Request::new(pb::GetEntityRequest {
                    kind: pb::EntityKind::Schema as i32,
                    id: Some(id(namespace, "shadow", 1)),
                }))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );

        armed.store(false, Ordering::SeqCst);
        let (after, after_etag) =
            read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
        assert_eq!(after.encode_to_vec(), bytes);
        assert_eq!(after_etag, etag);
        assert_eq!(outgoing_of(&service, namespace, "order").await, outgoing);
        assert_eq!(schema_of(&after).fields.len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_authorization_failures_publish_nothing() {
    for kind in [AuthorizerFailure::Unavailable, AuthorizerFailure::TimedOut] {
        let namespace = "bd-authz-down";
        let service = service().await;
        put(&service, product_schema(namespace, 1)).await;
        let mut order = order_shell(namespace);
        if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
            schema.property_relationships.push(relationship(
                namespace,
                1,
                Some(path(&[("sku", false)])),
                Some("hasMany"),
            ));
        }
        let (order, etag) = put_with_etag(&service, order).await;
        let bytes = order.encode_to_vec();
        let outgoing = outgoing_of(&service, namespace, "order").await;
        let authorizer = FailingAuthorizer { kind };
        let code = authorizer.status().code();
        let message = authorizer.status().message().to_owned();
        let service = service.with_authorizer(Arc::new(authorizer));

        let read_error = service
            .get_entity(bound_writer(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
            }))
            .await
            .unwrap_err();
        assert_eq!(read_error.code(), code, "{read_error}");
        assert_eq!(read_error.message(), message, "{read_error}");
        let reference_error = service
            .get_outgoing_references(bound_writer(pb::GetReferencesRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "order", 1)),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(reference_error.code(), code, "{reference_error}");
        assert_eq!(reference_error.message(), message, "{reference_error}");

        let mut edited = order.clone();
        if let Some(pb::entity::Kind::Schema(schema)) = &mut edited.kind {
            schema.title = "Blocked".into();
        }
        let write_error = service
            .put_entity(bound_writer(pb::PutEntityRequest {
                entity: Some(edited),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(write_error.code(), code, "{write_error}");
        assert_eq!(write_error.message(), message, "{write_error}");

        let mut shadow = order_shell(namespace);
        if let Some(pb::entity::Kind::Schema(schema)) = &mut shadow.kind {
            schema.id = Some(id(namespace, "shadow", 1));
            schema.title = "Shadow".into();
        }
        let shadow_error = service
            .put_entity(bound_writer(pb::PutEntityRequest {
                entity: Some(shadow),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(shadow_error.code(), code, "{shadow_error}");
        assert_eq!(
            service
                .get_entity(Request::new(pb::GetEntityRequest {
                    kind: pb::EntityKind::Schema as i32,
                    id: Some(id(namespace, "shadow", 1)),
                }))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound
        );

        let (after, after_etag) =
            read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
        assert_eq!(after.encode_to_vec(), bytes);
        assert_eq!(after_etag, etag);
        let reread = outgoing_of(&service, namespace, "order").await;
        assert_eq!(reread, outgoing);
        assert_eq!(
            relationship_statuses(&reread),
            vec![pb::BusinessPropertyReferenceStatus::PropertyResolved as i32]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn remainder_type_library_target_is_not_a_schema() {
    let namespace = "bd-kind";
    let service = service().await;
    let library_id = id(namespace, "orders", 1);
    put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
                id: Some(library_id.clone()),
                title: "orders".into(),
                files: vec![pb::ProtoSourceFile {
                    path: "orders.proto".into(),
                    content:
                        "syntax = \"proto3\"; package orders.v1; message Order { string id = 1; }"
                            .into(),
                }],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let library = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(library_id.clone()),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert!(matches!(
        library.kind,
        Some(pb::entity::Kind::TypeLibrary(_))
    ));
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(library_id.clone()),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );

    let mut bad = order_shell(namespace);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut bad.kind {
        schema.id = Some(id(namespace, "bad-library", 1));
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("product_ref", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(library_id.clone()),
                }),
                ..Default::default()
            });
    }
    expect_unpublished(
        &service,
        namespace,
        "bad-library",
        bad,
        Code::NotFound,
        "entity not found",
    )
    .await;
    let library_after = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::TypeLibrary as i32,
            id: Some(library_id),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(library_after.encode_to_vec(), library.encode_to_vec());
}

fn type_ref_version(schema: &pb::Schema) -> u64 {
    match &schema.fields[2].r#type.as_ref().unwrap().kind {
        Some(field_type::Kind::Ref(value)) => match &value.r#ref.as_ref().unwrap().r#ref {
            Some(pb::type_ref::Ref::Schema(schema_ref)) => schema_ref.id.as_ref().unwrap().version,
            other => panic!("schema ref, got {other:?}"),
        },
        other => panic!("ref, got {other:?}"),
    }
}

fn selector_neutral(reference: &pb::Reference) -> pb::Reference {
    let mut reference = reference.clone();
    reference.to = None;
    reference.connection_id = None;
    reference.from_field.clear();
    reference
}

#[derive(Clone, Copy)]
enum BusinessPayload {
    AggregateFalse,
    Identifiers,
    Relationships,
}

fn payload_name(payload: BusinessPayload) -> &'static str {
    match payload {
        BusinessPayload::AggregateFalse => "aggregate",
        BusinessPayload::Identifiers => "identifiers",
        BusinessPayload::Relationships => "relationships",
    }
}

fn business_payloads() -> [BusinessPayload; 3] {
    [
        BusinessPayload::AggregateFalse,
        BusinessPayload::Identifiers,
        BusinessPayload::Relationships,
    ]
}

fn schema_shell(namespace: &str, slug: &str) -> pb::Entity {
    schema_entity(
        namespace,
        slug,
        1,
        pb::Schema {
            title: "Owner".into(),
            fields: vec![
                object_field("customer", false, vec![string_field("id")]),
                schema_ref_field("product_ref", id(namespace, "product", 1)),
            ],
            ..Default::default()
        },
    )
}

fn with_business_payload(namespace: &str, slug: &str, payload: BusinessPayload) -> pb::Entity {
    let mut entity = schema_shell(namespace, slug);
    if let Some(pb::entity::Kind::Schema(schema)) = &mut entity.kind {
        match payload {
            BusinessPayload::AggregateFalse => schema.aggregate_root = Some(false),
            BusinessPayload::Identifiers => schema.identifiers.push(pb::BusinessIdentifier {
                property: Some(path(&[("customer", false), ("id", false)])),
                role: pb::BusinessIdentifierRole::Primary as i32,
                ..Default::default()
            }),
            BusinessPayload::Relationships => {
                schema.property_relationships.push(relationship(
                    namespace,
                    1,
                    Some(path(&[("sku", false)])),
                    None,
                ));
            }
        }
    }
    entity
}

fn clear_business(entity: &pb::Entity) -> pb::Entity {
    let mut entity = entity.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut entity.kind {
        schema.aggregate_root = None;
        schema.identifiers.clear();
        schema.property_relationships.clear();
    }
    entity
}

fn ownership_policies() -> [(&'static str, pb::SourceFieldOwnership); 4] {
    [
        (
            "none",
            pb::SourceFieldOwnership {
                policy: Some(pb::source_field_ownership::Policy::None(
                    pb::SourceNoOwnership {},
                )),
            },
        ),
        (
            "whole",
            pb::SourceFieldOwnership {
                policy: Some(pb::source_field_ownership::Policy::WholeOwner(
                    pb::SourceWholeOwnerOwnership {},
                )),
            },
        ),
        ("title", field_ownership(&[pb::SourceOwnedField::Title])),
        ("schema", field_ownership(&[pb::SourceOwnedField::Schema])),
    ]
}

fn entity_selection(namespace: &str, slug: &str) -> pb::SourceSelection {
    pb::SourceSelection {
        selection: Some(pb::source_selection::Selection::Entity(pb::EntityRef {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(namespace, slug, 1)),
        })),
    }
}

async fn source_mapping(
    service: &EventModelServiceImpl,
    registration_id: &str,
    mapping_id: &str,
) -> pb::SourceMapping {
    service
        .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
            registration_id: registration_id.into(),
            mapping_id: mapping_id.into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .mapping
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn closure_real_items_field_resolves_without_a_synthetic_hop() {
    let namespace = "bd-items-real";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![object_field("items", true, vec![string_field("sku")])],
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("items", true), ("sku", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("items", true), ("sku", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(namespace, "product", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                ..Default::default()
            });
    }
    let stored = put(&service, order).await;
    let schema = schema_of(&stored);
    let identifier = &schema.identifiers[0].property.as_ref().unwrap().segments;
    assert_eq!(identifier.len(), 2);
    assert_eq!(identifier[0].name, "items");
    assert!(identifier[0].repeated);
    assert_eq!(identifier[1].name, "sku");
    assert!(!identifier[1].repeated);
    let identifier_id = connection(&stored, false);
    let relationship_id = connection(&stored, true);
    assert_ne!(identifier_id, "");
    assert_ne!(relationship_id, identifier_id);
    let outgoing = outgoing_of(&service, namespace, "order").await;
    let hit = outgoing
        .iter()
        .find(|reference| reference.from_field.contains("property_relationships["))
        .unwrap();
    assert_eq!(
        hit.business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::PropertyResolved as i32)
    );
    assert_eq!(hit.connection_id.as_ref().unwrap().value, relationship_id);
    assert!(!outgoing
        .iter()
        .any(|reference| reference.from_field.contains("identifiers")));
    let (before, etag) =
        read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
    let mut hopped = before.clone();
    if let Some(pb::entity::Kind::Schema(body)) = &mut hopped.kind {
        body.identifiers[0].connection_id = None;
        body.identifiers[0].property =
            Some(path(&[("items", true), ("items", false), ("sku", false)]));
    }
    expect_unchanged(
        &service,
        namespace,
        "order",
        1,
        hopped,
        "property segment items is missing or ambiguous",
    )
    .await;
    let (after, after_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(namespace, "order", 1)).await;
    assert_eq!(after, before);
    assert_eq!(after_etag, etag);
    assert_eq!(connection(&after, false), identifier_id);
    assert_eq!(connection(&after, true), relationship_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn closure_type_ref_edit_leaves_the_relationship_and_its_id() {
    let namespace = "bd-typeref-only";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
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
    let uid = stored.system.as_ref().unwrap().uid.clone();
    assert_eq!(schema_of(&stored).fields[2].name, "product_ref");
    assert_eq!(type_ref_version(schema_of(&stored)), 1);
    let mut edited = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut edited.kind {
        schema.fields[2] = schema_ref_field("product_ref", id(namespace, "product", 2));
    }
    let updated = put(&service, edited).await;
    let schema = schema_of(&updated);
    assert_eq!(type_ref_version(schema), 2);
    assert_eq!(
        schema.property_relationships[0]
            .target_entity
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .version,
        1
    );
    assert_eq!(connection(&updated, false), identifier_id);
    assert_eq!(connection(&updated, true), relationship_id);
    assert_eq!(updated.system.as_ref().unwrap().uid, uid);
    let statuses = relationship_statuses(&outgoing_of(&service, namespace, "order").await);
    assert_eq!(
        statuses,
        vec![pb::BusinessPropertyReferenceStatus::PropertyResolved as i32]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn closure_target_reference_denial_is_not_source_write_denial() {
    let service = service().await;
    let source_ns = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-src-deny".into(),
            parent: "acme".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden_ns = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-hid-deny".into(),
            parent: "beta".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    put(&service, product_schema(&source_ns, 1)).await;
    let hidden = put(
        &service,
        schema_entity(
            &hidden_ns,
            "product",
            1,
            pb::Schema {
                title: "hidden-target-title-sentinel".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    let hidden_uid = hidden.system.as_ref().unwrap().uid.clone();
    let order = put(
        &service,
        schema_entity(
            &source_ns,
            "order",
            1,
            pb::Schema {
                title: "Order".into(),
                fields: vec![string_field("product_ref")],
                ..Default::default()
            },
        ),
    )
    .await;
    let mut titled = order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut titled.kind {
        schema.title = "Written".into();
        schema.aggregate_root = Some(true);
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("product_ref", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
    }
    let written = service
        .put_entity(tenant_request(
            pb::PutEntityRequest {
                entity: Some(titled),
                ..Default::default()
            },
            "acme-writer",
            Role::Writer,
            "acme",
        ))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(schema_of(&written).title, "Written");
    let mut linked = written.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut linked.kind {
        schema
            .property_relationships
            .push(relationship(&source_ns, 1, None, Some("hasMany")));
    }
    service
        .put_entity(tenant_request(
            pb::PutEntityRequest {
                entity: Some(linked),
                ..Default::default()
            },
            "acme-writer",
            Role::Writer,
            "acme",
        ))
        .await
        .unwrap();
    let (linked, linked_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(&source_ns, "order", 1)).await;
    let relationship_id = connection(&linked, true);
    assert_ne!(relationship_id, "");
    assert_eq!(
        linked.system.as_ref().unwrap().uid,
        written.system.as_ref().unwrap().uid
    );
    let mut hidden_target = linked.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut hidden_target.kind {
        schema.property_relationships[0].connection_id = None;
        schema.property_relationships[0].target_entity = Some(pb::SchemaRef {
            id: Some(id(&hidden_ns, "product", 1)),
        });
    }
    let denied = service
        .put_entity(tenant_request(
            pb::PutEntityRequest {
                entity: Some(hidden_target),
                ..Default::default()
            },
            "acme-writer",
            Role::Writer,
            "acme",
        ))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::NotFound);
    assert_eq!(denied.message(), "entity not found");
    let rendered = format!("{denied:?}");
    assert!(!rendered.contains("hidden-target-title-sentinel"));
    assert!(!rendered.contains(&hidden_uid));
    assert!(!rendered.contains("not available"));
    assert!(!rendered.contains("may not write"));
    let (kept, kept_etag) =
        read_stored(&service, pb::EntityKind::Schema, id(&source_ns, "order", 1)).await;
    assert_eq!(kept, linked);
    assert_eq!(kept_etag, linked_etag);
    assert_eq!(connection(&kept, true), relationship_id);
    let mut stolen = hidden.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut stolen.kind {
        schema.title = "Stolen".into();
    }
    let write_denied = service
        .put_entity(tenant_request(
            pb::PutEntityRequest {
                entity: Some(stolen),
                ..Default::default()
            },
            "acme-writer",
            Role::Writer,
            "acme",
        ))
        .await
        .unwrap_err();
    assert_eq!(write_denied.code(), Code::PermissionDenied);
    assert!(write_denied.message().contains("not available"));
    assert!(!format!("{write_denied:?}").contains("hidden-target-title-sentinel"));
    let hidden_after = get(&service, &hidden_ns, "product", 1).await;
    assert_eq!(
        schema_of(&hidden_after).title,
        "hidden-target-title-sentinel"
    );
    assert_eq!(hidden_after.system.as_ref().unwrap().uid, hidden_uid);
    let lane_id = id(&hidden_ns, "orders", 1);
    let lane = put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Swimlane(pb::Swimlane {
                id: Some(lane_id.clone()),
                title: "Orders".into(),
                stream_id: "orders".into(),
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let mut lane_edit = lane.clone();
    if let Some(pb::entity::Kind::Swimlane(body)) = lane_edit.kind.as_mut() {
        body.title = "Denied".into();
    }
    let refused = service
        .put_entity(tenant_request(
            pb::PutEntityRequest {
                entity: Some(lane_edit),
                ..Default::default()
            },
            "acme-writer",
            Role::Writer,
            "acme",
        ))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied);
    let retained = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Swimlane as i32,
            id: Some(lane_id),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(retained, lane);
}

#[tokio::test(flavor = "multi_thread")]
async fn closure_hidden_schema_get_and_checked_target_are_generic() {
    let service = service().await;
    let visible = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-vis-get".into(),
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
            name: "bd-hid-get".into(),
            parent: "beta".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let hidden_product = put(
        &service,
        schema_entity(
            &hidden,
            "product",
            1,
            pb::Schema {
                title: "hidden-schema-title-sentinel".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    let hidden_uid = hidden_product.system.as_ref().unwrap().uid.clone();
    put(
        &service,
        schema_entity(
            &visible,
            "missing-product",
            1,
            pb::Schema {
                title: "Ephemeral".into(),
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
                ..Default::default()
            });
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("product_ref", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(&visible, "missing-product", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                ..Default::default()
            });
    }
    put(&service, order).await;
    service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(&visible, "missing-product", 1)),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let hidden_get = service
        .get_entity(tenant_request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&hidden, "product", 1)),
            },
            "acme-reader",
            Role::Reader,
            "acme",
        ))
        .await
        .unwrap_err();
    let missing_hidden_get = service
        .get_entity(tenant_request(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&hidden, "missing-product", 1)),
            },
            "acme-reader",
            Role::Reader,
            "acme",
        ))
        .await
        .unwrap_err();
    assert_eq!(hidden_get.code(), Code::NotFound);
    assert_eq!(hidden_get.message(), "entity not found");
    assert_eq!(missing_hidden_get.code(), hidden_get.code());
    assert_eq!(missing_hidden_get.message(), hidden_get.message());
    let rendered = format!("{hidden_get:?}{missing_hidden_get:?}");
    assert!(!rendered.contains("hidden-schema-title-sentinel"));
    assert!(!rendered.contains(&hidden_uid));
    let listed = service
        .list_entities(tenant_request(
            pb::ListEntitiesRequest {
                kinds: vec![pb::EntityKind::Schema as i32],
                namespaces: vec![hidden.clone()],
                page_size: 100,
                ..Default::default()
            },
            "acme-reader",
            Role::Reader,
            "acme",
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.entities.len(), 0);
    let listed_rendered = format!("{listed:?}");
    assert!(!listed_rendered.contains("hidden-schema-title-sentinel"));
    assert!(!listed_rendered.contains(&hidden_uid));
    let page = service
        .get_outgoing_references(tenant_request(
            pb::GetReferencesRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(&visible, "order", 1)),
                ..Default::default()
            },
            "acme-reader",
            Role::Reader,
            "acme",
        ))
        .await
        .unwrap()
        .into_inner();
    let hidden_hit = page
        .references
        .iter()
        .find(|reference| {
            reference
                .to
                .as_ref()
                .and_then(|to| to.id.as_ref())
                .is_some_and(|id| id.namespace == hidden)
        })
        .unwrap();
    let missing_hit = page
        .references
        .iter()
        .find(|reference| {
            reference
                .to
                .as_ref()
                .and_then(|to| to.id.as_ref())
                .is_some_and(|id| id.slug == "missing-product")
        })
        .unwrap();
    assert_eq!(
        hidden_hit.business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::Unavailable as i32)
    );
    assert_eq!(
        missing_hit.business_property_status,
        hidden_hit.business_property_status
    );
    assert_ne!(
        hidden_hit.business_property_status,
        Some(pb::BusinessPropertyReferenceStatus::Unspecified as i32)
    );
    assert_eq!(selector_neutral(hidden_hit), selector_neutral(missing_hit));
    let page_rendered = format!("{page:?}");
    assert!(!page_rendered.contains("hidden-schema-title-sentinel"));
    assert!(!page_rendered.contains(&hidden_uid));
}

#[tokio::test(flavor = "multi_thread")]
async fn closure_source_policies_reject_create_and_whole_owner_matrix() {
    let service = service().await;
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "bd-src-matrix".into(),
            parent: "acme".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    put(&service, product_schema(&namespace, 1)).await;
    let mut keys = Vec::new();
    for (policy, _) in ownership_policies() {
        for payload in business_payloads() {
            keys.push(format!("c-{policy}-{}", payload_name(payload)));
        }
    }
    for payload in business_payloads() {
        let name = payload_name(payload);
        keys.push(format!("w-current-{name}"));
        keys.push(format!("w-in-{name}"));
        keys.push(format!("w-clear-{name}"));
    }
    let registration = service
        .create_source_registration(Request::new(pb::CreateSourceRegistrationRequest {
            registration: Some(pb::SourceRegistration {
                id: "bd-source-matrix".into(),
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
    let resources = keys
        .iter()
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
    for (policy_name, policy) in ownership_policies() {
        for payload in business_payloads() {
            let slug = format!("c-{policy_name}-{}", payload_name(payload));
            let created = service
                .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
                    registration_revision: registration.revision.clone(),
                    mapping: Some(pb::SourceMapping {
                        id: slug.clone(),
                        source: Some(source_key(&registration.id, &slug)),
                        selection: Some(entity_selection(&namespace, &slug)),
                        ownership: Some(policy.clone()),
                        ..Default::default()
                    }),
                }))
                .await
                .unwrap()
                .into_inner()
                .mapping
                .unwrap();
            assert!(created.base.is_none(), "{slug}");
            let incoming = with_business_payload(&namespace, &slug, payload);
            if matches!(payload, BusinessPayload::AggregateFalse) {
                assert_eq!(schema_of(&incoming).aggregate_root, Some(false));
            }
            let rejected = service
                .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
                    registration_id: registration.id.clone(),
                    registration_revision: registration.revision.clone(),
                    snapshot_id: snapshot.id.clone(),
                    run_id: format!("run-{slug}"),
                    native_payloads: vec![pb::SourceNativePayload {
                        source: Some(source_key(&registration.id, &slug)),
                        entity: Some(incoming),
                    }],
                    ..Default::default()
                }))
                .await
                .unwrap_err();
            assert_eq!(rejected.code(), Code::InvalidArgument, "{slug}: {rejected}");
            assert!(
                rejected
                    .message()
                    .contains("business declarations support native authoring only"),
                "{slug}: {rejected}"
            );
            assert_eq!(
                service
                    .get_entity(Request::new(pb::GetEntityRequest {
                        kind: pb::EntityKind::Schema as i32,
                        id: Some(id(&namespace, &slug, 1)),
                    }))
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound,
                "{slug}"
            );
            let mapping = source_mapping(&service, &registration.id, &slug).await;
            assert!(mapping.base.is_none(), "{slug}");
            assert_eq!(mapping.revision, created.revision, "{slug}");
            assert_eq!(mapping.last_accepted_run_id, "");
            assert_eq!(
                service
                    .get_source_run(Request::new(pb::GetSourceRunRequest {
                        registration_id: registration.id.clone(),
                        run_id: format!("run-{slug}"),
                    }))
                    .await
                    .unwrap_err()
                    .code(),
                Code::NotFound,
                "{slug}"
            );
        }
    }
    let whole = ownership_policies()
        .into_iter()
        .find(|(name, _)| *name == "whole")
        .unwrap()
        .1;
    for payload in business_payloads() {
        let name = payload_name(payload);
        let current_slug = format!("w-current-{name}");
        let (current, current_etag) = put_with_etag(
            &service,
            with_business_payload(&namespace, &current_slug, payload),
        )
        .await;
        let current_rejected = service
            .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: current_slug.clone(),
                    source: Some(source_key(&registration.id, &current_slug)),
                    selection: Some(entity_selection(&namespace, &current_slug)),
                    ownership: Some(whole.clone()),
                    adoption: Some(pb::SourceAdoption {
                        observed: Some(observed_schema(&current, &current_etag)),
                        authorized_adoption: true,
                    }),
                    ..Default::default()
                }),
            }))
            .await
            .unwrap_err();
        assert_eq!(
            current_rejected.code(),
            Code::InvalidArgument,
            "{current_slug}: {current_rejected}"
        );
        assert!(
            current_rejected
                .message()
                .contains("business declarations support native authoring only"),
            "{current_slug}: {current_rejected}"
        );
        assert_eq!(
            service
                .get_source_mapping(Request::new(pb::GetSourceMappingRequest {
                    registration_id: registration.id.clone(),
                    mapping_id: current_slug.clone(),
                }))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound,
            "{current_slug}"
        );
        let (kept, kept_etag) = read_stored(
            &service,
            pb::EntityKind::Schema,
            id(&namespace, &current_slug, 1),
        )
        .await;
        assert_eq!(kept, current, "{current_slug}");
        assert_eq!(kept_etag, current_etag, "{current_slug}");

        let incoming_slug = format!("w-in-{name}");
        let (clean, clean_etag) =
            put_with_etag(&service, schema_shell(&namespace, &incoming_slug)).await;
        assert_eq!(schema_of(&clean).aggregate_root, None);
        assert_eq!(schema_of(&clean).identifiers.len(), 0);
        assert_eq!(schema_of(&clean).property_relationships.len(), 0);
        let incoming_mapping = service
            .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: incoming_slug.clone(),
                    source: Some(source_key(&registration.id, &incoming_slug)),
                    selection: Some(entity_selection(&namespace, &incoming_slug)),
                    ownership: Some(whole.clone()),
                    adoption: Some(pb::SourceAdoption {
                        observed: Some(observed_schema(&clean, &clean_etag)),
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
        assert!(incoming_mapping.base.is_some(), "{incoming_slug}");
        let incoming_rejected = service
            .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                snapshot_id: snapshot.id.clone(),
                run_id: format!("run-{incoming_slug}"),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source_key(&registration.id, &incoming_slug)),
                    entity: Some(with_business_payload(&namespace, &incoming_slug, payload)),
                }],
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(
            incoming_rejected.code(),
            Code::InvalidArgument,
            "{incoming_slug}: {incoming_rejected}"
        );
        assert!(
            incoming_rejected
                .message()
                .contains("business declarations support native authoring only"),
            "{incoming_slug}: {incoming_rejected}"
        );
        let (incoming_kept, incoming_etag) = read_stored(
            &service,
            pb::EntityKind::Schema,
            id(&namespace, &incoming_slug, 1),
        )
        .await;
        assert_eq!(incoming_kept, clean, "{incoming_slug}");
        assert_eq!(incoming_etag, clean_etag, "{incoming_slug}");
        let incoming_after = source_mapping(&service, &registration.id, &incoming_slug).await;
        assert_eq!(
            incoming_after.base, incoming_mapping.base,
            "{incoming_slug}"
        );
        assert_eq!(
            incoming_after.revision, incoming_mapping.revision,
            "{incoming_slug}"
        );
        assert_eq!(incoming_after.last_accepted_run_id, "");
        assert_eq!(
            service
                .get_source_run(Request::new(pb::GetSourceRunRequest {
                    registration_id: registration.id.clone(),
                    run_id: format!("run-{incoming_slug}"),
                }))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound,
            "{incoming_slug}"
        );

        let clear_slug = format!("w-clear-{name}");
        let (clear_clean, clear_clean_etag) =
            put_with_etag(&service, schema_shell(&namespace, &clear_slug)).await;
        let clear_mapping = service
            .create_source_mapping(Request::new(pb::CreateSourceMappingRequest {
                registration_revision: registration.revision.clone(),
                mapping: Some(pb::SourceMapping {
                    id: clear_slug.clone(),
                    source: Some(source_key(&registration.id, &clear_slug)),
                    selection: Some(entity_selection(&namespace, &clear_slug)),
                    ownership: Some(whole.clone()),
                    adoption: Some(pb::SourceAdoption {
                        observed: Some(observed_schema(&clear_clean, &clear_clean_etag)),
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
        let (declared, declared_etag) = put_with_etag(
            &service,
            with_business_payload(&namespace, &clear_slug, payload),
        )
        .await;
        assert_ne!(declared_etag, clear_clean_etag, "{clear_slug}");
        let clear_rejected = service
            .preview_source_run(Request::new(pb::PreviewSourceRunRequest {
                registration_id: registration.id.clone(),
                registration_revision: registration.revision.clone(),
                snapshot_id: snapshot.id.clone(),
                run_id: format!("run-{clear_slug}"),
                native_payloads: vec![pb::SourceNativePayload {
                    source: Some(source_key(&registration.id, &clear_slug)),
                    entity: Some(clear_business(&declared)),
                }],
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(
            clear_rejected.code(),
            Code::InvalidArgument,
            "{clear_slug}: {clear_rejected}"
        );
        assert!(
            clear_rejected
                .message()
                .contains("business declarations support native authoring only"),
            "{clear_slug}: {clear_rejected}"
        );
        let (clear_kept, clear_kept_etag) = read_stored(
            &service,
            pb::EntityKind::Schema,
            id(&namespace, &clear_slug, 1),
        )
        .await;
        assert_eq!(clear_kept, declared, "{clear_slug}");
        assert_eq!(clear_kept_etag, declared_etag, "{clear_slug}");
        let clear_after = source_mapping(&service, &registration.id, &clear_slug).await;
        assert_eq!(clear_after.base, clear_mapping.base, "{clear_slug}");
        assert_eq!(clear_after.revision, clear_mapping.revision, "{clear_slug}");
        assert_eq!(clear_after.last_accepted_run_id, "");
        assert_eq!(
            service
                .get_source_run(Request::new(pb::GetSourceRunRequest {
                    registration_id: registration.id.clone(),
                    run_id: format!("run-{clear_slug}"),
                }))
                .await
                .unwrap_err()
                .code(),
            Code::NotFound,
            "{clear_slug}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn closure_missing_legacy_ref_and_supersedes_stay_admitted() {
    let namespace = "bd-legacy-ref";
    let service = service().await;
    put(&service, product_schema(namespace, 1)).await;
    let mut order = schema_entity(
        namespace,
        "order",
        1,
        pb::Schema {
            title: "Order".into(),
            fields: vec![
                string_field("sku"),
                schema_ref_field("legacy_ref", id(namespace, "missing-type", 1)),
            ],
            supersedes: Some(id(namespace, "missing-prior", 1)),
            ..Default::default()
        },
    );
    if let Some(pb::entity::Kind::Schema(schema)) = &mut order.kind {
        schema.identifiers.push(pb::BusinessIdentifier {
            property: Some(path(&[("sku", false)])),
            role: pb::BusinessIdentifierRole::Primary as i32,
            ..Default::default()
        });
        schema
            .property_relationships
            .push(pb::BusinessPropertyRelationship {
                source: Some(path(&[("sku", false)])),
                target_entity: Some(pb::SchemaRef {
                    id: Some(id(namespace, "product", 1)),
                }),
                target_property: Some(path(&[("sku", false)])),
                ..Default::default()
            });
    }
    let stored = put(&service, order).await;
    let schema = schema_of(&stored);
    assert_eq!(schema.supersedes.as_ref().unwrap().slug, "missing-prior");
    match &schema.fields[1].r#type.as_ref().unwrap().kind {
        Some(field_type::Kind::Ref(value)) => match &value.r#ref.as_ref().unwrap().r#ref {
            Some(pb::type_ref::Ref::Schema(schema_ref)) => {
                assert_eq!(schema_ref.id.as_ref().unwrap().slug, "missing-type");
            }
            other => panic!("schema ref, got {other:?}"),
        },
        other => panic!("ref, got {other:?}"),
    }
    let identifier_id = connection(&stored, false);
    let relationship_id = connection(&stored, true);
    assert_ne!(identifier_id, "");
    assert_ne!(relationship_id, "");
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "missing-type", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id(namespace, "missing-prior", 1)),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let mut renamed = stored.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut renamed.kind {
        schema.title = "Renamed".into();
    }
    let renamed = put(&service, renamed).await;
    let schema = schema_of(&renamed);
    assert_eq!(schema.title, "Renamed");
    assert_eq!(schema.supersedes.as_ref().unwrap().slug, "missing-prior");
    assert_eq!(connection(&renamed, false), identifier_id);
    assert_eq!(connection(&renamed, true), relationship_id);
    let statuses = relationship_statuses(&outgoing_of(&service, namespace, "order").await);
    assert_eq!(
        statuses,
        vec![pb::BusinessPropertyReferenceStatus::PropertyResolved as i32]
    );
}
