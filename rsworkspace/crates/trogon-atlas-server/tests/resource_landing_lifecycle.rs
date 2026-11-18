#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

fn id(slug: &str) -> pb::Id {
    pb::Id {
        namespace: "commerce".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn on_branch<T>(body: T) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "x-trogon-atlas-branch",
        "resource-lifecycle".parse().unwrap(),
    );
    request
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind) -> pb::Entity {
    service
        .put_entity(Request::new(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(kind),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn fixture() -> (EventModelServiceImpl, pb::Entity) {
    let nats = trogon_atlas_testsupport::shared().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default())
            .unwrap();
    put(
        &service,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id("ready")),
            title: "Ready orders".into(),
            ..Default::default()
        }),
    )
    .await;
    let resource = put(
        &service,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id("order-cache")),
            title: "Order cache".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("ready")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "resource-lifecycle".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    (service, resource)
}

async fn delete_target(service: &EventModelServiceImpl, branch: bool) {
    let body = pb::DeleteEntityRequest {
        kind: pb::EntityKind::ReadModel as i32,
        id: Some(id("ready")),
        mode: pb::delete_entity_request::Mode::Force as i32,
        ..Default::default()
    };
    service
        .delete_entity(if branch {
            on_branch(body)
        } else {
            Request::new(body)
        })
        .await
        .unwrap();
}

async fn land(service: &EventModelServiceImpl) {
    for dry_run in [true, false] {
        let merged = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "resource-lifecycle".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            merged.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{merged:?}"
        );
    }
}

async fn assert_unresolved(service: &EventModelServiceImpl, kind: pb::EntityKind) {
    let result = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "commerce".into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(
        result
            .reports
            .iter()
            .flat_map(|report| &report.issues)
            .any(|issue| issue.code
                == if kind == pb::EntityKind::Channel {
                    "CHANNEL_OBSERVATION_UNRESOLVED"
                } else {
                    "ARCHITECTURE_REFERENCE_UNRESOLVED"
                }
                && issue
                    .subject
                    .as_ref()
                    .is_some_and(|subject| subject.kind == kind as i32)),
        "{result:?}"
    );
}

#[tokio::test]
async fn landing_forced_target_delete_preserves_unchanged_store_and_diagnostics() {
    let (service, resource) = fixture().await;
    delete_target(&service, true).await;
    land(&service).await;
    let stored = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::DataStore as i32,
            id: Some(id("order-cache")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(stored, resource);
    assert_unresolved(&service, pb::EntityKind::DataStore).await;
}

#[tokio::test]
async fn landing_unrelated_change_does_not_block_on_already_unresolved_store() {
    let (service, _) = fixture().await;
    delete_target(&service, false).await;
    service
        .put_entity(on_branch(pb::PutEntityRequest {
            entity: Some(pb::Entity {
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id("unrelated")),
                    title: "Unrelated event".into(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap();
    land(&service).await;
    assert_unresolved(&service, pb::EntityKind::DataStore).await;
}

#[tokio::test]
async fn landing_changed_resource_rejects_deleted_anchor_without_reviving_baseline_target() {
    let (service, mut resource) = fixture().await;
    let Some(pb::entity::Kind::DataStore(value)) = resource.kind.as_mut() else {
        panic!("DataStore")
    };
    value.doc = "Changed storage declaration".into();
    service
        .put_entity(on_branch(pb::PutEntityRequest {
            entity: Some(resource),
            ..Default::default()
        }))
        .await
        .unwrap();
    delete_target(&service, true).await;
    for dry_run in [true, false] {
        let error = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "resource-lifecycle".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::InvalidArgument, "{error}");
    }
    service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("ready")),
        }))
        .await
        .unwrap();
}

#[tokio::test]
async fn landing_accepted_processor_retarget_preserves_stale_channel_selector() {
    let (service, _) = fixture().await;
    for slug in ["consumer", "replacement-consumer"] {
        put(
            &service,
            pb::entity::Kind::Processor(pb::Processor {
                id: Some(id(slug)),
                title: slug.into(),
                ..Default::default()
            }),
        )
        .await;
    }
    let mut owner = put(
        &service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id("observation")),
            title: "Observe ready".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id("ready")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id("consumer")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    )
    .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_ref() else {
        panic!("AutomationSlice")
    };
    let channel = put(
        &service,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id("delivery")),
            title: "Delivery".into(),
            bindings: vec![pb::ChannelBinding {
                observation: Some(pb::ConnectionRef {
                    owner: Some(pb::EntityRef {
                        kind: pb::EntityKind::AutomationSlice as i32,
                        id: slice.id.clone(),
                    }),
                    owner_uid: owner.system.as_ref().unwrap().uid.clone(),
                    id: slice.source_read_models[0].connection_id.clone(),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    )
    .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_mut() else {
        panic!("AutomationSlice")
    };
    slice.source_read_models[0].connection_id = None;
    slice.processor = Some(pb::ProcessorEdge {
        processor: Some(pb::ProcessorRef {
            id: Some(id("replacement-consumer")),
        }),
        ..Default::default()
    });
    service
        .put_entity(on_branch(pb::PutEntityRequest {
            entity: Some(owner),
            ..Default::default()
        }))
        .await
        .unwrap();
    land(&service).await;
    let stored = service
        .get_entity(Request::new(pb::GetEntityRequest {
            kind: pb::EntityKind::Channel as i32,
            id: Some(id("delivery")),
        }))
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap();
    assert_eq!(stored, channel);
    assert_unresolved(&service, pb::EntityKind::Channel).await;
}
