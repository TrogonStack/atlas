//! `delete_entity` refuses to delete a referenced entity unless the caller
//! asks for `mode=FORCE`. A `DeleteEntityRequest` embedded in a
//! `BatchMutateOp` must refuse it too -- a batch is not a side door around
//! the same contract.

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

fn event(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

fn event_superseding(ns: &str, slug: &str, supersedes: &pb::Id) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: slug.into(),
            supersedes: Some(supersedes.clone()),
            ..Default::default()
        })),
    }
}

async fn put(c: &mut EventModelServiceClient<Channel>, entity: pb::Entity) {
    c.put_entity(pb::PutEntityRequest {
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();
}

fn delete_op(ns: &str, slug: &str, mode: pb::delete_entity_request::Mode) -> pb::BatchMutateOp {
    pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(ns, slug)),
            mode: mode as i32,
            if_match: String::new(),
            operation_id: String::new(),
        })),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_delete_of_a_referenced_entity_fails_instead_of_applying() {
    let mut c = client().await;
    put(&mut c, event("batch-ref", "v1")).await;
    put(
        &mut c,
        event_superseding("batch-ref", "v2", &id("batch-ref", "v1")),
    )
    .await;

    for validate_only in [true, false] {
        let resp = c
            .batch_mutate(pb::BatchMutateRequest {
                ops: vec![delete_op(
                    "batch-ref",
                    "v1",
                    pb::delete_entity_request::Mode::FailIfReferenced,
                )],
                validate_only,
                operation_id: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            resp.status,
            pb::batch_mutate_response::Status::Failed as i32,
            "validate_only={validate_only}: {resp:?}"
        );
        assert_eq!(resp.failure.len(), 1);
        assert_eq!(resp.failure[0].code, "ENTITY_REFERENCED");

        // The entity must still be there: a rejected batch commits nothing.
        let got = c
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("batch-ref", "v1")),
            })
            .await;
        assert!(
            got.is_ok(),
            "validate_only={validate_only}: entity must survive a rejected batch"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_delete_of_a_referenced_entity_with_force_mode_still_applies() {
    let mut c = client().await;
    put(&mut c, event("batch-ref-force", "v1")).await;
    put(
        &mut c,
        event_superseding("batch-ref-force", "v2", &id("batch-ref-force", "v1")),
    )
    .await;

    let resp = c
        .batch_mutate(pb::BatchMutateRequest {
            ops: vec![delete_op(
                "batch-ref-force",
                "v1",
                pb::delete_entity_request::Mode::Force,
            )],
            validate_only: false,
            operation_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );

    // FORCE still deletes a referenced entity, but the caller should learn
    // what it just orphaned instead of a result that looks like a clean
    // delete -- the same awareness delete_entity itself gives on success.
    let referrers = match resp.results[0].result.as_ref().unwrap() {
        pb::batch_mutate_op_result::Result::Delete(d) => &d.referrers,
        other @ pb::batch_mutate_op_result::Result::Put(_) => {
            panic!("expected a delete result, got {other:?}")
        }
    };
    assert_eq!(referrers.len(), 1, "{referrers:?}");

    let got = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("batch-ref-force", "v1")),
        })
        .await;
    assert!(got.is_err(), "FORCE must delete despite the referrer");
}
