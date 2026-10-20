#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "merge-preview".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(slug: &str) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id(slug)),
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn on_branch<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    request
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "deletion".parse().unwrap());
    request
}

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0)
}

async fn create_branch(service: &EventModelServiceImpl) {
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "deletion".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
}

async fn delete_on_branch(service: &EventModelServiceImpl) {
    service
        .delete_entity(on_branch(pb::DeleteEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("removed")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_preview_deletes_a_baseline_entity_omitted_from_the_snapshot() {
    let service = service().await;
    service
        .put_entity(Request::new(event("removed")))
        .await
        .unwrap();
    create_branch(&service).await;
    delete_on_branch(&service).await;
    for dry_run in [true, false] {
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "deletion".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(merged.deleted_count, 1);
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32
        );
        let current = service
            .get_entity(Request::new(pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("removed")),
            }))
            .await;
        assert_eq!(current.is_ok(), dry_run);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_preview_rejects_a_delete_that_never_existed_on_baseline() {
    let service = service().await;
    create_branch(&service).await;
    service
        .put_entity(on_branch(event("removed")))
        .await
        .unwrap();
    delete_on_branch(&service).await;
    let mut errors = Vec::new();
    for dry_run in [true, false] {
        errors.push(
            service
                .merge_branch(Request::new(pb::MergeBranchRequest {
                    name: "deletion".into(),
                    dry_run,
                    ..Default::default()
                }))
                .await
                .unwrap_err()
                .code(),
        );
    }
    assert_eq!(errors, [Code::NotFound, Code::Aborted]);
}
