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
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

async fn fixture() -> (String, String, tokio::task::JoinHandle<()>, axum::Router) {
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
    let artifacts = Arc::new(
        ArtifactService::new(
            Arc::new(blobs),
            catalog,
            endpoint.clone(),
            1024 * 1024,
            Duration::from_secs(60),
        )
        .unwrap(),
    );
    let store = nats.store().await;
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
    let registry = TokenRegistry::from_file_contents(serde_json::from_value(serde_json::json!({"principals":{
        "writer":{"role":"writer","parent":"artifact-owner","tokens":["artifact-writer-fixture"]},
        "reader":{"role":"reader","parent":"artifact-owner","tokens":["artifact-reader-fixture"]},
        "hidden":{"role":"reader","parent":"different-owner","tokens":["artifact-hidden-fixture"]}
    }})).unwrap()).unwrap();
    let data = data_plane(
        EventModelServiceServer::new(service)
            .max_decoding_message_size(2 * 1024 * 1024)
            .max_encoding_message_size(2 * 1024 * 1024),
        BearerAuth::new(registry),
    );
    let mut routes = tonic::service::Routes::builder();
    routes.add_service(data.clone());
    let mut routes = routes.routes();
    let http = trogon_atlas_server::artifacts::http::router(data, 1024 * 1024);
    *routes.axum_router_mut() = routes.axum_router_mut().clone().merge(http.clone());
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .accept_http1(true)
            .add_routes(routes)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (endpoint, namespace, task, http)
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

#[tokio::test]
async fn unauthorized_http_uploads_are_rejected_without_reading_the_body() {
    use tower::ServiceExt;
    let (_, namespace, task, router) = fixture().await;
    for (token, expected) in [
        (None, 401),
        (Some("invalid-fixture-token"), 401),
        (Some("artifact-reader-fixture"), 403),
    ] {
        let body = axum::body::Body::from_stream(futures::stream::pending::<
            Result<Bytes, std::io::Error>,
        >());
        let mut request = axum::http::Request::put(format!(
            "/api/artifact-uploads/{namespace}/{}",
            uuid::Uuid::now_v7()
        ));
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            router.clone().oneshot(request.body(body).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status().as_u16(), expected);
    }
    task.abort();
}

#[tokio::test]
async fn authenticated_http_upload_finalize_and_download_preserve_exact_bytes_and_namespace() {
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
    let http = reqwest::Client::new();
    assert_eq!(
        http.put(&upload.upload_url)
            .body(content.clone())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.put(&upload.upload_url)
            .bearer_auth("artifact-reader-fixture")
            .body(content.clone())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.put(&upload.upload_url)
            .bearer_auth("artifact-writer-fixture")
            .body(content.clone())
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
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
    assert_eq!(
        http.get(&artifact.content_url)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.get(&artifact.content_url)
            .bearer_auth("artifact-hidden-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let response = http
        .get(&artifact.content_url)
        .bearer_auth("artifact-reader-fixture")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await.unwrap(), content);
    assert_eq!(
        http.put(&upload.upload_url)
            .bearer_auth("artifact-writer-fixture")
            .body(vec![1u8; content.len()])
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    task.abort();
}

#[tokio::test]
async fn digest_mismatch_and_oversized_http_body_never_publish_an_artifact() {
    let (endpoint, namespace, task, _) = fixture().await;
    let mut writer = client(&endpoint, "artifact-writer-fixture").await;
    let upload = writer
        .begin_artifact_upload(begin(&namespace, b"expected"))
        .await
        .unwrap()
        .into_inner();
    let http = reqwest::Client::new();
    assert_eq!(
        http.put(&upload.upload_url)
            .bearer_auth("artifact-writer-fixture")
            .body("modified")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        http.put(&upload.upload_url)
            .bearer_auth("artifact-writer-fixture")
            .body(vec![0u8; 1024 * 1024 + 1])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
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
    let make = || {
        ArtifactService::new(
            blobs.clone(),
            catalog.clone(),
            "http://localhost:50069".into(),
            1024,
            Duration::from_secs(60),
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
