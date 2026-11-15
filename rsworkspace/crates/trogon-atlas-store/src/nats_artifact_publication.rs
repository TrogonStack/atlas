use std::sync::Arc;

use async_nats::jetstream::{
    kv::Store as KvStore,
    message::PublishMessage,
    stream::{DiscardPolicy, LastRawMessageErrorKind, RetentionPolicy, StorageType, Stream},
    Context,
};
use async_trait::async_trait;
use prost::Message as _;

use super::{writer_lease::WriterLease, KV_OP_TIMEOUT};
use crate::{
    artifact_publication::{
        ArtifactPublicationAuthority, ArtifactPublicationCandidate, ArtifactPublicationPermit,
        ArtifactPublicationReceipt, ArtifactPublicationState, ArtifactReceiptId,
    },
    StoreError, StoreResult,
};

const MAX_ATTEMPTS: usize = 8;

#[derive(Clone, PartialEq, prost::Message)]
struct ReceiptRow {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(string, tag = "2")]
    id: String,
    #[prost(bytes = "vec", tag = "3")]
    candidate_hash: Vec<u8>,
    #[prost(string, tag = "4")]
    holder: String,
    #[prost(uint64, tag = "5")]
    epoch: u64,
    #[prost(uint64, tag = "6")]
    previous_sequence: u64,
    #[prost(uint64, tag = "7")]
    lease_revision: u64,
    #[prost(int64, tag = "8")]
    lease_expires_at_micros: i64,
}

pub(super) struct NatsArtifactPublicationAuthority {
    context: Context,
    bucket: String,
    lease: Arc<WriterLease>,
}

impl NatsArtifactPublicationAuthority {
    pub(super) fn new(context: Context, lease_kv: KvStore, lease: Arc<WriterLease>) -> Self {
        Self {
            context,
            bucket: lease_kv.name,
            lease,
        }
    }

    fn subject(&self, id: &ArtifactReceiptId) -> String {
        format!("$KV.{}.artifact-receipt.{}", self.bucket, id.as_str())
    }

    async fn stream(&self) -> StoreResult<Stream> {
        let stream = tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.context.get_stream(format!("KV_{}", self.bucket)),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        let config = &stream.cached_info().config;
        if config.storage != StorageType::File
            || config.max_messages_per_subject != 1
            || !config.max_age.is_zero()
            || config.retention != RetentionPolicy::Limits
            || (config.discard == DiscardPolicy::Old
                && (config.max_bytes > 0 || config.max_messages > 0))
            || config.discard_new_per_subject
            || config.mirror.is_some()
            || config
                .sources
                .as_ref()
                .is_some_and(|sources| !sources.is_empty())
            || config.subject_transform.is_some()
            || config.republish.is_some()
            || config.allow_message_ttl
            || config.allow_message_schedules
            || config.allow_message_counter
            || config.subjects != [format!("$KV.{}.>", self.bucket)]
        {
            return Err(StoreError::Unavailable(
                "artifact publication stream must use file-backed storage and retain immutable receipts without expiry, eviction, sources or transforms".into(),
            ));
        }
        Ok(stream)
    }

    async fn verify_in(
        &self,
        stream: &Stream,
        candidate: &ArtifactPublicationCandidate,
        id: &ArtifactReceiptId,
    ) -> StoreResult<ArtifactPublicationState> {
        let subject = self.subject(id);
        let message = match tokio::time::timeout(
            KV_OP_TIMEOUT,
            stream.get_last_raw_message_by_subject(&subject),
        )
        .await
        .map_err(|_| unavailable())?
        {
            Ok(message) => message,
            Err(error) if error.kind() == LastRawMessageErrorKind::NoMessageFound => {
                return Ok(ArtifactPublicationState::Missing);
            }
            Err(_) => return Err(unavailable()),
        };
        let row = ReceiptRow::decode(message.payload)
            .map_err(|_| StoreError::Unavailable("invalid artifact publication receipt".into()))?;
        if row.version != 1
            || row.id != id.as_str()
            || row.candidate_hash != candidate.hash
            || row.holder.is_empty()
            || row.epoch == 0
            || row.lease_revision == 0
            || row.lease_revision > row.previous_sequence
            || row.previous_sequence.checked_add(1) != Some(message.sequence)
            || row.lease_expires_at_micros <= 0
            || message.subject.as_str() != subject
        {
            return Err(StoreError::Unavailable(
                "artifact publication receipt does not match candidate".into(),
            ));
        }
        let expiry =
            async_nats::datetime::from_nanos(i128::from(row.lease_expires_at_micros) * 1000)
                .map_err(|_| {
                    StoreError::Unavailable("invalid artifact publication expiry".into())
                })?;
        if message.time >= expiry {
            return Ok(ArtifactPublicationState::Rejected);
        }
        Ok(ArtifactPublicationState::Accepted(
            ArtifactPublicationReceipt {
                id: id.clone(),
                sequence: message.sequence,
            },
        ))
    }
}

