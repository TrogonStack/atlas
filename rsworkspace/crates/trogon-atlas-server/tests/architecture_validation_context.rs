#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::Request;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{store::ListFilter, Store, WriteContext};

const BRANCH: &str = "resource-diagnostics";

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn on_branch<T>(body: T) -> Request<T> {
    let mut request = Request::new(body);
    request
        .metadata_mut()
        .insert("x-trogon-atlas-branch", BRANCH.parse().unwrap());
    request
}

async fn put(service: &EventModelServiceImpl, kind: pb::entity::Kind, branch: bool) -> pb::Entity {
    let request = pb::PutEntityRequest {
        entity: Some(pb::Entity {
            kind: Some(kind),
            ..Default::default()
        }),
        ..Default::default()
    };
    service
        .put_entity(if branch {
            on_branch(request)
        } else {
            Request::new(request)
        })
        .await
        .unwrap()
        .into_inner()
        .entity
        .unwrap()
}

async fn fixture(
    service: &EventModelServiceImpl,
    resource_namespace: &str,
    target_namespace: &str,
) {
    put(
        service,
        pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(target_namespace, "source")),
            title: "Hidden read target display".into(),
            ..Default::default()
        }),
        false,
    )
    .await;
    put(
        service,
        pb::entity::Kind::Processor(pb::Processor {
            id: Some(id(resource_namespace, "consumer")),
            title: "Consumer".into(),
            ..Default::default()
        }),
        false,
    )
    .await;
    put(
        service,
        pb::entity::Kind::Component(pb::Component {
            id: Some(id(target_namespace, "broker")),
            title: "Hidden broker display".into(),
            ..Default::default()
        }),
        false,
    )
    .await;
    let owner = put(
        service,
        pb::entity::Kind::AutomationSlice(pb::AutomationSlice {
            id: Some(id(resource_namespace, "observation")),
            title: "Observe".into(),
            source_read_models: vec![pb::ReadModelEdge {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id(target_namespace, "source")),
                }),
                ..Default::default()
            }],
            processor: Some(pb::ProcessorEdge {
                processor: Some(pb::ProcessorRef {
                    id: Some(id(resource_namespace, "consumer")),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        false,
    )
    .await;
    let Some(pb::entity::Kind::AutomationSlice(slice)) = owner.kind.as_ref() else {
        panic!()
    };
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: BRANCH.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    put(
        service,
        pb::entity::Kind::Channel(pb::Channel {
            id: Some(id(resource_namespace, "transport")),
            title: "Transport".into(),
            broker: Some(pb::channel::Broker::Component(pb::ComponentRef {
                id: Some(id(target_namespace, "broker")),
            })),
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
        true,
    )
    .await;
    put(
        service,
        pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id(resource_namespace, "storage")),
            title: "Storage".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef {
                    id: Some(id(target_namespace, "source")),
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
        true,
    )
    .await;
}

async fn service() -> EventModelServiceImpl {
    let nats = trogon_atlas_testsupport::shared().await;
    EventModelServiceImpl::try_new(Arc::new(nats.store().await), ServerMetrics::default()).unwrap()
}

async fn issues(service: &EventModelServiceImpl, namespace: &str) -> Vec<pb::ValidationIssue> {
    service
        .validate_project(on_branch(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                namespace.into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner()
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .filter(|issue| {
            issue.code.starts_with("ARCHITECTURE_")
                || issue.code == "CHANNEL_OBSERVATION_UNRESOLVED"
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn native_capped_project_validation_resolves_present_anchors() {
    let nats = trogon_atlas_testsupport::shared().await;
    let backend = Arc::new(nats.store().await);
    let seed = EventModelServiceImpl::try_new(backend.clone(), ServerMetrics::default()).unwrap();
    fixture(&seed, "validation-context", "validation-context").await;
    seed.merge_branch(Request::new(pb::MergeBranchRequest {
        name: BRANCH.into(),
        ..Default::default()
    }))
    .await
    .unwrap();
    let mut writes = tokio::task::JoinSet::new();
    for index in 0..10_064 {
        let backend = backend.clone();
        writes.spawn(async move {
            backend
                .create(
                    pb::EntityKind::DataStore,
                    &pb::Entity {
                        kind: Some(pb::entity::Kind::DataStore(pb::DataStore {
                            id: Some(id("validation-context", &format!("z-storage-{index:05}"))),
                            title: "Storage declaration".into(),
                            store_class: Some(pb::StoreClass {
                                kind: Some(pb::store_class::Kind::Known(
                                    pb::store_class::Known::Cache as i32,
                                )),
                            }),
                            attachments: vec![pb::ReadModelAttachment {
                                read_model: Some(pb::ReadModelRef {
                                    id: Some(id("validation-context", "source")),
                                }),
                                ..Default::default()
                            }],
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                    WriteContext::baseline(),
                )
                .await
                .unwrap();
        });
        if writes.len() == 32 {
            writes.join_next().await.unwrap().unwrap();
        }
    }
    while let Some(result) = writes.join_next().await {
        result.unwrap();
    }
    for (kind, slug) in [
        (pb::EntityKind::ReadModel, "source"),
        (pb::EntityKind::Processor, "consumer"),
        (pb::EntityKind::Component, "broker"),
        (pb::EntityKind::AutomationSlice, "observation"),
    ] {
        let stored = backend
            .get(kind, &id("validation-context", slug), None)
            .await
            .unwrap();
        let mut entity = stored.entity;
        match entity.kind.as_mut().unwrap() {
            pb::entity::Kind::ReadModel(value) => value.doc = "Retained target".into(),
            pb::entity::Kind::Processor(value) => value.doc = "Retained target".into(),
            pb::entity::Kind::Component(value) => value.doc = "Retained target".into(),
            pb::entity::Kind::AutomationSlice(value) => value.doc = "Retained target".into(),
            _ => panic!(),
        }
        backend
            .update(kind, &entity, &stored.etag, false, WriteContext::baseline())
            .await
            .unwrap();
    }
    let projected = backend
        .list(ListFilter::default(), Some(2), None)
        .await
        .unwrap();
    assert_eq!(projected.len(), 2);
    assert!(projected
        .iter()
        .any(|stored| matches!(stored.entity.kind, Some(pb::entity::Kind::Channel(_)))));
    assert!(projected
        .iter()
        .any(|stored| matches!(stored.entity.kind, Some(pb::entity::Kind::DataStore(_)))));
    for (kind, slug) in [
        (pb::EntityKind::ReadModel, "source"),
        (pb::EntityKind::Processor, "consumer"),
        (pb::EntityKind::Component, "broker"),
        (pb::EntityKind::AutomationSlice, "observation"),
    ] {
        backend
            .get(kind, &id("validation-context", slug), None)
            .await
            .unwrap();
    }
    let service = EventModelServiceImpl::try_new(backend, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(2);
    let snapshot = service
        .get_snapshot_id(Request::new(pb::GetSnapshotIdRequest {
            namespaces: vec!["validation-context".into()],
            include_entries: true,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(snapshot.truncated);
    assert_eq!(snapshot.entries.len(), 2);
    assert!(snapshot
        .entries
        .iter()
        .all(|entry| entry.entity.as_ref().is_some_and(|entity| [
            pb::EntityKind::Channel as i32,
            pb::EntityKind::DataStore as i32
        ]
        .contains(&entity.kind))));
    let response = service
        .validate_project(Request::new(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::Namespace(
                "validation-context".into(),
            )),
        }))
        .await
        .unwrap()
        .into_inner();
    let findings: Vec<_> = response
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .filter(|issue| {
            issue.code.starts_with("ARCHITECTURE_")
                || issue.code == "CHANNEL_OBSERVATION_UNRESOLVED"
        })
        .collect();
    assert_eq!(findings, Vec::<pb::ValidationIssue>::new());
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: "cap-land".into(),
            ..Default::default()
        }))
        .await
        .unwrap();
    let mut write = Request::new(pb::PutEntityRequest {
        entity: Some(pb::Entity {
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(id("validation-context", "unrelated-change")),
                title: "Unrelated change".into(),
                ..Default::default()
            })),
            ..Default::default()
        }),
        ..Default::default()
    });
    write
        .metadata_mut()
        .insert("x-trogon-atlas-branch", "cap-land".parse().unwrap());
    service.put_entity(write).await.unwrap();
    for dry_run in [true, false] {
        let response = service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: "cap-land".into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            response.status,
            if dry_run {
                pb::merge_branch_response::Status::Validated
            } else {
                pb::merge_branch_response::Status::Applied
            } as i32,
            "{response:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn validation_keeps_actual_branch_target_deletion_unresolved() {
    let service = service().await;
    fixture(&service, "validation-context", "validation-context").await;
    service
        .delete_entity(on_branch(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("validation-context", "source")),
            mode: pb::delete_entity_request::Mode::Force as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let findings = issues(&service, "validation-context").await;
    assert!(findings
        .iter()
        .any(|issue| issue.code == "CHANNEL_OBSERVATION_UNRESOLVED"));
    assert!(findings
        .iter()
        .any(|issue| issue.code == "ARCHITECTURE_REFERENCE_UNRESOLVED"
            && issue
                .subject
                .as_ref()
                .is_some_and(|subject| subject.kind == pb::EntityKind::DataStore as i32)));
    for dry_run in [true, false] {
        assert!(service
            .merge_branch(Request::new(pb::MergeBranchRequest {
                name: BRANCH.into(),
                dry_run,
                ..Default::default()
            }))
            .await
            .is_err());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn project_diagnostics_do_not_resolve_or_disclose_unreadable_anchors() {
    use pb::{
        event_model_service_client::EventModelServiceClient,
        event_model_service_server::EventModelServiceServer,
    };
    use tonic::service::interceptor::InterceptedService;
    use trogon_atlas_server::auth::{AuthzLayer, BearerAuth, TokenRegistry};

    let nats = trogon_atlas_testsupport::shared().await;
    let backend = Arc::new(nats.store().await);
    let service = EventModelServiceImpl::try_new(backend, ServerMetrics::default()).unwrap();
    let mut namespaces = Vec::new();
    for (name, parent) in [("resources", "acme"), ("anchors", "other")] {
        namespaces.push(
            service
                .register_namespace(Request::new(pb::RegisterNamespaceRequest {
                    name: name.into(),
                    parent: parent.into(),
                }))
                .await
                .unwrap()
                .into_inner()
                .namespace
                .unwrap()
                .id,
        );
    }
    fixture(&service, &namespaces[0], &namespaces[1]).await;
    let registry = toml::from_str::<trogon_atlas_server::auth::RegistryFile>(
        r#"
[principals.resource-reader]
role = "reader"
tokens = ["diagnostic-scope-reader"]
parent = "acme"
"#,
    )
    .unwrap();
    let grpc = EventModelServiceServer::new(service);
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
        BearerAuth::new(Arc::new(
            TokenRegistry::from_file_contents(registry).unwrap(),
        )),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client = EventModelServiceClient::connect(format!("http://{address}"))
        .await
        .unwrap();
    let mut request = on_branch(pb::ValidateProjectRequest {
        scope: Some(pb::validate_project_request::Scope::Namespace(
            namespaces[0].clone(),
        )),
    });
    request.metadata_mut().insert(
        "authorization",
        "Bearer diagnostic-scope-reader".parse().unwrap(),
    );
    let response = client.validate_project(request).await.unwrap().into_inner();
    let rendered = format!("{response:?}");
    assert!(!rendered.contains("Hidden read target display"));
    assert!(!rendered.contains("Hidden broker display"));
    let findings: Vec<_> = response
        .reports
        .into_iter()
        .flat_map(|report| report.issues)
        .collect();
    assert!(findings
        .iter()
        .any(|issue| issue.code == "CHANNEL_OBSERVATION_UNRESOLVED"));
    assert!(findings
        .iter()
        .any(|issue| issue.code == "ARCHITECTURE_REFERENCE_UNRESOLVED"
            && issue.subject_field == "component"));
    assert!(findings
        .iter()
        .any(|issue| issue.code == "ARCHITECTURE_REFERENCE_UNRESOLVED"
            && issue.subject_field == "attachments.read_model"));
}
