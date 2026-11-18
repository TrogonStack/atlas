#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use tonic::{Code, Status};
use trogon_atlas_proto as pb;
use trogon_atlas_server::artifacts::{
    storage::{BlobStorage, NatsBlobStorage, SignedUpload},
    ArtifactService,
};
use trogon_atlas_store::Store as _;

struct InterruptedStagingCleanup {
    inner: NatsBlobStorage,
    fail_staging_delete: AtomicBool,
    staging_deletes: AtomicUsize,
}

#[async_trait]
impl BlobStorage for InterruptedStagingCleanup {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    async fn put(&self, key: &str, content: Bytes) -> Result<(), Status> {
        self.inner.put(key, content).await
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        self.inner.get(key, max_bytes).await
    }

    async fn delete(&self, key: &str) -> Result<(), Status> {
        if key.starts_with("staging/") {
            self.staging_deletes.fetch_add(1, Ordering::SeqCst);
            if self.fail_staging_delete.swap(false, Ordering::SeqCst) {
                return Err(Status::unavailable("injected staging cleanup interruption"));
            }
        }
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

async fn fixture() -> (
    ArtifactService,
    Arc<InterruptedStagingCleanup>,
    async_nats::jetstream::kv::Store,
    Arc<trogon_atlas_store::NatsStore>,
) {
    let nats = trogon_atlas_testsupport::shared().await;
    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let storage = Arc::new(InterruptedStagingCleanup {
        inner: NatsBlobStorage::open(
            js.clone(),
            async_nats::jetstream::object_store::Config {
                bucket: format!("artifact-cleanup-bytes-{suffix}"),
                ..Default::default()
            },
        )
        .await
        .unwrap(),
        fail_staging_delete: AtomicBool::new(true),
        staging_deletes: AtomicUsize::new(0),
    });
    let catalog = js
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: format!("artifact-cleanup-records-{suffix}"),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let store = Arc::new(nats.store().await);
    let service = ArtifactService::new(
        storage.clone(),
        catalog.clone(),
        1024,
        Duration::from_secs(60),
        store.artifact_publication_authority().unwrap(),
    )
    .unwrap();
    (service, storage, catalog, store)
}

#[tokio::test]
async fn accepted_receipt_retry_reclaims_nats_staging_after_ready() {
    accepted_receipt_retry_reclaims_staging(false).await;
}

#[tokio::test]
async fn accepted_receipt_retry_reclaims_nats_staging_after_interrupted_materialization() {
    accepted_receipt_retry_reclaims_staging(true).await;
}

#[tokio::test]
async fn accepted_receipt_cleanup_cannot_redirect_to_confirmed_final_content() {
    let (service, storage, catalog, _store) = fixture().await;
    let content = Bytes::from_static(b"accepted upload");
    let upload = service
        .begin(
            pb::BeginArtifactUploadRequest {
                namespace: "sample".into(),
                filename: "state.bin".into(),
                media_type: "application/octet-stream".into(),
                size: content.len() as u64,
                sha256: trogon_atlas_client::artifacts::sha256_hex(&content),
            },
            "writer",
        )
        .await
        .unwrap();
    let id = upload.upload_id;
    let staging_key = format!("staging/{id}");
    service
        .upload("sample", &id, "writer", content.clone())
        .await
        .unwrap();
    let accepted = service.complete("sample", &id, "writer").await.unwrap();
    assert_eq!(storage.get(&staging_key, 1024).await.unwrap(), content);

    let mut row: serde_json::Value =
        serde_json::from_slice(&catalog.get(&id).await.unwrap().unwrap()).unwrap();
    row["staging_key"] = row["final_key"].clone();
    catalog
        .put(&id, serde_json::to_vec(&row).unwrap().into())
        .await
        .unwrap();
    assert_eq!(service.get("sample", &id).await.unwrap(), accepted);
    assert_eq!(
        service.complete("sample", &id, "writer").await.unwrap(),
        accepted
    );
    assert_eq!(
        service
            .content("sample", &id)
            .await
            .expect("accepted-receipt cleanup must never delete confirmed final bytes")
            .content,
        content
    );
    assert_eq!(
        storage.get(&staging_key, 1024).await.unwrap_err().code(),
        Code::NotFound,
        "recovery still cleans up the canonical staging object"
    );
}

async fn accepted_receipt_retry_reclaims_staging(interrupted_materialization: bool) {
    let (service, storage, catalog, _store) = fixture().await;
    let content = Bytes::from_static(b"accepted upload");
    let upload = service
        .begin(
            pb::BeginArtifactUploadRequest {
                namespace: "sample".into(),
                filename: "state.bin".into(),
                media_type: "application/octet-stream".into(),
                size: content.len() as u64,
                sha256: trogon_atlas_client::artifacts::sha256_hex(&content),
            },
            "writer",
        )
        .await
        .unwrap();
    let id = upload.upload_id;
    let staging_key = format!("staging/{id}");
    service
        .upload("sample", &id, "writer", content.clone())
        .await
        .unwrap();
    let accepted = service.complete("sample", &id, "writer").await.unwrap();
    assert_eq!(storage.staging_deletes.load(Ordering::SeqCst), 1);
    assert_eq!(storage.get(&staging_key, 1024).await.unwrap(), content);

    if interrupted_materialization {
        let mut row: serde_json::Value =
            serde_json::from_slice(&catalog.get(&id).await.unwrap().unwrap()).unwrap();
        row["phase"] = "Verifying".into();
        row["expires"] = 0.into();
        catalog
            .put(&id, serde_json::to_vec(&row).unwrap().into())
            .await
            .unwrap();
    }
    assert_eq!(
        service
            .complete("sample", &id, "another-writer")
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(storage.staging_deletes.load(Ordering::SeqCst), 1);

    storage.fail_staging_delete.store(true, Ordering::SeqCst);
    assert_eq!(
        service.complete("sample", &id, "writer").await.unwrap(),
        accepted
    );
    assert_eq!(
        storage.staging_deletes.load(Ordering::SeqCst),
        2,
        "accepted-receipt recovery must retry best-effort staging cleanup"
    );
    assert_eq!(storage.get(&staging_key, 1024).await.unwrap(), content);
    assert_eq!(service.get("sample", &id).await.unwrap(), accepted);
    assert_eq!(
        service.content("sample", &id).await.unwrap().content,
        content
    );

    assert_eq!(
        service.complete("sample", &id, "writer").await.unwrap(),
        accepted
    );
    assert_eq!(storage.staging_deletes.load(Ordering::SeqCst), 3);
    assert_eq!(
        storage.get(&staging_key, 1024).await.unwrap_err().code(),
        Code::NotFound
    );
    assert_eq!(
        service.content("sample", &id).await.unwrap().content,
        content
    );
    assert_eq!(
        service.complete("sample", &id, "writer").await.unwrap(),
        accepted
    );
    assert_eq!(storage.staging_deletes.load(Ordering::SeqCst), 4);
    assert_eq!(
        service.content("sample", &id).await.unwrap().content,
        content
    );
}
