#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use prost::Message;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{key::entity_key, NatsStore, NatsStoreConfig, StoreMetrics};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "catalog-fault".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn document(slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Document(pb::Document {
            id: Some(id(slug)),
            title: Some(pb::CatalogTitle { value: slug.into() }),
            subjects: vec![pb::SubjectAssociation {
                subject: Some(pb::CatalogSubject {
                    selection: Some(pb::catalog_subject::Selection::Entity(pb::EntityRef {
                        kind: pb::EntityKind::ReadModel as i32,
                        id: Some(id("receipt")),
                    })),
                }),
                ..Default::default()
            }],
            body: Some(pb::document::Body::Link(pb::ExternalDocumentLink {
                value: "https://example.com/document".into(),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

#[tokio::test]
async fn unreadable_catalog_owner_cannot_be_reported_as_complete_empty_discovery() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = format!(
        "catalog-fault-{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    );
    let mut config = NatsStoreConfig::new(&nats.url, StoreMetrics::default());
    config.bucket = format!("test-{suffix}");
    config.branches_bucket = format!("test-branches-{suffix}");
    config.changesets_bucket = format!("test-changesets-{suffix}");
    config.revisions_bucket = format!("test-revisions-{suffix}");
    config.batches_bucket = format!("test-batches-{suffix}");
    config.namespaces_bucket = format!("test-namespaces-{suffix}");
    config.lease_bucket = format!("test-writer-lease-{suffix}");
    config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.{suffix}");
    let bucket = config.bucket.clone();
    let store = Arc::new(NatsStore::connect_with(config).await.unwrap());
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
                    id: Some(id("receipt")),
                    title: "Receipt".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    let target = pb::DocumentSelection {
        selection: Some(pb::document_selection::Selection::Document(
            pb::DocumentRef {
                id: Some(id("hub")),
            },
        )),
    };
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(document("hub")),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut owner = document("private-corrupt-owner");
    let Some(pb::entity::Kind::Document(body)) = owner.kind.as_mut() else {
        panic!("expected document");
    };
    body.related_content.push(pb::RelatedContent {
        role: pb::RelatedContentRole::Example as i32,
        document: Some(target.clone()),
        ..Default::default()
    });
    let stored = service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(owner),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    let query = || {
        Request::new(pb::ListIncomingDocumentDeclarationsRequest {
            document: Some(target.clone()),
            page_size: Some(1),
            ..Default::default()
        })
    };
    let healthy = service
        .list_incoming_document_declarations(query())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(healthy.declarations.len(), 1);
    assert!(!healthy.incomplete);
    assert_eq!(healthy.failures, []);
    let connection = match healthy.declarations[0].declaration.as_ref().unwrap() {
        pb::catalog_declaration::Declaration::RelatedContent(value) => value.connection_id.clone(),
        other => panic!("expected related content, got {other:?}"),
    };
    assert!(connection.is_some());

    let client = async_nats::connect(&nats.url).await.unwrap();
    let kv = async_nats::jetstream::new(client)
        .get_key_value(&bucket)
        .await
        .unwrap();
    let key = entity_key(pb::EntityKind::Document, &id("private-corrupt-owner"));
    kv.put(key.clone(), vec![0xff].into()).await.unwrap();
    let fresh = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default()).unwrap();
    let failure = fresh
        .list_incoming_document_declarations(query())
        .await
        .unwrap_err();
    assert!(
        matches!(failure.code(), Code::Internal | Code::DataLoss),
        "{failure}"
    );
    assert!(!failure.message().contains("private-corrupt-owner"));
    assert!(!failure.message().contains("catalog-fault"));
    assert!(!failure.message().contains(&bucket));

    kv.put(key, stored.encode_to_vec().into()).await.unwrap();
    let recovered = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let repaired = recovered
        .list_incoming_document_declarations(query())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(repaired.declarations.len(), 1);
    let prior = &healthy.declarations[0];
    let current = &repaired.declarations[0];
    assert_eq!(current.declaration, prior.declaration);
    assert_eq!(current.field, prior.field);
    assert_eq!(current.state, prior.state);
    let prior_owner = prior.owner.as_ref().unwrap();
    let current_owner = current.owner.as_ref().unwrap();
    assert_eq!(current_owner.owner, prior_owner.owner);
    assert_eq!(current_owner.uid, prior_owner.uid);
    assert_ne!(current_owner.etag, prior_owner.etag);
    assert!(!repaired.incomplete);
    assert_eq!(repaired.failures, []);
}
