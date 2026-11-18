#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use tonic::{Code, Status};
use trogon_atlas_proto as pb;
use trogon_atlas_server::artifacts::{
    storage::{BlobStorage, SignedUpload},
    ArtifactService,
};
use trogon_atlas_store::Store as _;

#[derive(Default)]
struct RecoverableStorage {
    objects: Mutex<HashMap<String, Bytes>>,
    final_puts: AtomicUsize,
    fail_readback: AtomicBool,
    fail_put_reply: AtomicBool,
}

#[async_trait]
impl BlobStorage for RecoverableStorage {
    fn backend_name(&self) -> &'static str {
        "nats"
    }

    async fn put(&self, key: &str, content: Bytes) -> Result<(), Status> {
        self.objects.lock().unwrap().insert(key.into(), content);
        if key.starts_with("artifacts/") {
            self.final_puts.fetch_add(1, Ordering::SeqCst);
            if self.fail_put_reply.swap(false, Ordering::SeqCst) {
                return Err(Status::unavailable("injected lost provider acknowledgment"));
            }
        }
        Ok(())
    }

    async fn get(&self, key: &str, max_bytes: usize) -> Result<Bytes, Status> {
        if key.starts_with("artifacts/") && self.fail_readback.swap(false, Ordering::SeqCst) {
            return Err(Status::unavailable("injected transient final readback"));
        }
        let content = self
            .objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| Status::not_found("object not found"))?;
        if content.len() > max_bytes {
            return Err(Status::resource_exhausted("object too large"));
        }
        Ok(content)
    }

    async fn delete(&self, key: &str) -> Result<(), Status> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn presign_put(
        &self,
        _: &str,
        _: Duration,
        _: u64,
        _: &str,
        _: &str,
    ) -> Result<Option<SignedUpload>, Status> {
        Ok(None)
    }

    async fn presign_get(&self, _: &str, _: Duration, _: &str) -> Result<Option<String>, Status> {
        Ok(None)
    }
}

