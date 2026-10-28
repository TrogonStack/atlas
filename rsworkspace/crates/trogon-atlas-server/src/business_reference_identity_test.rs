//! Occurrence identity for business reference status.
//!
//! `GetIncomingReferences` stamps `from_field` and `connection_id` from one
//! owner body, then reloads that owner to check the path. A committed
//! reorder is coherent, so the public RPC cannot freeze the reload on a
//! different body. This calls the same decorator with the disagreement that
//! reload would observe: index 0 is the sibling, and `connection_id` still
//! names the original occurrence.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{collections::HashMap, sync::Arc};

use tonic::Request;
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelService};

use super::EventModelServiceImpl;
use crate::metrics::ServerMetrics;

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "bd-occurrence".into(),
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
            kind: Some(pb::field_type::Kind::String(pb::field_type::StringType {})),
        }),
        ..Default::default()
    }
}

fn schema_of(entity: &pb::Entity) -> &pb::Schema {
    match &entity.kind {
        Some(pb::entity::Kind::Schema(schema)) => schema,
        other => panic!("schema, got {other:?}"),
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

fn reference(connection_id: Option<pb::ConnectionId>) -> pb::Reference {
    pb::Reference {
        from: Some(pb::EntityRef {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id("order")),
        }),
        from_field: "property_relationships[0].target_entity".into(),
        to: Some(pb::EntityRef {
            kind: pb::EntityKind::Schema as i32,
            id: Some(id("product")),
        }),
        connection_id,
        ..Default::default()
    }
}

async fn status_for(
    service: &EventModelServiceImpl,
    connection_id: Option<pb::ConnectionId>,
    owners: Option<&HashMap<pb::EntityKey, pb::Entity>>,
) -> Option<i32> {
    let vis = crate::ownership::Visibility::unrestricted(Arc::from("bd-occurrence"));
    let mut references = [reference(connection_id)];
    service
        .decorate_business_property_status(&mut references, None, None, &vis, owners)
        .await
        .unwrap();
    references[0].business_property_status
}

#[tokio::test(flavor = "multi_thread")]
async fn review_red_reordered_reference_status_follows_the_occurrence_id() {
    let service = service().await;
    put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Schema(pb::Schema {
                id: Some(id("product")),
                title: "Product".into(),
                fields: vec![string_field("sku")],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let order = put(
        &service,
        pb::Entity {
            kind: Some(pb::entity::Kind::Schema(pb::Schema {
                id: Some(id("order")),
                title: "Order".into(),
                fields: vec![string_field("product_ref")],
                property_relationships: vec![
                    pb::BusinessPropertyRelationship {
                        source: Some(path("product_ref")),
                        target_entity: Some(pb::SchemaRef {
                            id: Some(id("product")),
                        }),
                        target_property: Some(path("sku")),
                        label: Some(pb::CatalogTitle {
                            value: "resolved".into(),
                        }),
                        ..Default::default()
                    },
                    pb::BusinessPropertyRelationship {
                        source: Some(path("product_ref")),
                        target_entity: Some(pb::SchemaRef {
                            id: Some(id("product")),
                        }),
                        label: Some(pb::CatalogTitle {
                            value: "entity-only".into(),
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            })),
            ..Default::default()
        },
    )
    .await;
    let entity_only = schema_of(&order).property_relationships[1]
        .connection_id
        .clone()
        .unwrap();
    assert_ne!(entity_only.value, "");
    assert_eq!(
        status_for(&service, Some(entity_only.clone()), None).await,
        Some(pb::BusinessPropertyReferenceStatus::EntityOnly as i32),
        "status followed property_relationships[0] instead of connection_id"
    );
    assert_eq!(
        status_for(&service, None, None).await,
        Some(pb::BusinessPropertyReferenceStatus::Unavailable as i32)
    );
    assert_eq!(
        status_for(
            &service,
            Some(pb::ConnectionId {
                value: "11111111-1111-4111-8111-111111111111".into(),
            }),
            None,
        )
        .await,
        Some(pb::BusinessPropertyReferenceStatus::Unavailable as i32)
    );
    let mut partial = order;
    if let Some(pb::entity::Kind::Schema(schema)) = &mut partial.kind {
        schema.property_relationships.retain(|relationship| {
            relationship
                .label
                .as_ref()
                .is_some_and(|title| title.value == "resolved")
        });
    }
    let owners = HashMap::from([(
        pb::EntityKey::new(pb::EntityKind::Schema, &id("order")),
        partial,
    )]);
    assert_eq!(
        status_for(&service, Some(entity_only), Some(&owners)).await,
        Some(pb::BusinessPropertyReferenceStatus::Unavailable as i32),
        "status used a later store body instead of the coherent snapshot"
    );
}
