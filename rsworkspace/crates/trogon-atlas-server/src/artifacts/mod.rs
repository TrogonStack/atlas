pub mod config;
pub mod storage;

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_nats::jetstream::kv::{Operation, Store};
use bytes::Bytes;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tonic::Status;
use trogon_atlas_proto as pb;

use self::storage::BlobStorage;

pub const MAX_ARTIFACT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct Upload {
    namespace: String,
    id: String,
    principal: String,
    filename: String,
    media_type: String,
    size: u64,
    sha256: String,
    expires: i64,
    phase: Phase,
    staging_key: String,
    final_key: Option<String>,
    backend: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Pending,
    Receiving,
    Uploaded,
    Verifying,
    Ready,
}

pub struct ArtifactService {
    blobs: Arc<dyn BlobStorage>,
    catalog: Store,
    max_bytes: u64,
    upload_ttl: Duration,
}

impl ArtifactService {
    pub fn new(
        blobs: Arc<dyn BlobStorage>,
        catalog: Store,
        max_bytes: u64,
        upload_ttl: Duration,
    ) -> Result<Self, Status> {
        validate_configuration(max_bytes, upload_ttl)?;
        Ok(Self {
            blobs,
            catalog,
            max_bytes,
            upload_ttl,
        })
    }

    pub fn info(&self) -> pb::GetArtifactStorageInfoResponse {
        pb::GetArtifactStorageInfoResponse {
            enabled: true,
            backend: self.blobs.backend_name().into(),
            max_bytes: self.max_bytes,
            upload_ttl_seconds: self.upload_ttl.as_secs(),
            direct_upload: self.blobs.backend_name() == "s3",
        }
    }

