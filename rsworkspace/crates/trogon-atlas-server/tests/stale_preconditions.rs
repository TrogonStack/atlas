//! A caller plans a write from a read and enforces that read at apply time.
//! These tests pin the wire contract that makes that possible: lookups report
//! the etag each planned write must match, and a refused precondition comes
//! back as structured data naming the entity and both revisions.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::transport::Channel;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::service::EventModelServiceImpl;

async fn client() -> EventModelServiceClient<Channel> {
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
    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn event_ref(ns: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, slug)),
    }
}

fn put(entity: pb::Entity, create_only: bool, if_match: &str) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only,
        if_match: if_match.into(),
        validate_only: false,
        force: false,
    }
}

async fn put_etag(c: &mut EventModelServiceClient<Channel>, entity: pb::Entity) -> String {
    c.put_entity(put(entity, false, ""))
        .await
        .unwrap()
        .into_inner()
        .etag
}

async fn batch_put(
    c: &mut EventModelServiceClient<Channel>,
    op: pb::PutEntityRequest,
    validate_only: bool,
) -> pb::BatchMutateResponse {
    c.batch_mutate(pb::BatchMutateRequest {
        operation_id: String::new(),
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(op)),
        }],
        validate_only,
    })
    .await
    .unwrap()
    .into_inner()
}

fn expect_precondition_failure(resp: &pb::BatchMutateResponse) -> &pb::PreconditionFailure {
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32,
        "{resp:?}"
    );
    resp.precondition_failure
        .as_ref()
        .expect("a refused precondition must be reported as structured data")
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_reports_the_etag_each_planned_write_must_match() {
    let mut c = client().await;
    let etag = put_etag(&mut c, event("stale-etags", "present", "Present")).await;

    for dense in [true, false] {
        let resp = c
            .batch_get_entities(pb::BatchGetEntitiesRequest {
                keys: vec![
                    event_ref("stale-etags", "present"),
                    event_ref("stale-etags", "absent"),
                ],
                dense,
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(resp.etags.len(), resp.entities.len(), "dense={dense}");
        assert_eq!(resp.etags[0], etag, "dense={dense}");
        if dense {
            assert_eq!(resp.etags[1], "", "a placeholder carries no etag");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_only_against_a_concurrent_create_reports_both_revisions() {
    let mut c = client().await;
    let actual = put_etag(&mut c, event("stale-create", "e", "Other agent")).await;

    for validate_only in [true, false] {
        let resp = batch_put(
            &mut c,
            put(event("stale-create", "e", "Mine"), true, ""),
            validate_only,
        )
        .await;
        let failure = expect_precondition_failure(&resp);
        assert_eq!(failure.subject, Some(event_ref("stale-create", "e")));
        assert_eq!(failure.expected_etag, "", "the plan expected no entity");
        assert_eq!(failure.actual_etag, actual, "validate_only={validate_only}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn if_match_against_a_newer_revision_reports_both_revisions() {
    let mut c = client().await;
    let planned = put_etag(&mut c, event("stale-update", "e", "Base")).await;
    let actual = put_etag(&mut c, event("stale-update", "e", "Other agent")).await;

    for validate_only in [true, false] {
        let resp = batch_put(
            &mut c,
            put(event("stale-update", "e", "Mine"), false, &planned),
            validate_only,
        )
        .await;
        let failure = expect_precondition_failure(&resp);
        assert_eq!(failure.subject, Some(event_ref("stale-update", "e")));
        assert_eq!(failure.expected_etag, planned);
        assert_eq!(failure.actual_etag, actual, "validate_only={validate_only}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn if_match_against_a_deleted_entity_reports_it_absent() {
    let mut c = client().await;
    let planned = put_etag(&mut c, event("stale-delete", "e", "Base")).await;
    c.delete_entity(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(id("stale-delete", "e")),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        if_match: String::new(),
    })
    .await
    .unwrap();

    for validate_only in [true, false] {
        let resp = batch_put(
            &mut c,
            put(event("stale-delete", "e", "Mine"), false, &planned),
            validate_only,
        )
        .await;
        let failure = expect_precondition_failure(&resp);
        assert_eq!(failure.expected_etag, planned);
        assert_eq!(failure.actual_etag, "", "validate_only={validate_only}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unrelated_batch_failure_is_not_reported_as_stale() {
    let mut c = client().await;
    let resp = c
        .batch_mutate(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                    operation_id: String::new(),
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("stale-unrelated", "missing")),
                    mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
                    if_match: String::new(),
                })),
            }],
            validate_only: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Failed as i32
    );
    assert_eq!(resp.precondition_failure, None);
}