#[async_trait]
impl ArtifactPublicationAuthority for NatsArtifactPublicationAuthority {
    async fn validate(&self) -> StoreResult<()> {
        self.stream().await.map(|_| ())
    }

    async fn capture_permit(&self) -> StoreResult<ArtifactPublicationPermit> {
        let (holder, epoch) = self.lease.publication_identity()?;
        let stream = self.stream().await?;
        self.lease
            .publication_snapshot(
                &self.context,
                &stream,
                &format!("$KV.{}.writer-lease", self.bucket),
                &holder,
                epoch,
            )
            .await?;
        Ok(ArtifactPublicationPermit {
            bucket: self.bucket.clone(),
            holder,
            epoch,
        })
    }

    async fn publish(
        &self,
        permit: &ArtifactPublicationPermit,
        candidate: &ArtifactPublicationCandidate,
        id: &ArtifactReceiptId,
    ) -> StoreResult<ArtifactPublicationReceipt> {
        if permit.bucket != self.bucket {
            return Err(StoreError::InvalidArgument(
                "artifact permit belongs to another store".into(),
            ));
        }
        for _ in 0..MAX_ATTEMPTS {
            let stream = self.stream().await?;
            match self.verify_in(&stream, candidate, id).await? {
                ArtifactPublicationState::Accepted(receipt) => return Ok(receipt),
                ArtifactPublicationState::Rejected => return Err(rejected()),
                ArtifactPublicationState::Missing => {}
            }
            let previous_sequence = stream.cached_info().state.last_sequence;
            let lease = self
                .lease
                .publication_snapshot(
                    &self.context,
                    &stream,
                    &format!("$KV.{}.writer-lease", self.bucket),
                    &permit.holder,
                    permit.epoch,
                )
                .await?;
            if lease.revision > previous_sequence {
                continue;
            }
            let row = ReceiptRow {
                version: 1,
                id: id.as_str().into(),
                candidate_hash: candidate.hash.clone(),
                holder: permit.holder.clone(),
                epoch: permit.epoch.get(),
                previous_sequence,
                lease_revision: lease.revision,
                lease_expires_at_micros: lease.expires_at_micros,
            };
            let message = PublishMessage::build()
                .expected_stream(&stream.cached_info().config.name)
                .expected_last_sequence(previous_sequence)
                .expected_last_subject_sequence(0)
                .payload(row.encode_to_vec().into());
            let outcome = tokio::time::timeout(KV_OP_TIMEOUT, async {
                self.context
                    .send_publish(self.subject(id), message)
                    .await?
                    .await
            })
            .await;
            match self.verify(candidate, id).await? {
                ArtifactPublicationState::Accepted(receipt) => return Ok(receipt),
                ArtifactPublicationState::Rejected => return Err(rejected()),
                ArtifactPublicationState::Missing => {}
            }
            if !matches!(outcome, Ok(Err(error)) if error.kind() == async_nats::jetstream::context::PublishErrorKind::WrongLastSequence)
            {
                return Err(unavailable());
            }
        }
        Err(StoreError::Unavailable(
            "artifact publication contention; retry the same upload".into(),
        ))
    }

    async fn verify(
        &self,
        candidate: &ArtifactPublicationCandidate,
        id: &ArtifactReceiptId,
    ) -> StoreResult<ArtifactPublicationState> {
        self.verify_in(&self.stream().await?, candidate, id).await
    }
}

fn unavailable() -> StoreError {
    StoreError::Unavailable(
        "artifact publication authority unavailable; retry the same upload".into(),
    )
}

fn rejected() -> StoreError {
    StoreError::Unavailable("artifact publication arrived after its writer lease expired".into())
}
