#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use http_body::Body as _;
use prost::Message as _;
use tokio::sync::{watch, Notify};
use tonic::{body::Body, Code, Status};
use tower::ServiceExt as _;
use trogon_atlas_client::client::{client_for_transport, ConnectOptions, Transport};
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
    mcp::{data_plane, DataPlane},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};
use trogon_atlas_store::Store as _;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const LARGE_BYTES: usize = 16 * 1024 * 1024;

struct ObservedStorage {
    inner: NatsBlobStorage,
    reads: AtomicUsize,
    last_limit: AtomicUsize,
    block_next: AtomicUsize,
    entered: watch::Sender<usize>,
    release: Notify,
}

#[async_trait]
impl BlobStorage for ObservedStorage {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<(), Status> {
        self.inner.put(key, data).await
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        self.last_limit.store(max_bytes, Ordering::SeqCst);
        let count = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
        self.entered.send_replace(count);
        if self
            .block_next
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
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
        sha256: &str,
        media_type: &str,
    ) -> Result<Option<SignedUpload>, Status> {
        self.inner
            .presign_put(key, ttl, size, sha256, media_type)
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

struct Fixture {
    data: DataPlane,
    namespace: String,
    large: String,
    small: String,
    storage: Arc<ObservedStorage>,
}

impl Fixture {
    async fn new() -> Self {
        let nats = trogon_atlas_testsupport::shared().await;
        let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
        let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
        let inner = NatsBlobStorage::open(
            js.clone(),
            async_nats::jetstream::object_store::Config {
                bucket: format!("download-budget-{suffix}"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (entered, _) = watch::channel(0);
        let storage = Arc::new(ObservedStorage {
            inner,
            reads: AtomicUsize::new(0),
            last_limit: AtomicUsize::new(0),
            block_next: AtomicUsize::new(0),
            entered,
            release: Notify::new(),
        });
        let catalog = js
            .create_key_value(async_nats::jetstream::kv::Config {
                bucket: format!("download-records-{suffix}"),
                ..Default::default()
            })
            .await
            .unwrap();
        let store = Arc::new(nats.store().await);
        let artifacts = ArtifactService::new(
            storage.clone(),
            catalog,
            32 * 1024 * 1024,
            Duration::from_secs(60),
            store.artifact_publication_authority().unwrap(),
        )
        .unwrap();
        let service = EventModelServiceImpl::try_new(store, ServerMetrics::default())
            .unwrap()
            .with_artifacts(Arc::new(artifacts));
        let namespace = service
            .register_namespace(tonic::Request::new(pb::RegisterNamespaceRequest {
                name: "download-budget".into(),
                parent: "download-owner".into(),
            }))
            .await
            .unwrap()
            .into_inner()
            .namespace
            .unwrap()
            .id;
        let registry = TokenRegistry::from_file_contents(
            serde_json::from_value(serde_json::json!({"principals": {
                "writer": {"role":"admin", "tokens":["download-budget-fixture"]}
            }}))
            .unwrap(),
        )
        .unwrap();
        let data = data_plane(
            EventModelServiceServer::new(service)
                .max_decoding_message_size(36 * 1024 * 1024)
                .max_encoding_message_size(36 * 1024 * 1024),
            BearerAuth::new(registry),
        );
        let mut client = client_for_transport(
            Transport::new(data.clone()),
            Some("download-budget-fixture"),
            &ConnectOptions {
                max_message_bytes: Some(36 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        let mut ids = Vec::new();
        for size in [LARGE_BYTES, 64 * 1024] {
            let content = vec![0x61; size];
            let upload = client
                .begin_artifact_upload(pb::BeginArtifactUploadRequest {
                    namespace: namespace.clone(),
                    filename: "capture.bin".into(),
                    media_type: "application/octet-stream".into(),
                    size: size as u64,
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
            ids.push(
                client
                    .complete_artifact_upload(pb::CompleteArtifactUploadRequest {
                        namespace: namespace.clone(),
                        upload_id: upload.upload_id,
                    })
                    .await
                    .unwrap()
                    .into_inner()
                    .artifact
                    .unwrap()
                    .artifact_id,
            );
        }
        storage.reads.store(0, Ordering::SeqCst);
        storage.entered.send_replace(0);
        Self {
            data,
            namespace,
            large: ids.remove(0),
            small: ids.remove(0),
            storage,
        }
    }

    fn request(&self, id: &str) -> http::Request<axum::body::Body> {
        let message = pb::GetArtifactContentRequest {
            namespace: self.namespace.clone(),
            artifact_id: id.into(),
        }
        .encode_to_vec();
        let mut frame = Vec::with_capacity(message.len() + 5);
        frame.push(0);
        frame.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
        frame.extend_from_slice(&message);
        http::Request::post(
            "/trogonatlas.api.eventmodel.v1alpha1.EventModelService/GetArtifactContent",
        )
        .header("content-type", "application/grpc")
        .header("authorization", "Bearer download-budget-fixture")
        .body(axum::body::Body::from(frame))
        .unwrap()
    }

    async fn response(&self, id: &str) -> http::Response<Body> {
        self.data
            .clone()
            .oneshot(self.request(id))
            .await
            .unwrap()
            .map(Body::new)
    }

    async fn excess_response(&self) -> http::Response<Body> {
        tokio::time::timeout(Duration::from_secs(10), self.response(&self.large))
            .await
            .expect("excess download admission must refuse rather than wait for held bodies")
    }
}

fn assert_admitted(response: &http::Response<Body>) {
    assert!(Status::from_header_map(response.headers()).is_none());
}

fn assert_exhausted(response: &http::Response<Body>) {
    assert_eq!(
        Status::from_header_map(response.headers()).map(|status| status.code()),
        Some(Code::ResourceExhausted)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpolled_downloads_bound_buffering_before_another_provider_read() {
    let _serial = SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let sibling = Fixture::new().await;
    let first = fixture.response(&fixture.large).await;
    let second = sibling.response(&sibling.large).await;
    assert_admitted(&first);
    assert_admitted(&second);
    let third = fixture.excess_response().await;
    assert_exhausted(&third);
    assert_eq!(fixture.storage.reads.load(Ordering::SeqCst), 1);
    assert_eq!(sibling.storage.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.storage.last_limit.load(Ordering::SeqCst),
        LARGE_BYTES
    );
    let small = fixture.response(&fixture.small).await;
    assert_admitted(&small);
    assert_eq!(fixture.storage.last_limit.load(Ordering::SeqCst), 64 * 1024);
    drop((first, second, small));
    assert_admitted(&fixture.response(&fixture.large).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn held_transmission_frame_slices_keep_capacity_until_their_last_drop() {
    let _serial = SERIAL.lock().await;
    let fixture = Fixture::new().await;
    let mut first = fixture.response(&fixture.large).await;
    let second = fixture.response(&fixture.large).await;
    assert_admitted(&first);
    assert_admitted(&second);
    let frame = std::future::poll_fn(|cx| Pin::new(first.body_mut()).poll_frame(cx))
        .await
        .unwrap()
        .unwrap();
    let data = frame.into_data().unwrap();
    assert!(data.len() >= LARGE_BYTES);
    let retained = data.slice(..1);
    drop((data, first));
    let third = fixture.excess_response().await;
    assert_exhausted(&third);
    drop(retained);
    assert_admitted(&fixture.response(&fixture.large).await);
    drop(second);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_blocked_provider_read_releases_its_reserved_capacity() {
    let _serial = SERIAL.lock().await;
    let fixture = Fixture::new().await;
    fixture.storage.block_next.store(2, Ordering::SeqCst);
    let first = tokio::spawn(
        fixture
            .data
            .clone()
            .oneshot(fixture.request(&fixture.large)),
    );
    let second = tokio::spawn(
        fixture
            .data
            .clone()
            .oneshot(fixture.request(&fixture.large)),
    );
    fixture
        .storage
        .entered
        .subscribe()
        .wait_for(|count| *count >= 2)
        .await
        .unwrap();
    let third =
        tokio::time::timeout(Duration::from_secs(10), fixture.response(&fixture.large)).await;
    let rejected = third
        .as_ref()
        .ok()
        .and_then(|response| Status::from_header_map(response.headers()))
        .map(|status| status.code());
    let reads = fixture.storage.reads.load(Ordering::SeqCst);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let replacement = fixture.response(&fixture.large).await;
    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    assert!(
        third.is_ok(),
        "excess admission must not wait for provider capacity"
    );
    assert_eq!(rejected, Some(Code::ResourceExhausted));
    assert_eq!(reads, 2);
    assert_admitted(&replacement);
}
