#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use prost::Message;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{key::entity_key, NatsStore, NatsStoreConfig, Store, StoreMetrics};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "malformed-native".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn component(slug: &str, metadata: Vec<prost_types::Any>) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(slug)),
            title: slug.into(),
            metadata,
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn target() -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Component as i32,
        id: Some(id("target")),
    }
}

fn assert_generic(status: &tonic::Status) {
    assert!(
        matches!(status.code(), Code::DataLoss | Code::Internal),
        "{status}"
    );
    assert!(!status.message().contains("corrupt-owner"), "{status}");
    assert!(!status.message().contains("malformed-native"), "{status}");
}

#[tokio::test]
async fn malformed_persisted_group_metadata_fails_every_reference_consumer_without_panicking() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = format!(
        "malformed-{}",
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
    let service = EventModelServiceImpl::try_new(store.clone(), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0);
    for slug in ["target", "replacement"] {
        service
            .put_entity(Request::new(pb::PutEntityRequest {
                entity: Some(component(slug, Vec::new())),
                ..Default::default()
            }))
            .await
            .unwrap();
    }
    let corrupt = component(
        "corrupt-owner",
        vec![prost_types::Any {
            type_url:
                "type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation"
                    .into(),
            value: vec![0xff],
        }],
    );
    let client = async_nats::connect(&nats.url).await.unwrap();
    let kv = async_nats::jetstream::new(client)
        .get_key_value(&bucket)
        .await
        .unwrap();
    kv.put(
        entity_key(pb::EntityKind::Component, &id("corrupt-owner")),
        corrupt.encode_to_vec().into(),
    )
    .await
    .unwrap();

    let incoming = service
        .get_incoming_references(Request::new(pb::GetReferencesRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id("target")),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_generic(&incoming);

    let impact = service
        .get_impact(Request::new(pb::GetImpactRequest {
            root: Some(target()),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_generic(&impact);

    for mode in [
        pb::delete_entity_request::Mode::FailIfReferenced,
        pb::delete_entity_request::Mode::DryRun,
    ] {
        let deletion = service
            .delete_entity(Request::new(pb::DeleteEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(id("target")),
                mode: mode as i32,
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_generic(&deletion);
    }
    assert!(service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Component as i32,
            id: Some(id("target")),
        }))
        .await
        .is_ok());

    for dry_run in [true, false] {
        let retarget = service
            .retarget_references(Request::new(pb::RetargetReferencesRequest {
                from: Some(target()),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Component as i32,
                    id: Some(id("replacement")),
                }),
                dry_run,
                ..Default::default()
            }))
            .await;
        match retarget {
            Err(status) => assert_generic(&status),
            Ok(reply) => {
                let reply = reply.into_inner();
                assert!(reply.rewritten == 0 && reply.failed > 0, "{reply:?}");
            }
        }
    }

    let guard = store
        .referenced_targets(
            &[pb::EntityKey::new(pb::EntityKind::Component, &id("target"))],
            None,
        )
        .await;
    assert!(
        guard.is_err(),
        "store guard treated malformed state as empty"
    );
}
