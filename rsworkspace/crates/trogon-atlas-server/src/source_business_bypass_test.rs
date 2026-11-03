//! Causal proof that `SourceBusinessBypass` must not skip target admission
//! for a declaration the stored schema does not already have.
//!
//! Public source preview drops newly authored business fields before
//! `batch_mutate`, so this crate-visible extension is the caller that can
//! still submit them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tonic::{Code, Request};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService, field_type};

use super::{source_integration::SourceBusinessBypass, EventModelServiceImpl};
use crate::metrics::ServerMetrics;

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "bd-bypass-proof".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn path(name: &str) -> pb::PropertyPath {
    pb::PropertyPath {
        segments: vec![pb::PropertySegment {
            name: name.into(),
            repeated: false,
        }],
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

fn schema_entity(slug: &str, schema: pb::Schema) -> pb::Entity {
    let mut schema = schema;
    schema.id = Some(id(slug));
    pb::Entity {
        kind: Some(pb::entity::Kind::Schema(schema)),
        ..Default::default()
    }
}

fn schema_of(entity: &pb::Entity) -> &pb::Schema {
    match &entity.kind {
        Some(pb::entity::Kind::Schema(schema)) => schema,
        other => panic!("schema, got {other:?}"),
    }
}

fn relationship(target: &str) -> pb::BusinessPropertyRelationship {
    pb::BusinessPropertyRelationship {
        source: Some(path("product_ref")),
        target_entity: Some(pb::SchemaRef {
            id: Some(id(target)),
        }),
        target_property: Some(path("sku")),
        label: Some(pb::CatalogTitle {
            value: "hasMany".into(),
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

async fn get(service: &EventModelServiceImpl, slug: &str) -> pb::Entity {
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id(slug)),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn bypass_batch(
    service: &EventModelServiceImpl,
    entity: pb::Entity,
    validate_only: bool,
) -> Result<pb::BatchMutateResponse, tonic::Status> {
    let mut request = Request::new(pb::BatchMutateRequest {
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            })),
        }],
        validate_only,
        ..Default::default()
    });
    request.extensions_mut().insert(SourceBusinessBypass);
    service
        .batch_mutate(request)
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn source_bypass_admits_only_preserved_declarations() {
    let service = service().await;
    put(
        &service,
        schema_entity(
            "product",
            pb::Schema {
                title: "Product".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            },
        ),
    )
    .await;
    let order = put(
        &service,
        schema_entity(
            "order",
            pb::Schema {
                title: "Order".into(),
                fields: vec![string_field("product_ref")],
                property_relationships: vec![relationship("product")],
                ..Default::default()
            },
        ),
    )
    .await;
    let mut product = get(&service, "product").await;
    if let Some(pb::entity::Kind::Schema(schema)) = &mut product.kind {
        schema.fields.clear();
    }
    put(&service, product).await;

    let mut renamed = order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut renamed.kind {
        schema.title = "Renamed".into();
    }
    let preserved = bypass_batch(&service, renamed, false).await.unwrap();
    assert_eq!(
        preserved.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let preserved_order = get(&service, "order").await;
    assert_eq!(schema_of(&preserved_order).title, "Renamed");
    assert_eq!(
        schema_of(&preserved_order).property_relationships,
        schema_of(&order).property_relationships
    );

    let mut broken_id = preserved_order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut broken_id.kind {
        schema.property_relationships[0].connection_id = Some(pb::ConnectionId {
            value: String::new(),
        });
    }
    let shape = bypass_batch(&service, broken_id, false).await.unwrap_err();
    assert_eq!(shape.code(), Code::InvalidArgument);
    assert_eq!(
        schema_of(&get(&service, "order").await).property_relationships,
        schema_of(&order).property_relationships
    );

    let mut broken_path = preserved_order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut broken_path.kind {
        schema.fields.clear();
    }
    let local_path = bypass_batch(&service, broken_path, false)
        .await
        .unwrap_err();
    assert_eq!(local_path.code(), Code::InvalidArgument);
    assert!(
        local_path.message().contains("property segment"),
        "{local_path:?}"
    );
    assert_eq!(schema_of(&get(&service, "order").await).fields.len(), 1);

    let mut changed = preserved_order.clone();
    if let Some(pb::entity::Kind::Schema(schema)) = &mut changed.kind {
        schema.aggregate_root = Some(true);
    }
    let authored = bypass_batch(&service, changed, false).await.unwrap_err();
    assert_eq!(authored.code(), Code::InvalidArgument, "{authored:?}");
    let unchanged = get(&service, "order").await;
    assert_eq!(schema_of(&unchanged).aggregate_root, None);
    assert_eq!(schema_of(&unchanged).title, "Renamed");

    let created = schema_entity(
        "fresh",
        pb::Schema {
            title: "Fresh".into(),
            fields: vec![string_field("product_ref")],
            property_relationships: vec![relationship("absent")],
            ..Default::default()
        },
    );
    let create = bypass_batch(&service, created.clone(), false)
        .await
        .unwrap_err();
    assert_eq!(create.code(), Code::NotFound, "{create:?}");
    assert_eq!(
        service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Schema as i32,
                id: Some(id("fresh")),
            }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    let dry_run = bypass_batch(&service, created, true).await.unwrap_err();
    assert_eq!(dry_run.code(), Code::NotFound, "{dry_run:?}");
}
