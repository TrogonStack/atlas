#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use prost::Message;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap()
}

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn orphan_event(namespace: &str, slug: &str, doc: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            metadata: vec![prost_types::Any {
                type_url: "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.OrphanAnnotation"
                    .into(),
                value: pb::OrphanAnnotation { doc: doc.into() }.encode_to_vec(),
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn event_model(namespace: &str, slug: &str, event: pb::Id) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
            id: Some(id(namespace, slug)),
            title: format!("{namespace}/{slug}"),
            members: vec![pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(event),
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn put_op(entity: pb::Entity) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        })),
    }
}

async fn put(service: &EventModelServiceImpl, entity: pb::Entity) {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(entity),
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn batch_put(service: &EventModelServiceImpl, entities: Vec<pb::Entity>) {
    service
        .batch_mutate(Request::new(pb::BatchMutateRequest {
            ops: entities.into_iter().map(put_op).collect(),
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn orphan_codes(service: &EventModelServiceImpl, model: pb::Id) -> Vec<String> {
    service
        .validate_event_model(Request::new(pb::ValidateEventModelRequest {
            event_model: None,
            event_model_id: Some(model),
        }))
        .await
        .unwrap()
        .into_inner()
        .issues
        .into_iter()
        .map(|i| i.code)
        .filter(|code| code.starts_with("ORPHAN_"))
        .collect()
}

const WHITESPACE_DOCS: &[&str] = &[" ", "\t", "\n", " \t\r\n "];

#[tokio::test(flavor = "multi_thread")]
async fn whitespace_orphan_reason_written_by_put_does_not_suppress() {
    let service = service().await;
    for (n, doc) in WHITESPACE_DOCS.iter().enumerate() {
        let ns = format!("sr-put-{n}");
        put(&service, orphan_event(&ns, "legacy", doc)).await;
        put(&service, event_model(&ns, "board", id(&ns, "legacy"))).await;

        assert_eq!(
            orphan_codes(&service, id(&ns, "board")).await,
            vec!["ORPHAN_DOC_EMPTY"],
            "{doc:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn whitespace_orphan_reason_written_by_batch_mutate_does_not_suppress() {
    let service = service().await;
    for (n, doc) in WHITESPACE_DOCS.iter().enumerate() {
        let ns = format!("sr-batch-{n}");
        batch_put(
            &service,
            vec![
                orphan_event(&ns, "legacy", doc),
                event_model(&ns, "board", id(&ns, "legacy")),
            ],
        )
        .await;

        assert_eq!(
            orphan_codes(&service, id(&ns, "board")).await,
            vec!["ORPHAN_DOC_EMPTY"],
            "{doc:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stated_orphan_reason_suppresses() {
    let service = service().await;
    put(
        &service,
        orphan_event("sr-stated", "legacy", "  kept for replaying v1 streams  "),
    )
    .await;
    put(
        &service,
        event_model("sr-stated", "board", id("sr-stated", "legacy")),
    )
    .await;

    assert_eq!(
        orphan_codes(&service, id("sr-stated", "board")).await,
        Vec::<String>::new()
    );
}
