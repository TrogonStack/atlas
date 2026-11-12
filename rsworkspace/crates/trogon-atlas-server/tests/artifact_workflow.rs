#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use tokio::net::TcpListener;
use trogon_atlas_proto::{
    self as pb,
    event_model_service_server::{EventModelService, EventModelServiceServer},
};
use trogon_atlas_server::{
    artifacts::{storage::NatsBlobStorage, ArtifactService},
    auth::{BearerAuth, LiveRegistry, TokenRegistry},
    mcp::{data_plane, DataPlane},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::Store as _;

async fn fixture() -> (String, String, tokio::task::JoinHandle<()>, DataPlane) {
    let (endpoint, namespace, task, data, _) = fixture_with_registry().await;
    (endpoint, namespace, task, data)
}

fn registry(restrict_writer: bool) -> TokenRegistry {
    let namespaces = if restrict_writer {
        vec!["artifact-allowed-only"]
    } else {
        Vec::new()
    };
    TokenRegistry::from_file_contents(serde_json::from_value(serde_json::json!({"principals":{
        "writer":{"role":"writer","parent":"artifact-owner","namespaces":namespaces,"tokens":["artifact-writer-fixture"]},
        "reader":{"role":"reader","parent":"artifact-owner","tokens":["artifact-reader-fixture"]},
        "hidden":{"role":"reader","parent":"different-owner","tokens":["artifact-hidden-fixture"]},
        "foreign-writer":{"role":"writer","parent":"different-owner","tokens":["artifact-foreign-writer-fixture"]},
        "agent-writer":{"kind":"agent","role":"writer","parent":"artifact-owner","tokens":["artifact-agent-writer-fixture"]},
        "unrestricted":{"role":"writer","tokens":["artifact-unrestricted-fixture"]}
    }})).unwrap()).unwrap()
}

async fn fixture_with_registry() -> (
    String,
    String,
    tokio::task::JoinHandle<()>,
    DataPlane,
    LiveRegistry,
) {
    let nats = trogon_atlas_testsupport::shared().await;
    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let blobs = NatsBlobStorage::open(
        js.clone(),
        async_nats::jetstream::object_store::Config {
            bucket: format!("artifact-bytes-{suffix}"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let catalog = js
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: format!("artifact-records-{suffix}"),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let store = nats.store().await;
    let artifacts = Arc::new(
        ArtifactService::new(
            Arc::new(blobs),
            catalog,
            1024 * 1024,
            Duration::from_secs(60),
            store.artifact_publication_authority().unwrap(),
        )
        .unwrap(),
    );
    let service = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default())
        .unwrap()
        .with_artifacts(artifacts);
    let namespace = service
        .register_namespace(tonic::Request::new(pb::RegisterNamespaceRequest {
            name: "artifact-fixture".into(),
            parent: "artifact-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let auth = BearerAuth::new(registry(false));
    let live = auth.live();
    let data = data_plane(
        EventModelServiceServer::new(service)
            .max_decoding_message_size(2 * 1024 * 1024)
            .max_encoding_message_size(2 * 1024 * 1024),
        auth,
    );
    let mut routes = tonic::service::Routes::builder();
    routes.add_service(data.clone());
    let routes = routes.routes();
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .accept_http1(true)
            .add_routes(routes)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (endpoint, namespace, task, data, live)
}

async fn client(endpoint: &str, token: &str) -> trogon_atlas_client::client::Client {
    trogon_atlas_client::client::connect(endpoint, Some(token), 15)
        .await
        .unwrap()
}

fn begin(namespace: &str, content: &[u8]) -> pb::BeginArtifactUploadRequest {
    pb::BeginArtifactUploadRequest {
        namespace: namespace.into(),
        filename: "state.png".into(),
        media_type: "image/png".into(),
        size: content.len() as u64,
        sha256: trogon_atlas_client::artifacts::sha256_hex(content),
    }
}

fn event(namespace: &str) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        entity: Some(pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: namespace.into(),
                    slug: "claimed".into(),
                    version: 1,
                }),
                title: "Claimed".into(),
                ..Default::default()
            })),
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn artifact_unrestricted_upload_reserves_unclaimed_namespace_permanently() {
    let (endpoint, _, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-unrestricted-fixture").await;
    let mut foreign = client(&endpoint, "artifact-foreign-writer-fixture").await;
    let namespace = format!(
        "artifact-unclaimed-{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    );
    let content = b"reserved artifact".to_vec();
    let upload = writer
        .begin_artifact_upload(begin(&namespace, &content))
        .await
        .unwrap()
        .into_inner();
    let listed = writer
        .list_namespaces(pb::ListNamespacesRequest {})
        .await
        .unwrap()
        .into_inner();
    let reserved = listed
        .namespaces
        .iter()
        .find(|record| record.id == namespace)
        .expect("accepting an upload must reserve even an unrestricted writer's namespace");
    assert_eq!(reserved.parent, "default");
    writer
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id.clone(),
            content: content.clone(),
        })
        .await
        .unwrap();
    let artifact = writer
        .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id,
        })
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    assert_eq!(
        foreign
            .put_entity(event(&namespace))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        foreign
            .get_artifact_content(pb::GetArtifactContentRequest {
                namespace: namespace.clone(),
                artifact_id: artifact.artifact_id.clone(),
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    assert_eq!(
        writer
            .get_artifact_content(pb::GetArtifactContentRequest {
                namespace,
                artifact_id: artifact.artifact_id,
            })
            .await
            .unwrap()
            .into_inner()
            .content,
        content
    );
    task.abort();
}

#[tokio::test]
async fn artifact_bound_writer_can_reserve_a_new_namespace() {
    let (endpoint, _, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let namespace = format!(
        "artifact-owned-{}",
        trogon_atlas_testsupport::unique_suffix_for_test()
    );
    writer
        .begin_artifact_upload(begin(&namespace, b"new namespace"))
        .await
        .expect("a bound writer may reserve an unclaimed namespace for its upload");
    let listed = writer
        .list_namespaces(pb::ListNamespacesRequest {})
        .await
        .unwrap()
        .into_inner();
    let record = listed
        .namespaces
        .iter()
        .find(|record| record.id == namespace)
        .expect("the accepted upload's namespace must be registered");
    assert_eq!(record.parent, "artifact-owner");
    task.abort();
}

#[tokio::test]
async fn artifact_retention_preserves_branch_namespace_ownership_after_deletion() {
    let (endpoint, _, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-agent-writer-fixture").await;
    let mut foreign = client(&endpoint, "artifact-foreign-writer-fixture").await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let namespace = format!("artifact-branch-{suffix}");
    let branch = format!("@artifact-owner:artifact-branch-{suffix}");
    writer
        .create_branch(pb::CreateBranchRequest {
            name: branch.clone(),
            doc: "Artifact namespace retention".into(),
        })
        .await
        .unwrap();
    let mut branch_write = tonic::Request::new(event(&namespace));
    branch_write
        .metadata_mut()
        .insert("x-trogon-atlas-branch", branch.parse().unwrap());
    writer.put_entity(branch_write).await.unwrap();
    assert_eq!(
        writer
            .get_entity(pb::GetEntityRequest {
                kind: pb::EntityKind::Event.into(),
                id: Some(pb::Id {
                    namespace: namespace.clone(),
                    slug: "claimed".into(),
                    version: 1,
                }),
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    let content = b"retained after branch deletion".to_vec();
    let upload = writer
        .begin_artifact_upload(begin(&namespace, &content))
        .await
        .unwrap()
        .into_inner();
    writer
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id.clone(),
            content: content.clone(),
        })
        .await
        .unwrap();
    let artifact = writer
        .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id,
        })
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    writer
        .delete_branch(pb::DeleteBranchRequest { name: branch })
        .await
        .unwrap();
    assert_eq!(
        writer
            .get_artifact_content(pb::GetArtifactContentRequest {
                namespace: namespace.clone(),
                artifact_id: artifact.artifact_id.clone(),
            })
            .await
            .expect("branch deletion must retain the artifact owner's namespace")
            .into_inner()
            .content,
        content
    );
    assert_eq!(
        foreign
            .put_entity(event(&namespace))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        foreign
            .get_artifact_content(pb::GetArtifactContentRequest {
                namespace,
                artifact_id: artifact.artifact_id,
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    task.abort();
}

#[tokio::test]
async fn artifact_writer_namespace_scope_is_rechecked_for_valid_upload_sessions() {
    let (endpoint, namespace, task, _, live) = fixture_with_registry().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let content = b"valid scoped session".to_vec();
    let upload = writer
        .begin_artifact_upload(begin(&namespace, &content))
        .await
        .unwrap()
        .into_inner();
    live.install(Arc::new(registry(true)));
    assert_eq!(
        writer
            .begin_artifact_upload(begin(&namespace, &content))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let transfer = pb::UploadArtifactContentRequest {
        namespace: namespace.clone(),
        upload_id: upload.upload_id.clone(),
        content: content.clone(),
    };
    assert_eq!(
        writer
            .upload_artifact_content(transfer.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    live.install(Arc::new(registry(false)));
    writer.upload_artifact_content(transfer).await.unwrap();
    live.install(Arc::new(registry(true)));
    let completion = pb::CompleteArtifactUploadRequest {
        namespace: namespace.clone(),
        upload_id: upload.upload_id,
    };
    assert_eq!(
        writer
            .complete_artifact_upload(completion.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    live.install(Arc::new(registry(false)));
    let artifact = writer
        .complete_artifact_upload(completion)
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    live.install(Arc::new(registry(true)));
    assert_eq!(
        writer
            .get_artifact_content(pb::GetArtifactContentRequest {
                namespace,
                artifact_id: artifact.artifact_id,
            })
            .await
            .unwrap()
            .into_inner()
            .content,
        content
    );
    task.abort();
}

#[tokio::test]
async fn nats_begin_uses_grpc_without_http_upload_target() {
    let (endpoint, namespace, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let upload = writer
        .begin_artifact_upload(begin(&namespace, b"grpc upload"))
        .await
        .unwrap()
        .into_inner();
    assert!(!upload.direct_upload);
    assert!(
        upload.upload_url.is_empty(),
        "NATS uploads must use UploadArtifactContent RPC without an HTTP upload URL"
    );
    assert!(upload.headers.is_empty());
    task.abort();
}

#[tokio::test]
async fn nats_download_requires_get_artifact_content_rpc() {
    let (endpoint, namespace, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let content = b"grpc download".to_vec();
    let upload = writer
        .begin_artifact_upload(begin(&namespace, &content))
        .await
        .unwrap()
        .into_inner();
    writer
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id.clone(),
            content: content.clone(),
        })
        .await
        .unwrap();
    let artifact = writer
        .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id,
        })
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    let result = writer
        .get_artifact_download(pb::GetArtifactDownloadRequest {
            namespace: namespace.clone(),
            artifact_id: artifact.artifact_id.clone(),
        })
        .await;
    assert!(
        result.is_err(),
        "NATS downloads must not advertise an HTTP artifact URL"
    );
    let error = result.unwrap_err();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(error.message().contains("GetArtifactContent"));
    assert_eq!(
        writer
            .get_artifact_content(pb::GetArtifactContentRequest {
                namespace,
                artifact_id: artifact.artifact_id,
            })
            .await
            .unwrap()
            .into_inner()
            .content,
        content
    );
    task.abort();
}

#[tokio::test]
async fn unauthorized_rpc_uploads_are_rejected_without_reading_the_body() {
    use tower::ServiceExt;
    let (_, _, task, data_plane) = fixture().await;
    for (token, expected) in [
        (None, tonic::Code::Unauthenticated),
        (Some("invalid-fixture-token"), tonic::Code::Unauthenticated),
        (
            Some("artifact-reader-fixture"),
            tonic::Code::PermissionDenied,
        ),
    ] {
        let body = axum::body::Body::from_stream(futures::stream::pending::<
            Result<Bytes, std::io::Error>,
        >());
        let mut request = axum::http::Request::post(
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/UploadArtifactContent",
        )
        .header("content-type", "application/grpc");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            data_plane.clone().oneshot(request.body(body).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        let status = tonic::Status::from_header_map(response.headers()).unwrap();
        assert_eq!(status.code(), expected);
    }
    task.abort();
}

#[tokio::test]
async fn authenticated_rpc_upload_finalize_and_content_preserve_exact_bytes_and_namespace() {
    let (endpoint, namespace, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let mut reader = client(&endpoint, "artifact-reader-fixture").await;
    let mut hidden = client(&endpoint, "artifact-hidden-fixture").await;
    let content = vec![0xab; 300 * 1024];
    assert_eq!(
        reader
            .begin_artifact_upload(begin(&namespace, &content))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let upload = writer
        .begin_artifact_upload(begin(&namespace, &content))
        .await
        .unwrap()
        .into_inner();
    assert!(!upload.direct_upload);
    assert_eq!(
        writer
            .get_artifact(pb::GetArtifactRequest {
                namespace: namespace.clone(),
                artifact_id: upload.upload_id.clone()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    assert!(
        upload.upload_url.is_empty(),
        "NATS RPC uploads must not advertise an HTTP target"
    );
    assert!(upload.headers.is_empty());
    let transfer = pb::UploadArtifactContentRequest {
        namespace: namespace.clone(),
        upload_id: upload.upload_id.clone(),
        content: content.clone(),
    };
    assert_eq!(
        reader
            .upload_artifact_content(transfer.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    writer.upload_artifact_content(transfer).await.unwrap();
    let request = pb::CompleteArtifactUploadRequest {
        namespace: namespace.clone(),
        upload_id: upload.upload_id.clone(),
    };
    let artifact = writer
        .complete_artifact_upload(request.clone())
        .await
        .unwrap()
        .into_inner()
        .artifact
        .unwrap();
    assert_eq!(
        artifact,
        writer
            .complete_artifact_upload(request)
            .await
            .unwrap()
            .into_inner()
            .artifact
            .unwrap()
    );
    assert_eq!(
        hidden
            .get_artifact(pb::GetArtifactRequest {
                namespace: namespace.clone(),
                artifact_id: artifact.artifact_id.clone()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    let get_content = pb::GetArtifactContentRequest {
        namespace: namespace.clone(),
        artifact_id: artifact.artifact_id.clone(),
    };
    assert_eq!(
        hidden
            .get_artifact_content(get_content.clone())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    let response = reader
        .get_artifact_content(get_content)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.artifact.unwrap(), artifact);
    assert_eq!(response.content, content);
    assert_eq!(
        writer
            .upload_artifact_content(pb::UploadArtifactContentRequest {
                namespace,
                upload_id: upload.upload_id,
                content: vec![1u8; content.len()],
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    task.abort();
}

#[tokio::test]
async fn digest_mismatch_and_oversized_rpc_content_never_publish_an_artifact() {
    let (endpoint, namespace, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let upload = writer
        .begin_artifact_upload(begin(&namespace, b"expected"))
        .await
        .unwrap()
        .into_inner();
    for content in [b"modified".to_vec(), vec![0u8; 1024 * 1024 + 1]] {
        assert_eq!(
            writer
                .upload_artifact_content(pb::UploadArtifactContentRequest {
                    namespace: namespace.clone(),
                    upload_id: upload.upload_id.clone(),
                    content,
                })
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    assert_eq!(
        writer
            .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
                namespace: namespace.clone(),
                upload_id: upload.upload_id.clone()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    assert_eq!(
        writer
            .get_artifact(pb::GetArtifactRequest {
                namespace,
                artifact_id: upload.upload_id
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    task.abort();
}

#[tokio::test]
async fn upload_record_survives_service_restart_and_final_content_ignores_staging_changes() {
    use trogon_atlas_server::artifacts::storage::BlobStorage;
    let nats = trogon_atlas_testsupport::shared().await;
    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let blobs = Arc::new(
        NatsBlobStorage::open(
            js.clone(),
            async_nats::jetstream::object_store::Config {
                bucket: format!("artifact-stable-{suffix}"),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
    );
    let catalog = js
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: format!("artifact-stable-catalog-{suffix}"),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let store = nats.store().await;
    let make = || {
        ArtifactService::new(
            blobs.clone(),
            catalog.clone(),
            1024,
            Duration::from_secs(60),
            store.artifact_publication_authority().unwrap(),
        )
        .unwrap()
    };
    let service = make();
    let upload = service
        .begin(begin("sample", b"original"), "writer")
        .await
        .unwrap();
    service
        .upload(
            "sample",
            &upload.upload_id,
            "writer",
            Bytes::from_static(b"original"),
        )
        .await
        .unwrap();
    drop(service);
    let reopened = make();
    let artifact = reopened
        .complete("sample", &upload.upload_id, "writer")
        .await
        .unwrap();
    blobs
        .put(
            &format!("staging/{}", upload.upload_id),
            Bytes::from_static(b"tampered"),
        )
        .await
        .unwrap();
    assert_eq!(
        reopened
            .content("sample", &artifact.artifact_id)
            .await
            .unwrap()
            .content,
        b"original"
    );
    assert_eq!(
        reopened
            .complete("sample", &upload.upload_id, "writer")
            .await
            .unwrap(),
        artifact
    );
    assert_eq!(
        reopened
            .complete("sample", &upload.upload_id, "different-writer")
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    assert_eq!(
        reopened
            .get("other", &artifact.artifact_id)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
}