async fn fixture() -> (
    ArtifactService,
    Arc<RecoverableStorage>,
    async_nats::jetstream::kv::Store,
    Arc<trogon_atlas_store::NatsStore>,
) {
    let nats = trogon_atlas_testsupport::shared().await;
    let js = async_nats::jetstream::new(async_nats::connect(&nats.url).await.unwrap());
    let catalog = js
        .create_key_value(async_nats::jetstream::kv::Config {
            bucket: format!(
                "artifact-retry-{}",
                trogon_atlas_testsupport::unique_suffix_for_test()
            ),
            history: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let storage = Arc::new(RecoverableStorage::default());
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

async fn uploaded(service: &ArtifactService) -> String {
    let content = Bytes::from_static(b"verified upload");
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
    service
        .upload("sample", &upload.upload_id, "writer", content)
        .await
        .unwrap();
    upload.upload_id
}

#[tokio::test]
async fn uploaded_retry_accepts_identical_bytes_and_refuses_mismatch_without_overwrite() {
    let (service, storage, _, _store) = fixture().await;
    let id = uploaded(&service).await;
    service
        .upload(
            "sample",
            &id,
            "writer",
            Bytes::from_static(b"verified upload"),
        )
        .await
        .expect("a lost transfer response must permit verified retry");
    assert_eq!(
        service
            .upload(
                "sample",
                &id,
                "writer",
                Bytes::from_static(b"modified upload"),
            )
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        storage.objects.lock().unwrap()[&format!("staging/{id}")],
        b"verified upload".as_slice()
    );
    service.complete("sample", &id, "writer").await.unwrap();
    assert_eq!(
        service
            .upload(
                "sample",
                &id,
                "writer",
                Bytes::from_static(b"verified upload"),
            )
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
}

#[tokio::test]
async fn final_candidate_survives_failed_readback_and_is_reused_after_restart() {
    candidate_recovery(true).await;
}

#[tokio::test]
async fn final_candidate_survives_ambiguous_provider_write() {
    candidate_recovery(false).await;
}

async fn candidate_recovery(readback_failure: bool) {
    let (service, storage, catalog, store) = fixture().await;
    let id = uploaded(&service).await;
    storage
        .fail_readback
        .store(readback_failure, Ordering::SeqCst);
    storage
        .fail_put_reply
        .store(!readback_failure, Ordering::SeqCst);
    assert_eq!(
        service
            .complete("sample", &id, "writer")
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    let row: serde_json::Value =
        serde_json::from_slice(&catalog.get(&id).await.unwrap().unwrap()).unwrap();
    let final_key = row["final_key"]
        .as_str()
        .expect("candidate identity must be durable before sending bytes")
        .to_owned();
    assert!(storage.objects.lock().unwrap().contains_key(&final_key));
    assert_eq!(storage.final_puts.load(Ordering::SeqCst), 1);
    drop(service);
    let restarted = ArtifactService::new(
        storage.clone(),
        catalog,
        1024,
        Duration::from_secs(60),
        store.artifact_publication_authority().unwrap(),
    )
    .unwrap();
    storage.fail_readback.store(true, Ordering::SeqCst);
    assert_eq!(
        restarted
            .complete("sample", &id, "writer")
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(storage.final_puts.load(Ordering::SeqCst), 1);
    let artifact = restarted.complete("sample", &id, "writer").await.unwrap();
    assert_eq!(
        restarted.complete("sample", &id, "writer").await.unwrap(),
        artifact
    );
    assert_eq!(storage.final_puts.load(Ordering::SeqCst), 1);
    assert_eq!(
        restarted.content("sample", &id).await.unwrap().content,
        b"verified upload"
    );
}

#[tokio::test]
async fn public_constructor_rejects_subsecond_ttl_but_accepts_one_second_and_one_hour() {
    let (_, storage, catalog, store) = fixture().await;
    for ttl in [Duration::from_nanos(1), Duration::from_millis(999)] {
        assert_eq!(
            ArtifactService::new(
                storage.clone(),
                catalog.clone(),
                1024,
                ttl,
                store.artifact_publication_authority().unwrap()
            )
            .err()
            .expect("subsecond TTL cannot be advertised or honored")
            .code(),
            Code::InvalidArgument
        );
    }
    for ttl in [Duration::from_secs(1), Duration::from_secs(3600)] {
        assert!(ArtifactService::new(
            storage.clone(),
            catalog.clone(),
            1024,
            ttl,
            store.artifact_publication_authority().unwrap()
        )
        .is_ok());
    }
}

#[tokio::test]
async fn ready_without_matching_authority_is_unreadable_and_bytes_are_retained() {
    let (service, storage, catalog, _store) = fixture().await;
    let id = uploaded(&service).await;
    service.complete("sample", &id, "writer").await.unwrap();
    let accepted = catalog.get(&id).await.unwrap().unwrap();
    let mut row: serde_json::Value = serde_json::from_slice(&accepted).unwrap();
    row.as_object_mut().unwrap().remove("receipt_id");
    catalog
        .put(&id, serde_json::to_vec(&row).unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        service.get("sample", &id).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        service.content("sample", &id).await.err().unwrap().code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        service
            .complete("sample", &id, "writer")
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert!(storage
        .objects
        .lock()
        .unwrap()
        .contains_key(row["final_key"].as_str().unwrap()));
    let mut row: serde_json::Value = serde_json::from_slice(&accepted).unwrap();
    row["filename"] = "changed.bin".into();
    catalog
        .put(&id, serde_json::to_vec(&row).unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        service.get("sample", &id).await.unwrap_err().code(),
        Code::Unavailable
    );
    let mut row: serde_json::Value = serde_json::from_slice(&accepted).unwrap();
    row["principal"] = "another-writer".into();
    catalog
        .put(&id, serde_json::to_vec(&row).unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        service
            .complete("sample", &id, "another-writer")
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    catalog.put(&id, accepted).await.unwrap();
    assert!(service.get("sample", &id).await.is_ok());
}

#[tokio::test]
async fn accepted_receipt_recovers_materialization_after_expiry_for_its_creator() {
    let (service, storage, catalog, _store) = fixture().await;
    let id = uploaded(&service).await;
    let accepted = service.complete("sample", &id, "writer").await.unwrap();
    let mut row: serde_json::Value =
        serde_json::from_slice(&catalog.get(&id).await.unwrap().unwrap()).unwrap();
    row["phase"] = "Verifying".into();
    row["expires"] = 0.into();
    catalog
        .put(&id, serde_json::to_vec(&row).unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        service
            .complete("sample", &id, "another-writer")
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(
        service.complete("sample", &id, "writer").await.unwrap(),
        accepted
    );
    assert_eq!(storage.final_puts.load(Ordering::SeqCst), 1);
    assert_eq!(service.get("sample", &id).await.unwrap(), accepted);
}
