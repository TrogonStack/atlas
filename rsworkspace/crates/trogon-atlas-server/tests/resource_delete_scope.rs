#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use pb::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::{EventModelService, EventModelServiceServer},
};
use tonic::{service::interceptor::InterceptedService, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{store::WriteContext, Store};

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn read_model(namespace: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(id(namespace, slug)),
            title: slug.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn data_store(namespace: &str, slug: &str, target: pb::Id) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::DataStore(pb::DataStore {
            id: Some(id(namespace, slug)),
            title: "Private store details".into(),
            store_class: Some(pb::StoreClass {
                kind: Some(pb::store_class::Kind::Known(
                    pb::store_class::Known::Cache as i32,
                )),
            }),
            attachments: vec![pb::ReadModelAttachment {
                read_model: Some(pb::ReadModelRef { id: Some(target) }),
                ..Default::default()
            }],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn authenticated<T>(body: T) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        "Bearer scoped-resource-writer".parse().unwrap(),
    );
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_resource_volume_does_not_block_bound_delete_or_expose_hidden_referrers() {
    let nats = trogon_atlas_testsupport::shared().await;
    let backend = Arc::new(nats.store().await);
    let seed = EventModelServiceImpl::try_new(backend.clone(), ServerMetrics::default()).unwrap();
    let mut namespaces = Vec::new();
    for (name, parent) in [("commerce", "acme"), ("foreign", "other")] {
        namespaces.push(
            seed.register_namespace(Request::new(pb::RegisterNamespaceRequest {
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
    let own = namespaces[0].clone();
    let foreign = namespaces[1].clone();
    for slug in ["unreferenced", "batch-unreferenced", "hidden-target"] {
        backend
            .create(
                pb::EntityKind::ReadModel,
                &read_model(&own, slug),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    backend
        .create(
            pb::EntityKind::ReadModel,
            &read_model(&foreign, "feed"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let mut writes = tokio::task::JoinSet::new();
    for index in 0..5001 {
        let backend = backend.clone();
        let namespace = foreign.clone();
        writes.spawn(async move {
            backend
                .create(
                    pb::EntityKind::DataStore,
                    &data_store(
                        &namespace,
                        &format!("foreign-store-{index}"),
                        id(&namespace, "feed"),
                    ),
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
    backend
        .create(
            pb::EntityKind::DataStore,
            &data_store(&foreign, "hidden-referrer", id(&own, "hidden-target")),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let registry = toml::from_str::<trogon_atlas_server::auth::RegistryFile>(
        r#"
[principals.scoped-writer]
role = "writer"
tokens = ["scoped-resource-writer"]
parent = "acme"
"#,
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(backend, ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(0);
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
    client
        .delete_entity(authenticated(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(&own, "unreferenced")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap();
    let error = client
        .delete_entity(authenticated(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(&own, "hidden-target")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::FailedPrecondition, "{error}");
    assert!(!error.message().contains(&foreign));
    assert!(!error.message().contains("hidden-referrer"));
    assert!(!error.message().contains("Private store details"));
    let preview = client
        .delete_entity(authenticated(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(&own, "hidden-target")),
            mode: pb::delete_entity_request::Mode::DryRun as i32,
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(preview.referrers, Vec::<pb::Reference>::new());
    for validate_only in [false, true] {
        for mode in [
            pb::delete_entity_request::Mode::Unspecified,
            pb::delete_entity_request::Mode::FailIfReferenced,
        ] {
            let response = client
                .batch_mutate(authenticated(pb::BatchMutateRequest {
                    validate_only,
                    ops: vec![
                        pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                                kind: pb::EntityKind::ReadModel as i32,
                                id: Some(id(&own, "batch-unreferenced")),
                                mode: mode as i32,
                                ..Default::default()
                            })),
                        },
                        pb::BatchMutateOp {
                            op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                                kind: pb::EntityKind::ReadModel as i32,
                                id: Some(id(&own, "hidden-target")),
                                mode: mode as i32,
                                ..Default::default()
                            })),
                        },
                    ],
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(
                response.status,
                pb::batch_mutate_response::Status::Failed as i32,
                "{response:?}"
            );
            assert_eq!(response.failed_op_index, 1);
            assert_eq!(response.failure.len(), 1);
            assert_eq!(response.failure[0].code, "ENTITY_REFERENCED");
            let details = format!("{response:?}");
            assert!(!details.contains(&foreign));
            assert!(!details.contains("hidden-referrer"));
            assert!(!details.contains("Private store details"));
        }
    }
    client
        .get_entity(authenticated(pb::GetEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id(&own, "hidden-target")),
        }))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unrestricted_resource_volume_does_not_block_delete_modes_or_batches() {
    let nats = trogon_atlas_testsupport::shared().await;
    let backend = Arc::new(nats.store().await);
    backend
        .create(
            pb::EntityKind::ReadModel,
            &read_model("public", "feed"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let mut writes = tokio::task::JoinSet::new();
    for index in 0..5001 {
        let backend = backend.clone();
        writes.spawn(async move {
            backend
                .create(
                    pb::EntityKind::DataStore,
                    &data_store("public", &format!("store-{index}"), id("public", "feed")),
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
    let service = EventModelServiceImpl::try_new(backend.clone(), ServerMetrics::default())
        .unwrap()
        .with_snapshot_cap(10000);
    for mode in [
        pb::delete_entity_request::Mode::Force,
        pb::delete_entity_request::Mode::DryRun,
        pb::delete_entity_request::Mode::FailIfReferenced,
    ] {
        let slug = format!("standalone-{mode:?}").to_lowercase();
        backend
            .create(
                pb::EntityKind::ReadModel,
                &read_model("public", &slug),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
        service
            .delete_entity(Request::new(pb::DeleteEntityRequest {
                kind: pb::EntityKind::ReadModel as i32,
                id: Some(id("public", &slug)),
                mode: mode as i32,
                ..Default::default()
            }))
            .await
            .unwrap();
        assert_eq!(
            backend
                .get(pb::EntityKind::ReadModel, &id("public", &slug), None)
                .await
                .is_ok(),
            mode == pb::delete_entity_request::Mode::DryRun
        );
    }
    for validate_only in [true, false] {
        for mode in [
            pb::delete_entity_request::Mode::Force,
            pb::delete_entity_request::Mode::Unspecified,
            pb::delete_entity_request::Mode::FailIfReferenced,
        ] {
            let slug = format!("batch-{validate_only}-{mode:?}").to_lowercase();
            backend
                .create(
                    pb::EntityKind::ReadModel,
                    &read_model("public", &slug),
                    WriteContext::baseline(),
                )
                .await
                .unwrap();
            let service = EventModelServiceImpl::try_new(backend.clone(), ServerMetrics::default())
                .unwrap()
                .with_snapshot_cap(10000);
            let response = service
                .batch_mutate(Request::new(pb::BatchMutateRequest {
                    validate_only,
                    ops: vec![pb::BatchMutateOp {
                        op: Some(pb::batch_mutate_op::Op::Delete(pb::DeleteEntityRequest {
                            kind: pb::EntityKind::ReadModel as i32,
                            id: Some(id("public", &slug)),
                            mode: mode as i32,
                            ..Default::default()
                        })),
                    }],
                    ..Default::default()
                }))
                .await
                .unwrap()
                .into_inner();
            assert_eq!(
                response.status,
                if validate_only {
                    pb::batch_mutate_response::Status::Validated
                } else {
                    pb::batch_mutate_response::Status::Applied
                } as i32,
                "{response:?}"
            );
            assert_eq!(
                backend
                    .get(pb::EntityKind::ReadModel, &id("public", &slug), None)
                    .await
                    .is_ok(),
                validate_only
            );
        }
    }
    let blocked = service
        .delete_entity(Request::new(pb::DeleteEntityRequest {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id("public", "feed")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(blocked.code(), Code::FailedPrecondition);
}