    pub async fn begin(
        &self,
        request: pb::BeginArtifactUploadRequest,
        principal: &str,
    ) -> Result<pb::BeginArtifactUploadResponse, Status> {
        validate_namespace(&request.namespace)?;
        if request.filename.is_empty()
            || request.filename.len() > 256
            || request
                .filename
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
            || request.media_type.len() > 128
            || request.media_type.parse::<mime::Mime>().is_err()
            || request.size == 0
            || request.size > self.max_bytes
            || request.sha256.len() != 64
            || !request
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Status::invalid_argument(
                "invalid artifact filename, media type, size or SHA-256",
            ));
        }
        let id = uuid::Uuid::now_v7().to_string();
        let staging_key = format!("staging/{id}");
        let signed = self
            .blobs
            .presign_put(
                &staging_key,
                self.upload_ttl,
                request.size,
                &request.sha256,
                &request.media_type,
            )
            .await?;
        let direct_upload = signed.is_some();
        let (upload_url, headers) = signed
            .map(|signed| (signed.url, signed.headers))
            .unwrap_or_default();
        let expires =
            Utc::now().timestamp() + i64::try_from(self.upload_ttl.as_secs()).unwrap_or(3600);
        let upload = Upload {
            namespace: request.namespace,
            id: id.clone(),
            principal: principal.into(),
            filename: request.filename,
            media_type: request.media_type,
            size: request.size,
            sha256: request.sha256,
            expires,
            phase: Phase::Pending,
            staging_key,
            final_key: None,
            backend: self.blobs.backend_name().into(),
        };
        self.catalog
            .create(&id, encode(&upload)?)
            .await
            .map_err(catalog_error)?;
        Ok(pb::BeginArtifactUploadResponse {
            upload_id: id,
            upload_url,
            headers,
            expires_unix_seconds: expires,
            direct_upload,
        })
    }

    async fn load(&self, namespace: &str, id: &str) -> Result<(Upload, u64), Status> {
        validate_namespace(namespace)?;
        uuid::Uuid::parse_str(id).map_err(|_| Status::invalid_argument("invalid artifact ID"))?;
        let entry = self
            .catalog
            .entry(id)
            .await
            .map_err(catalog_error)?
            .filter(|entry| entry.operation == Operation::Put)
            .ok_or_else(|| Status::not_found("artifact not found"))?;
        let upload: Upload = serde_json::from_slice(&entry.value)
            .map_err(|_| Status::internal("invalid artifact catalog record"))?;
        if upload.namespace != namespace || upload.backend != self.blobs.backend_name() {
            return Err(Status::not_found("artifact not found"));
        }
        Ok((upload, entry.revision))
    }

    async fn update(&self, upload: &Upload, revision: u64) -> Result<u64, Status> {
        self.catalog
            .update(&upload.id, encode(upload)?, revision)
            .await
            .map_err(|_| Status::aborted("artifact changed; refresh upload status before retrying"))
    }

    fn require_session(upload: &Upload, principal: &str) -> Result<(), Status> {
        if upload.principal != principal {
            return Err(Status::not_found("upload not found"));
        }
        if upload.phase != Phase::Ready && upload.expires <= Utc::now().timestamp() {
            return Err(Status::failed_precondition(
                "upload expired; begin a new upload",
            ));
        }
        Ok(())
    }

    pub async fn upload(
        &self,
        namespace: &str,
        id: &str,
        principal: &str,
        content: Bytes,
    ) -> Result<(), Status> {
        let (mut upload, revision) = self.load(namespace, id).await?;
        Self::require_session(&upload, principal)?;
        if self.info().direct_upload || upload.phase != Phase::Pending {
            return Err(Status::failed_precondition(
                "upload is not awaiting content",
            ));
        }
        verify(&upload, &content, self.max_bytes)?;
        upload.phase = Phase::Receiving;
        let revision = self.update(&upload, revision).await?;
        if let Err(error) = self.blobs.put(&upload.staging_key, content).await {
            upload.phase = Phase::Pending;
            let _ = self.update(&upload, revision).await;
            return Err(error);
        }
        upload.phase = Phase::Uploaded;
        self.update(&upload, revision).await?;
        Ok(())
    }

    pub async fn complete(
        &self,
        namespace: &str,
        id: &str,
        principal: &str,
    ) -> Result<pb::Artifact, Status> {
        let (mut upload, revision) = self.load(namespace, id).await?;
        Self::require_session(&upload, principal)?;
        if upload.phase == Phase::Ready {
            return Ok(Self::artifact(&upload));
        }
        if !matches!(upload.phase, Phase::Pending | Phase::Uploaded) {
            return Err(Status::failed_precondition(
                "upload transfer or verification is in progress; begin a new upload if interrupted",
            ));
        }
        let previous = upload.phase;
        upload.phase = Phase::Verifying;
        let revision = self.update(&upload, revision).await?;
        let result = async {
            let byte_limit = usize::try_from(self.max_bytes).map_err(|_| {
                Status::failed_precondition("artifact byte limit exceeds platform capacity")
            })?;
            let content = self.blobs.get(&upload.staging_key, byte_limit).await?;
            verify(&upload, &content, self.max_bytes)?;
            let final_key = format!("artifacts/{}", uuid::Uuid::now_v7());
            self.blobs.put(&final_key, content).await?;
            let persisted = self.blobs.get(&final_key, byte_limit).await?;
            verify(&upload, &persisted, self.max_bytes)?;
            Ok::<_, Status>(final_key)
        }
        .await;
        let final_key = match result {
            Ok(key) => key,
            Err(error) => {
                upload.phase = previous;
                let _ = self.update(&upload, revision).await;
                return Err(error);
            }
        };
        upload.final_key = Some(final_key);
        upload.phase = Phase::Ready;
        self.update(&upload, revision).await?;
        if !self.info().direct_upload {
            let _ = self.blobs.delete(&upload.staging_key).await;
        }
        Ok(Self::artifact(&upload))
    }

    fn artifact(upload: &Upload) -> pb::Artifact {
        pb::Artifact {
            namespace: upload.namespace.clone(),
            artifact_id: upload.id.clone(),
            filename: upload.filename.clone(),
            media_type: upload.media_type.clone(),
            size: upload.size,
            sha256: upload.sha256.clone(),
        }
    }

    async fn ready(&self, namespace: &str, id: &str) -> Result<Upload, Status> {
        let (upload, _) = self.load(namespace, id).await?;
        if upload.phase != Phase::Ready {
            return Err(Status::not_found("artifact not found"));
        }
        Ok(upload)
    }

    pub async fn get(&self, namespace: &str, id: &str) -> Result<pb::Artifact, Status> {
        Ok(Self::artifact(&self.ready(namespace, id).await?))
    }

    pub async fn content(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<pb::GetArtifactContentResponse, Status> {
        let upload = self.ready(namespace, id).await?;
        let key = upload
            .final_key
            .as_deref()
            .ok_or_else(|| Status::internal("invalid artifact record"))?;
        let byte_limit = usize::try_from(MAX_ARTIFACT_BYTES).map_err(|_| {
            Status::failed_precondition("artifact byte limit exceeds platform capacity")
        })?;
        let content = self.blobs.get(key, byte_limit).await?;
        verify(&upload, &content, MAX_ARTIFACT_BYTES)?;
        Ok(pb::GetArtifactContentResponse {
            artifact: Some(Self::artifact(&upload)),
            content: content.to_vec(),
        })
    }

    pub async fn download(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<pb::GetArtifactDownloadResponse, Status> {
        let upload = self.ready(namespace, id).await?;
        let key = upload
            .final_key
            .as_deref()
            .ok_or_else(|| Status::internal("invalid artifact record"))?;
        let signed = self
            .blobs
            .presign_get(key, self.upload_ttl, &upload.media_type)
            .await?
            .ok_or_else(|| {
                Status::failed_precondition(
                    "this backend does not issue presigned downloads; use GetArtifactContent RPC",
                )
            })?;
        let expires = Utc::now().timestamp()
            + i64::try_from(self.upload_ttl.as_secs()).map_err(|_| {
                Status::failed_precondition("artifact expiry exceeds platform capacity")
            })?;
        Ok(pb::GetArtifactDownloadResponse {
            url: signed,
            headers: HashMap::new(),
            expires_unix_seconds: expires,
        })
    }
}

pub fn validate_namespace(namespace: &str) -> Result<(), Status> {
    trogon_atlas_core::NamespaceId::parse(namespace)
        .map(|_| ())
        .map_err(|_| Status::invalid_argument("invalid namespace ID"))
}

pub(super) fn validate_configuration(max_bytes: u64, upload_ttl: Duration) -> Result<(), Status> {
    if max_bytes == 0
        || max_bytes > MAX_ARTIFACT_BYTES
        || upload_ttl.is_zero()
        || upload_ttl > Duration::from_secs(3600)
    {
        return Err(Status::invalid_argument(
            "invalid artifact service configuration",
        ));
    }
    Ok(())
}

fn verify(upload: &Upload, content: &[u8], max_bytes: u64) -> Result<(), Status> {
    if content.len() as u64 > max_bytes || content.len() as u64 != upload.size {
        return Err(Status::invalid_argument(
            "artifact size does not match upload declaration",
        ));
    }
    if trogon_atlas_client::artifacts::sha256_hex(content) != upload.sha256 {
        return Err(Status::invalid_argument(
            "artifact digest does not match upload declaration",
        ));
    }
    Ok(())
}

fn encode(upload: &Upload) -> Result<Bytes, Status> {
    serde_json::to_vec(upload)
        .map(Bytes::from)
        .map_err(|_| Status::internal("cannot encode artifact record"))
}

fn catalog_error<E>(_: E) -> Status {
    Status::unavailable("artifact catalog unavailable")
}
