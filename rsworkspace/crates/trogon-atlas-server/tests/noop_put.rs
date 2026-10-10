#![allow(clippy::unwrap_used, clippy::expect_used)]

// Idempotency as a store property: a put whose content is semantically
// identical to the stored row is skipped (no revision bump, no change
// event) unless forced. In-process gRPC server over an ephemeral
// JetStream store, same pattern as the other integration tests.

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::service::EventModelServiceImpl;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn client(endpoint: &str) -> EventModelServiceClient<tonic::transport::Channel> {
    EventModelServiceClient::connect(endpoint.to_string())
        .await
        .unwrap()
}

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity, force: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force,
    }
}

/// Current change-feed cursor (opaque resume token).
async fn change_cursor(c: &mut EventModelServiceClient<tonic::transport::Channel>) -> String {
    c.list_changes(pb::ListChangesRequest {
        since_token: String::new(),
        page_size: 1,
        scopes: vec![],
    })
    .await
    .unwrap()
    .into_inner()
    .next_token
}

/// Count change events recorded after `since` for one namespace. The
/// ephemeral store is shared across parallel tests, so assertions must be
/// namespace-scoped, never global-sequence arithmetic.
async fn changes_in_ns(
    c: &mut EventModelServiceClient<tonic::transport::Channel>,
    since: &str,
    ns: &str,
) -> usize {
    let mut count = 0;
    let mut token = since.to_string();
    loop {
        let page = c
            .list_changes(pb::ListChangesRequest {
                since_token: token.clone(),
                page_size: 100,
                scopes: vec![],
            })
            .await
            .unwrap()
            .into_inner();
        if page.events.is_empty() {
            return count;
        }
        count += page
            .events
            .iter()
            .filter(|e| {
                e.entity
                    .as_ref()
                    .and_then(|r| r.id.as_ref())
                    .is_some_and(|id| id.namespace == ns)
            })
            .count();
        if page.next_token == token {
            return count;
        }
        token = page.next_token;
    }
}

#[tokio::test]
async fn identical_put_is_a_noop_and_force_overwrites() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let first = c
        .put_entity(put_req(event("noop-ns", "thing.happened", "Thing"), false))
        .await
        .unwrap()
        .into_inner();
    assert!(!first.no_op);
    assert_ne!(first.etag, "");
    let stored = first.entity.clone().unwrap();
    assert!(stored.system.is_some(), "response carries the stored stamp");
    let uid = stored.system.clone().unwrap().uid;
    let cursor = change_cursor(&mut c).await;

    // Identical content again: skipped, same etag, no change event.
    let second = c
        .put_entity(put_req(event("noop-ns", "thing.happened", "Thing"), false))
        .await
        .unwrap()
        .into_inner();
    assert!(second.no_op, "identical put must be a no-op");
    assert_eq!(second.etag, first.etag, "etag must not advance on no-op");
    assert_eq!(
        second.entity.unwrap().system.unwrap().uid,
        uid,
        "incarnation identity is preserved"
    );
    assert_eq!(
        changes_in_ns(&mut c, &cursor, "noop-ns").await,
        0,
        "no change event on no-op"
    );

    // Forced: writes even though identical; uid still preserved.
    let forced = c
        .put_entity(put_req(event("noop-ns", "thing.happened", "Thing"), true))
        .await
        .unwrap()
        .into_inner();
    assert!(!forced.no_op);
    assert_ne!(forced.etag, first.etag, "force must bump the revision");
    assert_eq!(forced.entity.unwrap().system.unwrap().uid, uid);

    // Actually different content: a real write.
    let changed = c
        .put_entity(put_req(
            event("noop-ns", "thing.happened", "Renamed"),
            false,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!changed.no_op);
}

#[tokio::test]
async fn batch_mutate_reports_noop_per_op() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let put_op = |e: pb::Entity| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_req(e, false))),
    };
    let apply = |ops: Vec<pb::BatchMutateOp>| pb::BatchMutateRequest {
        operation_id: String::new(),
        ops,
        validate_only: false,
    };

    let first = c
        .batch_mutate(apply(vec![
            put_op(event("noop-batch", "a", "A")),
            put_op(event("noop-batch", "b", "B")),
        ]))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        first.status,
        pb::batch_mutate_response::Status::Applied as i32
    );

    let cursor = change_cursor(&mut c).await;

    // Re-apply one identical, one changed: exactly one write, one no-op.
    let second = c
        .batch_mutate(apply(vec![
            put_op(event("noop-batch", "a", "A")),
            put_op(event("noop-batch", "b", "B2")),
        ]))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        second.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
    let noops: Vec<bool> = second
        .results
        .iter()
        .map(|r| match &r.result {
            Some(pb::batch_mutate_op_result::Result::Put(p)) => p.no_op,
            _ => panic!("expected put results"),
        })
        .collect();
    assert_eq!(noops, vec![true, false]);

    // Exactly one change event was recorded for the second batch.
    assert_eq!(
        changes_in_ns(&mut c, &cursor, "noop-batch").await,
        1,
        "one write, one change event"
    );
}
