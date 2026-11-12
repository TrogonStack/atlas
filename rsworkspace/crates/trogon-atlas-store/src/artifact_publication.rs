use async_trait::async_trait;
use prost::Message as _;
use sha2::{Digest as _, Sha256};
use trogon_atlas_core::{Epoch, NamespaceId};
use trogon_atlas_proto::Artifact;

use crate::{StoreError, StoreResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactReceiptId(String);

impl ArtifactReceiptId {
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }

    pub fn parse(value: &str) -> StoreResult<Self> {
        match uuid::Uuid::parse_str(value) {
            Ok(id) if id.to_string() == value => Ok(Self(value.into())),
            _ => Err(StoreError::InvalidArgument(
                "invalid artifact receipt ID".into(),
            )),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Binds immutable accepted metadata to one verified final object generation.
pub struct ArtifactPublicationCandidate {
    pub(crate) hash: Vec<u8>,
}

impl ArtifactPublicationCandidate {
    pub fn new(
        artifact: &Artifact,
        backend: &str,
        final_key: &str,
        principal: &str,
    ) -> StoreResult<Self> {
        NamespaceId::parse(&artifact.namespace)
            .map_err(|_| StoreError::InvalidArgument("invalid artifact namespace".into()))?;
        uuid::Uuid::parse_str(&artifact.artifact_id)
            .map_err(|_| StoreError::InvalidArgument("invalid artifact ID".into()))?;
        final_key
            .strip_prefix("artifacts/")
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .ok_or_else(|| StoreError::InvalidArgument("invalid artifact final key".into()))?;
        if !matches!(backend, "nats" | "s3")
            || artifact.size == 0
            || artifact.filename.is_empty()
            || artifact.filename.len() > 256
            || artifact.media_type.is_empty()
            || artifact.media_type.len() > 128
            || artifact.sha256.len() != 64
            || !artifact
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StoreError::InvalidArgument(
                "invalid artifact candidate".into(),
            ));
        }
        let mut hash = Sha256::new();
        hash.update(b"atlas.artifact-publication.v1");
        for value in [
            artifact.encode_to_vec(),
            backend.as_bytes().to_vec(),
            final_key.as_bytes().to_vec(),
            principal.as_bytes().to_vec(),
        ] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value);
        }
        Ok(Self {
            hash: hash.finalize().to_vec(),
        })
    }
}

#[derive(Clone, Debug)]
/// Pins the original holder and epoch; same-holder renewals may extend commit expiry.
pub struct ArtifactPublicationPermit {
    pub(crate) bucket: String,
    pub(crate) holder: String,
    pub(crate) epoch: Epoch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactPublicationReceipt {
    pub(crate) id: ArtifactReceiptId,
    pub(crate) sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactPublicationState {
    Missing,
    Accepted(ArtifactPublicationReceipt),
    /// A stored attempt arrived after lease expiry and may be replaced by a fresh ID.
    Rejected,
}

#[async_trait]
/// Acceptance must linearize atomically with writer lease changes. Catalog Ready
/// rows are projections and require an exact Accepted receipt on every read.
pub trait ArtifactPublicationAuthority: Send + Sync {
    async fn validate(&self) -> StoreResult<()>;
    async fn capture_permit(&self) -> StoreResult<ArtifactPublicationPermit>;
    async fn publish(
        &self,
        permit: &ArtifactPublicationPermit,
        candidate: &ArtifactPublicationCandidate,
        id: &ArtifactReceiptId,
    ) -> StoreResult<ArtifactPublicationReceipt>;
    async fn verify(
        &self,
        candidate: &ArtifactPublicationCandidate,
        id: &ArtifactReceiptId,
    ) -> StoreResult<ArtifactPublicationState>;
}
