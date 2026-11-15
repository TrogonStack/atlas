#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use prost::Message as _;
use tokio::{net::TcpListener, sync::Notify};
use tonic::Status;
use trogon_atlas_core::WriterRole;
use trogon_atlas_proto::{
    self as pb,
    event_model_service_server::{EventModelService, EventModelServiceServer},
};
use trogon_atlas_server::{
    artifacts::{
        storage::{BlobStorage, NatsBlobStorage, SignedUpload},
        ArtifactService,
    },
    auth::{BearerAuth, TokenRegistry},
    mcp::data_plane,
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::{NatsStore, Store};

struct GatedStorage {
    inner: NatsBlobStorage,
    armed: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[async_trait]
impl BlobStorage for GatedStorage {
    fn backend_name(&self) -> &'static str {
        "nats"
    }

    async fn put(&self, key: &str, content: Bytes) -> Result<(), Status> {
        self.inner.put(key, content).await
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        if key.starts_with("staging/") && self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.get(key, max_bytes).await
    }

    async fn delete(&self, key: &str) -> Result<(), Status> {
        self.inner.delete(key).await
    }

    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        size: u64,
        digest: &str,
        media_type: &str,
    ) -> Result<Option<SignedUpload>, Status> {
        self.inner
            .presign_put(key, ttl, size, digest, media_type)
            .await
    }

    async fn presign_get(
        &self,
        key: &str,
        ttl: Duration,
        media_type: &str,
    ) -> Result<Option<String>, Status> {
        self.inner.presign_get(key, ttl, media_type).await
    }
}

#[derive(prost::Message)]
struct LeaseFaultRecord {
    #[prost(string, tag = "1")]
    holder_id: String,
    #[prost(uint64, tag = "2")]
    epoch: u64,
    #[prost(int64, tag = "3")]
    expires_at_micros: i64,
}

#[tokio::test]
async fn completion_cannot_publish_after_actual_writer_takeover() {
    let nats = trogon_atlas_testsupport::shared().await;
    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = nats.store_config();
    config.lease_duration = Duration::from_secs(120);
    config.lease_renew_interval = Duration::from_secs(60);
    let writer = Arc::new(NatsStore::connect_with(config.clone()).await.unwrap());
    let original_epoch = writer.writer_status().epoch;
    let blobs = Arc::new(GatedStorage {
        inner: NatsBlobStorage::open(
            js.clone(),
            async_nats::jetstream::object_store::Config {
                bucket: format!("artifact-fence-bytes-{suffix}"),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
        armed: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let catalog = js
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: format!("artifact-fence-catalog-{suffix}"),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let artifacts = Arc::new(
        ArtifactService::new(
            blobs.clone(),
            catalog,
            1024,
            Duration::from_secs(60),
            writer.artifact_publication_authority().unwrap(),
        )
        .unwrap(),
    );
    let service = EventModelServiceImpl::try_new(writer.clone(), ServerMetrics::default())
        .unwrap()
        .with_artifacts(artifacts);
    let namespace = service
        .register_namespace(tonic::Request::new(pb::RegisterNamespaceRequest {
            name: format!("artifact-fence-{suffix}"),
            parent: "default".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane(
                EventModelServiceServer::new(service),
                BearerAuth::new(TokenRegistry::from_legacy_token("artifact-fence-fixture")),
            ))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let mut client =
        trogon_atlas_client::client::connect(&endpoint, Some("artifact-fence-fixture"), 15)
            .await
            .unwrap();
    let content = b"lease-fenced artifact".to_vec();
    let upload = client
        .begin_artifact_upload(pb::BeginArtifactUploadRequest {
            namespace: namespace.clone(),
            filename: "state.bin".into(),
            media_type: "application/octet-stream".into(),
            size: content.len() as u64,
            sha256: trogon_atlas_client::artifacts::sha256_hex(&content),
        })
        .await
        .unwrap()
        .into_inner();
    client
        .upload_artifact_content(pb::UploadArtifactContentRequest {
            namespace: namespace.clone(),
            upload_id: upload.upload_id.clone(),
            content,
        })
        .await
        .unwrap();
    blobs.armed.store(true, Ordering::SeqCst);
    let mut completing_client = client.clone();
    let request = pb::CompleteArtifactUploadRequest {
        namespace: namespace.clone(),
        upload_id: upload.upload_id.clone(),
    };
    let completion =
        tokio::spawn(async move { completing_client.complete_artifact_upload(request).await });
    tokio::time::timeout(Duration::from_secs(5), blobs.entered.notified())
        .await
        .unwrap();

    // Expire the actual persisted lease so a real standby can take over while I/O is blocked.
    let lease = js.get_key_value(&config.lease_bucket).await.unwrap();
    let entry = lease.entry("writer-lease").await.unwrap().unwrap();
    let mut row = LeaseFaultRecord::decode(entry.value).unwrap();
    row.expires_at_micros = 0;
    lease
        .update("writer-lease", row.encode_to_vec().into(), entry.revision)
        .await
        .unwrap();
    config.role = WriterRole::Standby;
    let standby = NatsStore::connect_with(config).await.unwrap();
    assert!(standby.is_writer());
    assert!(standby.writer_status().epoch > original_epoch);
    assert!(
        writer.is_writer(),
        "the old process still has its genuine cached lease belief"
    );
    blobs.release.notify_one();
    let result = completion.await.unwrap();
    assert!(
        result.is_err(),
        "the previous writer must not publish Ready after another store takes over"
    );
    assert_eq!(
        client
            .get_artifact(pb::GetArtifactRequest {
                namespace,
                artifact_id: upload.upload_id,
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    server.abort();
}
