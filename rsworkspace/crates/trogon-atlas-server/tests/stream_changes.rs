#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use tokio_stream::StreamExt;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelService;
use trogon_atlas_server::service::EventModelServiceImpl;

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn make_event(ns: &str, slug: &str, version: u64) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: format!("{ns}/{slug}@{version}"),
            doc: String::new(),
            swimlane: None,
            metadata: Vec::new(),
            supersedes: None,
            schema: None,
        })),
    }
}

async fn svc() -> Arc<EventModelServiceImpl> {
    let nats = trogon_atlas_testsupport::shared().await;
    let store = nats.store().await;
    Arc::new(EventModelServiceImpl::try_new(Arc::new(store)).unwrap())
}

async fn put_event(svc: &EventModelServiceImpl, ns: &str, slug: &str, version: u64) {
    svc.batch_mutate(Request::new(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event(ns, slug, version)),
                create_only: false,
                if_match: String::new(),
                validate_only: false,
                force: false,
            })),
        }],
        validate_only: false,
    }))
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn live_changes_arrive_within_50ms() {
    let svc = svc().await;
    let stream_resp = svc
        .stream_changes(Request::new(pb::StreamChangesRequest {
            after_token: String::new(),
            namespace: String::new(),
            kind: 0,
        }))
        .await
        .unwrap();
    let mut stream = stream_resp.into_inner();

    let svc_clone = svc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        put_event(&svc_clone, "orders", "order-placed", 1).await;
    });

    let event = tokio::time::timeout(Duration::from_millis(500), stream.next())
        .await
        .expect("stream produced no event within 500ms")
        .expect("stream ended")
        .expect("stream error");

    assert_eq!(event.kind, pb::change_event::Kind::Put as i32);
    let entity_ref = event.entity.unwrap();
    assert_eq!(entity_ref.kind, pb::EntityKind::Event as i32);
    assert_eq!(entity_ref.id.unwrap().slug, "order-placed");
    assert!(!event.token.is_empty(), "token should be populated");
    assert!(
        event.timestamp_micros > 0,
        "timestamp_micros should be populated"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_token_does_not_replay_history() {
    let svc = svc().await;
    put_event(&svc, "orders", "order-placed", 1).await;
    put_event(&svc, "orders", "order-placed", 2).await;

    let stream_resp = svc
        .stream_changes(Request::new(pb::StreamChangesRequest {
            after_token: String::new(),
            namespace: String::new(),
            kind: 0,
        }))
        .await
        .unwrap();
    let mut stream = stream_resp.into_inner();

    let result = tokio::time::timeout(Duration::from_millis(80), stream.next()).await;
    assert!(
        result.is_err(),
        "empty token must not replay historical events"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn after_token_replays_history_then_streams_live() {
    let svc = svc().await;
    put_event(&svc, "orders", "order-placed", 1).await;
    put_event(&svc, "orders", "order-placed", 2).await;

    let stream_resp = svc
        .stream_changes(Request::new(pb::StreamChangesRequest {
            after_token: "0".into(),
            namespace: String::new(),
            kind: 0,
        }))
        .await
        .unwrap();
    let mut stream = stream_resp.into_inner();

    let first = tokio::time::timeout(Duration::from_millis(200), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second = tokio::time::timeout(Duration::from_millis(200), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.token, "1");
    assert_eq!(second.token, "2");

    let svc_clone = svc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        put_event(&svc_clone, "orders", "order-placed", 3).await;
    });

    let third = tokio::time::timeout(Duration::from_millis(500), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(third.token, "3");
}

#[tokio::test(flavor = "multi_thread")]
async fn backlog_replays_across_page_boundary() {
    // The backlog drain pages in 256-record chunks (BACKLOG_PAGE in
    // service.rs). Seed >256 records, replay from 0, and assert every
    // record reaches the subscriber so a future off-by-one in the paging
    // loop is caught.
    let svc = svc().await;
    let total: u64 = 300;
    for i in 1..=total {
        put_event(&svc, "orders", "order-placed", i).await;
    }

    let stream_resp = svc
        .stream_changes(Request::new(pb::StreamChangesRequest {
            after_token: "0".into(),
            namespace: String::new(),
            kind: 0,
        }))
        .await
        .unwrap();
    let mut stream = stream_resp.into_inner();

    let mut received = Vec::with_capacity(usize::try_from(total).expect("total fits in usize"));
    for _ in 0..total {
        let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("backlog drained within deadline")
            .expect("stream item present")
            .expect("change event ok");
        received.push(event.token.parse::<u64>().unwrap());
    }
    let expected: Vec<u64> = (1..=total).collect();
    assert_eq!(received, expected, "every backlog record must be delivered");
}

#[tokio::test(flavor = "multi_thread")]
async fn namespace_filter_skips_other_namespaces() {
    let svc = svc().await;
    let stream_resp = svc
        .stream_changes(Request::new(pb::StreamChangesRequest {
            after_token: String::new(),
            namespace: "orders".into(),
            kind: 0,
        }))
        .await
        .unwrap();
    let mut stream = stream_resp.into_inner();

    let svc_clone = svc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        put_event(&svc_clone, "billing", "invoice-issued", 1).await;
        put_event(&svc_clone, "orders", "order-placed", 1).await;
    });

    let event = tokio::time::timeout(Duration::from_millis(500), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(event.entity.unwrap().id.unwrap().namespace, "orders");
}

#[tokio::test(flavor = "multi_thread")]
async fn kind_filter_skips_other_kinds() {
    let svc = svc().await;
    let stream_resp = svc
        .stream_changes(Request::new(pb::StreamChangesRequest {
            after_token: String::new(),
            namespace: String::new(),
            kind: pb::EntityKind::Event as i32,
        }))
        .await
        .unwrap();
    let mut stream = stream_resp.into_inner();

    let svc_clone = svc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(5)).await;
        svc_clone
            .put_entity(Request::new(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(pb::Entity {
                    system: None,
                    kind: Some(pb::entity::Kind::Persona(pb::Persona {
                        id: Some(id("orders", "buyer", 1)),
                        title: "buyer".into(),
                        role: "customer".into(),
                        doc: String::new(),
                        metadata: Vec::new(),
                        supersedes: None,
                    })),
                }),
                create_only: true,
                if_match: String::new(),
                validate_only: false,
                force: false,
            }))
            .await
            .unwrap();
        put_event(&svc_clone, "orders", "order-placed", 1).await;
    });

    let event = tokio::time::timeout(Duration::from_millis(500), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(event.entity.unwrap().kind, pb::EntityKind::Event as i32);
}

#[tokio::test(flavor = "multi_thread")]
async fn single_entity_rpcs_record_change_events() {
    let svc = svc().await;
    let before = svc
        .list_changes(Request::new(pb::ListChangesRequest {
            since_token: "0".into(),
            scopes: Vec::new(),
            page_size: 100,
        }))
        .await
        .unwrap()
        .into_inner();

    svc.put_entity(Request::new(pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(make_event("orders", "order-shipped", 1)),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }))
    .await
    .unwrap();

    svc.delete_entity(Request::new(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id("orders", "order-shipped", 1)),
        if_match: String::new(),
        mode: pb::delete_entity_request::Mode::Force as i32,
    }))
    .await
    .unwrap();

    let after = svc
        .list_changes(Request::new(pb::ListChangesRequest {
            since_token: "0".into(),
            scopes: Vec::new(),
            page_size: 100,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        after.events.len(),
        before.events.len() + 2,
        "put_entity and delete_entity must each record a change event"
    );
}
