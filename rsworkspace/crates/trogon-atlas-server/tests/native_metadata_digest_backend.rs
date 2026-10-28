#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pb::event_model_service_server::EventModelService;
use tonic::{metadata::MetadataValue, Code, Request};
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

const NOTE: &str = "type.googleapis.com/catalog.notes.Note";

fn id(namespace: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: namespace.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn library(namespace: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::TypeLibrary(pb::TypeLibrary {
            id: Some(id(namespace, "catalog.notes")),
            title: "Notes".into(),
            files: vec![pb::ProtoSourceFile {
                path: "catalog/notes/v1/note.proto".into(),
                content: "syntax = \"proto3\";\npackage catalog.notes;\nmessage Note { string text = 1; }\n"
                    .into(),
            }],
            provenance: Some(pb::TypeLibraryProvenance {
                source: Some(pb::type_library_provenance::Source::Git(
                    pb::type_library_provenance::GitSnapshot {
                        repository_url: "https://example.com/acme/notes.git".into(),
                        r#ref: "main".into(),
                        commit: "0123456789abcdef".into(),
                        subdirectory: "proto".into(),
                    },
                )),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn note(text: &str) -> prost_types::Any {
    let mut value = Vec::new();
    prost::encoding::string::encode(1, &text.to_owned(), &mut value);
    prost_types::Any {
        type_url: NOTE.into(),
        value,
    }
}

fn component(namespace: &str, slug: &str, text: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::Component(pb::Component {
            id: Some(id(namespace, slug)),
            title: slug.into(),
            metadata: vec![note(text)],
            ..Default::default()
        })),
        ..Default::default()
    }
}

fn on_branch<T>(body: T, branch: Option<&str>) -> Request<T> {
    let mut request = Request::new(body);
    if let Some(branch) = branch {
        request.metadata_mut().insert(
            "x-trogon-atlas-branch",
            MetadataValue::try_from(branch).unwrap(),
        );
    }
    request
}

async fn service() -> EventModelServiceImpl {
    EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap()
    .with_snapshot_cap(0)
}

async fn branch(service: &EventModelServiceImpl, name: &str) {
    service
        .create_branch(Request::new(pb::CreateBranchRequest {
            name: name.into(),
            ..Default::default()
        }))
        .await
        .unwrap();
}

fn batch(entity: pb::Entity, operation_id: &str) -> pb::BatchMutateRequest {
    pb::BatchMutateRequest {
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                entity: Some(entity),
                ..Default::default()
            })),
        }],
        operation_id: operation_id.into(),
        ..Default::default()
    }
}

async fn exercise(branch_name: Option<&str>) {
    let service = service().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let namespace = format!("digest-{suffix}");
    if let Some(name) = branch_name {
        branch(&service, name).await;
    }
    service
        .put_entity(on_branch(
            pb::PutEntityRequest {
                entity: Some(library(&namespace)),
                ..Default::default()
            },
            branch_name,
        ))
        .await
        .unwrap();

    let put = |slug: &str, text: &str, operation_id: &str| {
        on_branch(
            pb::PutEntityRequest {
                entity: Some(component(&namespace, slug, text)),
                operation_id: operation_id.into(),
                ..Default::default()
            },
            branch_name,
        )
    };
    let first = service
        .put_entity(put("single", "alpha", &format!("put-{suffix}")))
        .await
        .unwrap()
        .into_inner();
    let replay = service
        .put_entity(put("single", "alpha", &format!("put-{suffix}")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.entity, replay.entity);
    assert_eq!(
        service
            .put_entity(put("single", "beta", &format!("put-{suffix}")))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );

    let op = format!("batch-{suffix}");
    let applied = service
        .batch_mutate(on_branch(
            batch(component(&namespace, "batched", "alpha"), &op),
            branch_name,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        applied.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{applied:?}"
    );
    let replayed = service
        .batch_mutate(on_branch(
            batch(component(&namespace, "batched", "alpha"), &op),
            branch_name,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        replayed.operation_receipt.as_ref().map(|r| r.replayed),
        Some(true),
        "{replayed:?}"
    );
    let reused = service
        .batch_mutate(on_branch(
            batch(component(&namespace, "batched", "beta"), &op),
            branch_name,
        ))
        .await
        .unwrap_err();
    assert_eq!(reused.code(), Code::AlreadyExists, "{reused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn tenant_typed_metadata_has_a_stable_operation_digest() {
    exercise(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tenant_typed_metadata_has_a_stable_operation_digest_on_a_branch() {
    exercise(Some("digest-branch")).await;
}

fn batch_of(entities: Vec<pb::Entity>, operation_id: &str) -> pb::BatchMutateRequest {
    pb::BatchMutateRequest {
        ops: entities
            .into_iter()
            .map(|entity| pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(entity),
                    ..Default::default()
                })),
            })
            .collect(),
        operation_id: operation_id.into(),
        ..Default::default()
    }
}

async fn component_exists(
    service: &EventModelServiceImpl,
    namespace: &str,
    slug: &str,
    branch_name: Option<&str>,
) -> bool {
    service
        .get_entity(on_branch(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Component as i32,
                id: Some(id(namespace, slug)),
            },
            branch_name,
        ))
        .await
        .is_ok()
}

async fn same_batch_library(branch_name: Option<&str>) {
    let service = service().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let namespace = format!("same-batch-{suffix}");
    if let Some(name) = branch_name {
        branch(&service, &format!("{name}-{suffix}")).await;
    }
    let branch_name = branch_name.map(|name| format!("{name}-{suffix}"));
    let branch_name = branch_name.as_deref();

    let plain = service
        .batch_mutate(on_branch(
            batch_of(
                vec![library(&namespace), component(&namespace, "plain", "alpha")],
                "",
            ),
            branch_name,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        plain.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "the same operation without an operation_id is the public contract: {plain:?}"
    );

    let other = format!("same-batch-other-{suffix}");
    let op = format!("same-batch-{suffix}");
    let entities = |text: &str| {
        vec![
            library(&other),
            component(&other, "keyed", text),
            component(&other, "keyed-too", text),
        ]
    };
    let applied = service
        .batch_mutate(on_branch(batch_of(entities("alpha"), &op), branch_name))
        .await
        .unwrap_or_else(|status| panic!("same-batch tenant metadata must digest: {status}"))
        .into_inner();
    assert_eq!(
        applied.status,
        pb::batch_mutate_response::Status::Applied as i32,
        "{applied:?}"
    );
    assert!(component_exists(&service, &other, "keyed", branch_name).await);
    assert!(component_exists(&service, &other, "keyed-too", branch_name).await);

    let replayed = service
        .batch_mutate(on_branch(batch_of(entities("alpha"), &op), branch_name))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        replayed.operation_receipt.as_ref().map(|r| r.replayed),
        Some(true),
        "{replayed:?}"
    );
    let reused = service
        .batch_mutate(on_branch(batch_of(entities("beta"), &op), branch_name))
        .await
        .unwrap_err();
    assert_eq!(reused.code(), Code::AlreadyExists, "{reused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_creating_a_type_library_digests_tenant_metadata_written_with_it() {
    same_batch_library(None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_creating_a_type_library_digests_tenant_metadata_written_with_it_on_a_branch() {
    same_batch_library(Some("same-batch")).await;
}
