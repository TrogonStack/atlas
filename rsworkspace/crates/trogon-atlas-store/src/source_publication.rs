use std::{collections::BTreeSet, sync::Arc, time::Duration};

use bytes::Bytes;
use prost::Message;
use sha2::{Digest, Sha256};
use trogon_atlas_proto as pb;

use super::{
    kv_entry_timeout, live_entry,
    native_fence::{blocked, Gate},
    now_micros, NatsStore, Revision, KV_OP_TIMEOUT,
};
use crate::{
    source::{
        SourceDocumentKey, SourcePublication, SourcePublicationObserver, SourcePublicationOutcome,
        SourcePublicationPoint, SOURCE_DOCUMENT_MAX_BYTES,
    },
    store::{ChangeKind, ChangesetOp, ChangesetRecord, ChangesetRef, Store, WriteContext},
    StoreError, StoreResult,
};

const SOURCE_CHUNK_BYTES: usize = 256 * 1024;

fn source_chunk_names(prefix: &str, byte_count: usize) -> Vec<String> {
    (0..byte_count.div_ceil(SOURCE_CHUNK_BYTES))
        .map(|index| format!("{prefix}.{index}"))
        .collect()
}

#[derive(Clone, PartialEq, Message)]
struct Transaction {
    #[prost(string, tag = "1")]
    registration: String,
    #[prost(string, tag = "2")]
    run_id: String,
    #[prost(string, tag = "3")]
    digest: String,
    #[prost(string, optional, tag = "4")]
    branch: Option<String>,
    #[prost(uint32, tag = "5")]
    phase: u32,
    #[prost(string, repeated, tag = "6")]
    prior_chunks: Vec<String>,
    #[prost(bytes = "vec", tag = "7")]
    prior_digest: Vec<u8>,
    #[prost(string, repeated, tag = "8")]
    marker_chunks: Vec<String>,
    #[prost(bytes = "vec", tag = "9")]
    marker_digest: Vec<u8>,
    #[prost(string, tag = "10")]
    changeset: String,
    #[prost(uint64, tag = "11")]
    writer_epoch: u64,
    #[prost(bool, tag = "12")]
    published: bool,
}
#[derive(Clone, PartialEq, Message)]
struct Priors {
    #[prost(message, repeated, tag = "1")]
    rows: Vec<Prior>,
}
#[derive(Clone, PartialEq, Message)]
struct Prior {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(bytes = "vec", optional, tag = "2")]
    raw: Option<Vec<u8>>,
    #[prost(message, optional, tag = "3")]
    entity: Option<pb::Entity>,
    #[prost(message, optional, tag = "4")]
    owner: Option<pb::EntityRef>,
}

fn transaction_key(registration: &SourceDocumentKey, run: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((registration.as_str().len() as u64).to_be_bytes());
    digest.update(registration.as_str().as_bytes());
    digest.update(run.as_bytes());
    format!(
        "t.{}.{}",
        super::source_control::registration_hash(registration),
        super::source_control::hex(&digest.finalize())
    )
}

impl NatsStore {
    #[must_use]
    pub fn with_source_publication_observer(
        mut self,
        observer: Arc<dyn SourcePublicationObserver>,
    ) -> Self {
        self.source_observer = Some(observer);
        self
    }
    fn source_point(&self, point: SourcePublicationPoint) -> StoreResult<()> {
        self.source_observer
            .as_ref()
            .map_or(Ok(()), |o| o.reached(point))
    }
    async fn source_chunks(
        &self,
        prefix: &str,
        bytes: &[u8],
        point: fn(usize) -> SourcePublicationPoint,
    ) -> StoreResult<Vec<String>> {
        if bytes.len() > SOURCE_DOCUMENT_MAX_BYTES {
            return Err(StoreError::InvalidArgument(
                "source publication storage budget exceeded".into(),
            ));
        }
        let mut chunks = Vec::new();
        for ((index, part), key) in bytes
            .chunks(SOURCE_CHUNK_BYTES)
            .enumerate()
            .zip(source_chunk_names(prefix, bytes.len()))
        {
            self.require_writer()?;
            tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.source_kv.put(&key, Bytes::copy_from_slice(part)),
            )
            .await
            .map_err(|_| blocked())?
            .map_err(|_| blocked())?;
            chunks.push(key);
            self.source_point(point(index))?;
        }
        Ok(chunks)
    }
    async fn read_source_chunks(&self, chunks: &[String], digest: &[u8]) -> StoreResult<Vec<u8>> {
        if chunks.len() > 512 || digest.len() != 32 {
            return Err(blocked());
        }
        let mut bytes = Vec::new();
        for key in chunks {
            let part = live_entry(kv_entry_timeout(&self.source_kv, key.clone()).await?)
                .ok_or_else(blocked)?;
            if part.value.len() > SOURCE_CHUNK_BYTES
                || bytes.len().saturating_add(part.value.len()) > SOURCE_DOCUMENT_MAX_BYTES
            {
                return Err(blocked());
            }
            bytes.extend_from_slice(&part.value);
        }
        if Sha256::digest(&bytes).as_slice() != digest {
            return Err(blocked());
        }
        Ok(bytes)
    }
    async fn load_transaction(&self, key: &str) -> StoreResult<Option<(u64, Transaction)>> {
        live_entry(kv_entry_timeout(&self.source_kv, key.to_owned()).await?)
            .map(|e| {
                let transaction = Transaction::decode(e.value.as_ref()).map_err(|_| blocked())?;
                Ok((e.revision, transaction))
            })
            .transpose()
    }
    async fn update_transaction(
        &self,
        key: &str,
        revision: u64,
        transaction: &Transaction,
    ) -> StoreResult<u64> {
        self.require_writer()?;
        self.lease
            .verify_epoch(&self.lease_kv, self.lease.current_epoch())
            .await?;
        tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.source_kv
                .update(key, Bytes::from(transaction.encode_to_vec()), revision),
        )
        .await
        .map_err(|_| blocked())?
        .map_err(|_| blocked())
    }
    async fn transaction_marker(
        &self,
        transaction: &Transaction,
    ) -> StoreResult<pb::SourceCommitMarker> {
        let bytes = self
            .read_source_chunks(&transaction.marker_chunks, &transaction.marker_digest)
            .await?;
        let marker = pb::SourceCommitMarker::decode(bytes.as_slice()).map_err(|_| blocked())?;
        if marker.run_id != transaction.run_id
            || marker.plan_digest != transaction.digest
            || marker.branch != transaction.branch
        {
            return Err(blocked());
        }
        Ok(marker)
    }
    async fn release_source_gate(&self, key: &str) -> StoreResult<()> {
        let (revision, gate) = self.load_source_gate().await?;
        if gate.busy {
            if gate.transaction != key {
                return Err(blocked());
            }
            self.set_source_gate(
                revision,
                Gate {
                    busy: false,
                    transaction: key.into(),
                    writer_epoch: self.lease.current_epoch().get(),
                },
            )
            .await?;
        }
        Ok(())
    }
    async fn check_publication_guards(&self, publication: &SourcePublication) -> StoreResult<()> {
        let plan = &publication.plan;
        if plan.operations.len() > 1000
            || plan.native_guards.len() > 5000
            || publication.values.len() > 4096
        {
            return Err(StoreError::InvalidArgument(
                "source publication work budget exceeded".into(),
            ));
        }
        let current = self
            .read_source_document(&publication.registration)
            .await?
            .ok_or(StoreError::NotFound)?;
        if current.revision != publication.control_revision {
            return Err(StoreError::EtagMismatch {
                expected: publication.control_revision.to_string(),
                found: current.revision.to_string(),
            });
        }
        for guard in &plan.native_guards {
            let owner = guard
                .owner
                .as_ref()
                .ok_or_else(|| StoreError::InvalidArgument("source guard has no owner".into()))?;
            let kind = pb::EntityKind::try_from(owner.kind)
                .map_err(|_| StoreError::InvalidArgument("invalid source kind".into()))?;
            let id = owner.id.as_ref().ok_or_else(|| {
                StoreError::InvalidArgument("source guard has no identity".into())
            })?;
            let current = self.get(kind, id, plan.branch.as_deref()).await;
            match (&guard.condition, current) {
                (
                    Some(pb::source_native_guard::Condition::Absent(true)),
                    Err(StoreError::NotFound),
                ) => {}
                (Some(pb::source_native_guard::Condition::Existing(observed)), Ok(current))
                    if current.etag == observed.etag
                        && current
                            .entity
                            .system
                            .as_ref()
                            .is_some_and(|s| s.uid == observed.uid) => {}
                (_, Err(e)) if !matches!(e, StoreError::NotFound) => return Err(e),
                _ => {
                    return Err(StoreError::InvalidArgument(
                        "source native guard no longer holds".into(),
                    ))
                }
            }
        }
        let mut targets = Vec::new();
        let mut overlay = Vec::new();
        let mut seen = BTreeSet::new();
        for operation in &plan.operations {
            let (guard, entity) = match operation.operation.as_ref() {
                Some(pb::source_native_operation::Operation::Put(put)) => {
                    (put.guard.as_ref(), put.entity.clone())
                }
                Some(pb::source_native_operation::Operation::Delete(delete)) => {
                    (delete.guard.as_ref(), None)
                }
                None => {
                    return Err(StoreError::InvalidArgument(
                        "source operation is empty".into(),
                    ))
                }
            };
            let guard = guard.ok_or_else(|| {
                StoreError::InvalidArgument("source operation has no guard".into())
            })?;
            if !plan.native_guards.contains(guard) {
                return Err(StoreError::InvalidArgument(
                    "source operation guard not retained".into(),
                ));
            }
            let owner = guard.owner.as_ref().ok_or_else(|| {
                StoreError::InvalidArgument("source operation has no owner".into())
            })?;
            let kind = pb::EntityKind::try_from(owner.kind)
                .map_err(|_| StoreError::InvalidArgument("invalid source kind".into()))?;
            let id = owner.id.as_ref().ok_or_else(|| {
                StoreError::InvalidArgument("source operation has no identity".into())
            })?;
            let key = pb::EntityKey::new(kind, id);
            if !seen.insert(crate::key::entity_key(kind, id)) {
                return Err(StoreError::InvalidArgument(
                    "source operation duplicates native owner".into(),
                ));
            }
            if entity.is_none() {
                targets.push(key.clone());
            }
            overlay.push((key, entity));
        }
        if !self
            .referenced_targets_with_overlay(&targets, plan.branch.as_deref(), &overlay)
            .await?
            .is_empty()
        {
            return Err(StoreError::InvalidArgument(
                "source deletion has a surviving dependency".into(),
            ));
        }
        Ok(())
    }

    fn check_transaction_identity(
        transaction: &Transaction,
        publication: &SourcePublication,
    ) -> StoreResult<()> {
        if transaction.registration != publication.registration.as_str()
            || transaction.run_id != publication.run_id
            || transaction.digest != publication.plan_digest
            || transaction.branch != publication.plan.branch
        {
            return Err(StoreError::InvalidArgument(
                "source run identity conflicts".into(),
            ));
        }
        Ok(())
    }

    pub(super) async fn prepare_source(&self, publication: &SourcePublication) -> StoreResult<()> {
        self.require_writer()?;
        let _native =
            tokio::time::timeout(Duration::from_secs(60), self.source_native_lock.write())
                .await
                .map_err(|_| blocked())?;
        let _document = self.source_document_lock.lock().await;
        self.prepare_source_locked(publication).await?;
        Ok(())
    }

    async fn prepare_source_locked(
        &self,
        publication: &SourcePublication,
    ) -> StoreResult<(u64, Transaction)> {
        let key = transaction_key(&publication.registration, &publication.run_id);
        if let Some(existing) = self.load_transaction(&key).await? {
            Self::check_transaction_identity(&existing.1, publication)?;
            return Ok(existing);
        }
        let current = self
            .read_source_document(&publication.registration)
            .await?
            .ok_or(StoreError::NotFound)?;
        if current.revision != publication.control_revision {
            return Err(StoreError::EtagMismatch {
                expected: publication.control_revision.to_string(),
                found: current.revision.to_string(),
            });
        }
        let transaction = Transaction {
            registration: publication.registration.as_str().into(),
            run_id: publication.run_id.clone(),
            digest: publication.plan_digest.clone(),
            branch: publication.plan.branch.clone(),
            phase: 0,
            changeset: uuid::Uuid::now_v7().to_string(),
            writer_epoch: self.lease.current_epoch().get(),
            ..Default::default()
        };
        let bytes = transaction.encode_to_vec();
        if publication.run_id.is_empty()
            || publication.run_id.len() > 1024
            || publication.plan_digest.is_empty()
            || publication.plan_digest.len() > 1024
            || bytes.len() > 65536
            || publication.storage_limit == 0
            || publication.storage_limit > SOURCE_DOCUMENT_MAX_BYTES as u64
            || self
                .source_storage_usage(&publication.registration)
                .await?
                .saturating_add(bytes.len() as u64)
                > publication.storage_limit
        {
            return Err(StoreError::InvalidArgument(
                "source preparation storage budget exceeded".into(),
            ));
        }
        self.lease
            .verify_epoch(&self.lease_kv, self.lease.current_epoch())
            .await?;
        let revision = tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.source_kv.create(&key, Bytes::from(bytes)),
        )
        .await
        .map_err(|_| blocked())?
        .map_err(|_| blocked())?;
        self.source_point(SourcePublicationPoint::IntentDurable)?;
        Ok((revision, transaction))
    }

    pub(super) async fn publish_source(
        &self,
        publication: &SourcePublication,
    ) -> StoreResult<SourcePublicationOutcome> {
        self.require_writer()?;
        let key = transaction_key(&publication.registration, &publication.run_id);
        let _lock = tokio::time::timeout(Duration::from_secs(60), self.source_native_lock.write())
            .await
            .map_err(|_| blocked())?;
        let _document = self.source_document_lock.lock().await;
        let (mut revision, mut transaction) = self.prepare_source_locked(publication).await?;
        match transaction.phase {
            0 => {}
            2 => {
                return self
                    .finalize_source_commit(&key, revision, &mut transaction)
                    .await;
            }
            3 => return Ok(SourcePublicationOutcome::NotCommitted),
            _ => return Err(blocked()),
        }
        let (gate_revision, gate) = self.load_source_gate().await?;
        if gate.busy {
            return Err(blocked());
        }
        self.source_scope(async {
            self.check_publication_guards(publication).await?;
            let mut priors = Priors::default();
            for operation in &publication.plan.operations {
                let guard = match operation.operation.as_ref() {
                    Some(pb::source_native_operation::Operation::Put(p)) => p.guard.as_ref(),
                    Some(pb::source_native_operation::Operation::Delete(d)) => d.guard.as_ref(),
                    None => None,
                }
                .ok_or_else(blocked)?;
                let owner = guard.owner.as_ref().ok_or_else(blocked)?;
                let kind = pb::EntityKind::try_from(owner.kind).map_err(|_| blocked())?;
                let id = owner.id.as_ref().ok_or_else(blocked)?;
                let raw_key = match &publication.plan.branch {
                    Some(branch) => crate::key::branch_delta_key(branch, kind, id),
                    None => crate::key::entity_key(kind, id),
                };
                let kv = if publication.plan.branch.is_some() {
                    &self.branches_kv
                } else {
                    &self.kv
                };
                let raw = live_entry(kv_entry_timeout(kv, raw_key.clone()).await?)
                    .map(|e| e.value.to_vec());
                let entity = match self.get(kind, id, publication.plan.branch.as_deref()).await {
                    Ok(e) => Some(e.entity),
                    Err(StoreError::NotFound) => None,
                    Err(e) => return Err(e),
                };
                priors.rows.push(Prior {
                    key: raw_key,
                    raw,
                    entity,
                    owner: Some(owner.clone()),
                });
            }
            let bytes = priors.encode_to_vec();
            let mut predicted = publication.values.clone();
            for value in &mut predicted {
                if let Some(operation) = publication
                    .plan
                    .operations
                    .iter()
                    .find(|o| o.mapping_id == value.mapping_id)
                {
                    if let Some(pb::source_native_operation::Operation::Put(put)) =
                        &operation.operation
                    {
                        let before = value.entity.as_ref();
                        value.entity = Some(
                            trogon_atlas_core::connection_persistence::prepare_entity(
                                before,
                                put.entity.as_ref().ok_or_else(blocked)?,
                            )
                            .map_err(|_| blocked())?,
                        );
                        if let Some(observed) = &mut value.observed {
                            observed.uid = value
                                .entity
                                .as_ref()
                                .and_then(|e| e.system.as_ref())
                                .map(|s| s.uid.clone())
                                .unwrap_or_default();
                            observed.etag = u64::MAX.to_string();
                        }
                    }
                }
            }
            let predicted_marker = pb::SourceCommitMarker {
                run_id: publication.run_id.clone(),
                plan_digest: publication.plan_digest.clone(),
                branch: publication.plan.branch.clone(),
                values: predicted,
                native_operation_id: uuid::Uuid::now_v7().to_string(),
                committed_at_unix_ms: i64::MAX,
            };
            let mut completed_transaction = transaction.clone();
            completed_transaction.phase = 2;
            completed_transaction.published = true;
            completed_transaction.writer_epoch = self.lease.current_epoch().get();
            completed_transaction.prior_chunks =
                source_chunk_names(&format!("x.{key}.p"), bytes.len());
            completed_transaction.marker_chunks =
                source_chunk_names(&format!("x.{key}.m"), predicted_marker.encoded_len());
            completed_transaction.prior_digest = vec![0; 32];
            completed_transaction.marker_digest = vec![0; 32];
            let metadata_growth = completed_transaction
                .encoded_len()
                .saturating_sub(transaction.encoded_len());
            let required = self
                .source_storage_usage(&publication.registration)
                .await?
                .saturating_add(bytes.len() as u64)
                .saturating_add(predicted_marker.encoded_len() as u64)
                .saturating_add(publication.finalization_reserve_bytes)
                .saturating_add(metadata_growth as u64);
            if publication.storage_limit == 0
                || publication.storage_limit > SOURCE_DOCUMENT_MAX_BYTES as u64
                || required > publication.storage_limit
            {
                return Err(StoreError::Unavailable(
                    "source physical publication storage budget exhausted".into(),
                ));
            }
            transaction.writer_epoch = self.lease.current_epoch().get();
            transaction.prior_chunks = self
                .source_chunks(
                    &format!("x.{key}.p"),
                    &bytes,
                    SourcePublicationPoint::PriorChunkDurable,
                )
                .await?;
            transaction.prior_digest = Sha256::digest(&bytes).to_vec();
            self.set_source_gate(
                gate_revision,
                Gate {
                    busy: true,
                    transaction: key.clone(),
                    writer_epoch: transaction.writer_epoch,
                },
            )
            .await?;
            transaction.phase = 1;
            revision = self
                .update_transaction(&key, revision, &transaction)
                .await?;
            self.source_point(SourcePublicationPoint::FenceAcquired)?;
            for (index, operation) in publication.plan.operations.iter().enumerate() {
                self.require_writer()?;
                self.lease
                    .verify_epoch(
                        &self.lease_kv,
                        trogon_atlas_core::Epoch::new(transaction.writer_epoch),
                    )
                    .await?;
                self.source_point(SourcePublicationPoint::BeforeNativeWrite(index))?;
                let result = self
                    .source_apply_operation(operation, publication.plan.branch.as_deref())
                    .await;
                self.source_point(SourcePublicationPoint::AfterNativeWrite(index))?;
                if result.is_err() {
                    return self
                        .rollback_source_transaction(&key, revision, &mut transaction)
                        .await;
                }
            }
            let mut values = publication.values.clone();
            for value in &mut values {
                let owner = value
                    .observed
                    .as_ref()
                    .and_then(|o| o.owner.as_ref())
                    .ok_or_else(blocked)?
                    .clone();
                let kind = pb::EntityKind::try_from(owner.kind).map_err(|_| blocked())?;
                let id = owner.id.as_ref().ok_or_else(blocked)?;
                match self.get(kind, id, publication.plan.branch.as_deref()).await {
                    Ok(current) => {
                        value.observed = Some(pb::SourceObservedOwner {
                            owner: Some(owner),
                            uid: current
                                .entity
                                .system
                                .as_ref()
                                .map(|s| s.uid.clone())
                                .unwrap_or_default(),
                            etag: current.etag,
                        });
                        value.entity = Some(current.entity);
                        value.deleted = false;
                    }
                    Err(StoreError::NotFound) => {
                        value.entity = None;
                        value.deleted = true;
                    }
                    Err(e) => return Err(e),
                }
            }
            let marker = pb::SourceCommitMarker {
                run_id: publication.run_id.clone(),
                plan_digest: publication.plan_digest.clone(),
                branch: publication.plan.branch.clone(),
                values,
                native_operation_id: transaction.changeset.clone(),
                committed_at_unix_ms: now_micros() / 1000,
            };
            let bytes = marker.encode_to_vec();
            transaction.marker_chunks = self
                .source_chunks(
                    &format!("x.{key}.m"),
                    &bytes,
                    SourcePublicationPoint::MarkerChunkDurable,
                )
                .await?;
            transaction.marker_digest = Sha256::digest(&bytes).to_vec();
            self.source_point(SourcePublicationPoint::BeforeCommitDecision)?;
            self.lease
                .verify_epoch(
                    &self.lease_kv,
                    trogon_atlas_core::Epoch::new(transaction.writer_epoch),
                )
                .await?;
            transaction.phase = 2;
            revision = self
                .update_transaction(&key, revision, &transaction)
                .await?;
            self.source_point(SourcePublicationPoint::CommitDecisionDurable)?;
            self.release_source_gate(&key).await?;
            self.source_point(SourcePublicationPoint::FenceReleased)?;
            self.source_finalize_scope(async {
                self.publish_source_history(&key, revision, &mut transaction, &priors, &marker)
                    .await
            })
            .await?;
            Ok(SourcePublicationOutcome::Committed(marker))
        })
        .await
    }

    async fn source_apply_operation(
        &self,
        operation: &pb::SourceNativeOperation,
        branch: Option<&str>,
    ) -> StoreResult<()> {
        let ctx = WriteContext::scoped(branch);
        match operation.operation.as_ref().ok_or_else(blocked)? {
            pb::source_native_operation::Operation::Put(put) => {
                let entity = put.entity.as_ref().ok_or_else(blocked)?;
                let kind = trogon_atlas_core::refs::entity_kind(entity).ok_or_else(blocked)?;
                match put
                    .guard
                    .as_ref()
                    .and_then(|g| g.condition.as_ref())
                    .ok_or_else(blocked)?
                {
                    pb::source_native_guard::Condition::Absent(true) => {
                        self.create(kind, entity, ctx).await?;
                    }
                    pb::source_native_guard::Condition::Existing(observed) => {
                        self.update(kind, entity, &observed.etag, false, ctx)
                            .await?;
                    }
                    pb::source_native_guard::Condition::Absent(false) => return Err(blocked()),
                }
            }
            pb::source_native_operation::Operation::Delete(delete) => {
                let guard = delete.guard.as_ref().ok_or_else(blocked)?;
                let owner = guard.owner.as_ref().ok_or_else(blocked)?;
                let kind = pb::EntityKind::try_from(owner.kind).map_err(|_| blocked())?;
                let id = owner.id.as_ref().ok_or_else(blocked)?;
                let Some(pb::source_native_guard::Condition::Existing(observed)) = &guard.condition
                else {
                    return Err(blocked());
                };
                self.delete(kind, id, Some(&observed.etag), ctx).await?;
            }
        }
        Ok(())
    }

    async fn rollback_source_transaction(
        &self,
        key: &str,
        revision: u64,
        transaction: &mut Transaction,
    ) -> StoreResult<SourcePublicationOutcome> {
        let bytes = self
            .read_source_chunks(&transaction.prior_chunks, &transaction.prior_digest)
            .await?;
        let priors = Priors::decode(bytes.as_slice()).map_err(|_| blocked())?;
        let kv = if transaction.branch.is_some() {
            &self.branches_kv
        } else {
            &self.kv
        };
        let recovery_epoch = self.lease.current_epoch();
        for prior in priors.rows {
            self.require_writer()?;
            self.lease
                .verify_epoch(&self.lease_kv, recovery_epoch)
                .await?;
            let current = live_entry(kv_entry_timeout(kv, prior.key.clone()).await?);
            match (prior.raw, current) {
                (Some(raw), Some(current)) if raw.as_slice() == current.value.as_ref() => {}
                (Some(raw), Some(current)) => {
                    tokio::time::timeout(
                        KV_OP_TIMEOUT,
                        kv.update(&prior.key, Bytes::from(raw), current.revision),
                    )
                    .await
                    .map_err(|_| blocked())?
                    .map_err(|_| blocked())?;
                }
                (Some(raw), None) => {
                    tokio::time::timeout(KV_OP_TIMEOUT, kv.put(&prior.key, Bytes::from(raw)))
                        .await
                        .map_err(|_| blocked())?
                        .map_err(|_| blocked())?;
                }
                (None, Some(current)) => {
                    tokio::time::timeout(
                        KV_OP_TIMEOUT,
                        kv.delete_expect_revision(&prior.key, Some(current.revision)),
                    )
                    .await
                    .map_err(|_| blocked())?
                    .map_err(|_| blocked())?;
                }
                (None, None) => {}
            }
        }
        transaction.phase = 3;
        self.update_transaction(key, revision, transaction).await?;
        self.release_source_gate(key).await?;
        Ok(SourcePublicationOutcome::NotCommitted)
    }

    pub(super) async fn recover_source(
        &self,
        registration: &SourceDocumentKey,
        run: &str,
        digest: &str,
    ) -> StoreResult<SourcePublicationOutcome> {
        self.require_writer()?;
        let _lock = tokio::time::timeout(Duration::from_secs(60), self.source_native_lock.write())
            .await
            .map_err(|_| blocked())?;
        let _document = self.source_document_lock.lock().await;
        let key = transaction_key(registration, run);
        let (revision, mut transaction) = self.load_transaction(&key).await?.ok_or_else(blocked)?;
        if transaction.registration != registration.as_str()
            || transaction.run_id != run
            || transaction.digest != digest
        {
            return Err(StoreError::InvalidArgument(
                "source recovery identity conflicts".into(),
            ));
        }
        let (_, gate) = self.load_source_gate().await?;
        if transaction.phase != 0 && gate.busy && gate.transaction != key {
            return Err(blocked());
        }
        self.source_scope(async {
            match transaction.phase {
                0 => {
                    transaction.phase = 3;
                    self.update_transaction(&key, revision, &transaction)
                        .await?;
                    if gate.busy && gate.transaction == key {
                        self.release_source_gate(&key).await?;
                    }
                    Ok(SourcePublicationOutcome::NotCommitted)
                }
                1 => {
                    // An acquired fence blocks every competing native writer until rollback is definitive.
                    if !gate.busy {
                        return Err(blocked());
                    }
                    self.rollback_source_transaction(&key, revision, &mut transaction)
                        .await
                }
                2 => {
                    self.finalize_source_commit(&key, revision, &mut transaction)
                        .await
                }
                3 => {
                    if gate.busy && gate.transaction == key {
                        self.release_source_gate(&key).await?;
                    }
                    Ok(SourcePublicationOutcome::NotCommitted)
                }
                _ => Err(blocked()),
            }
        })
        .await
    }

    async fn finalize_source_commit(
        &self,
        key: &str,
        revision: u64,
        transaction: &mut Transaction,
    ) -> StoreResult<SourcePublicationOutcome> {
        let (_, gate) = self.load_source_gate().await?;
        if gate.busy && gate.transaction != key {
            return Err(blocked());
        }
        let marker = self.transaction_marker(transaction).await?;
        self.release_source_gate(key).await?;
        if !transaction.published {
            let bytes = self
                .read_source_chunks(&transaction.prior_chunks, &transaction.prior_digest)
                .await?;
            let priors = Priors::decode(bytes.as_slice()).map_err(|_| blocked())?;
            self.source_finalize_scope(self.publish_source_history(
                key,
                revision,
                transaction,
                &priors,
                &marker,
            ))
            .await?;
        }
        Ok(SourcePublicationOutcome::Committed(marker))
    }

    pub(super) async fn expire_source_transaction(
        &self,
        registration: &SourceDocumentKey,
        run: &str,
        digest: &str,
    ) -> StoreResult<()> {
        self.require_writer()?;
        let _lock = tokio::time::timeout(Duration::from_secs(60), self.source_native_lock.write())
            .await
            .map_err(|_| blocked())?;
        let _document = self.source_document_lock.lock().await;
        let key = transaction_key(registration, run);
        let Some((_, transaction)) = self.load_transaction(&key).await? else {
            return Ok(());
        };
        if transaction.registration != registration.as_str()
            || transaction.run_id != run
            || transaction.digest != digest
        {
            return Err(blocked());
        }
        let (_, gate) = self.load_source_gate().await?;
        if gate.busy && gate.transaction == key {
            return Err(blocked());
        }
        if transaction.phase != 3 && !(transaction.phase == 2 && transaction.published) {
            return Err(blocked());
        }
        self.lease
            .verify_epoch(&self.lease_kv, self.lease.current_epoch())
            .await?;
        // A crash may leave a staged chunk without a durable reference in the transaction.
        let prefix = format!("x.{key}.");
        for chunk in self.source_registration_keys(registration).await? {
            if !chunk.starts_with(&prefix) {
                continue;
            }
            self.require_writer()?;
            self.lease
                .verify_epoch(&self.lease_kv, self.lease.current_epoch())
                .await?;
            tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.purge(&chunk))
                .await
                .map_err(|_| blocked())?
                .map_err(|_| blocked())?;
        }
        tokio::time::timeout(KV_OP_TIMEOUT, self.source_kv.purge(&key))
            .await
            .map_err(|_| blocked())?
            .map_err(|_| blocked())?;
        Ok(())
    }

    async fn publish_source_history(
        &self,
        key: &str,
        revision: u64,
        transaction: &mut Transaction,
        priors: &Priors,
        marker: &pb::SourceCommitMarker,
    ) -> StoreResult<()> {
        if transaction.published {
            return Ok(());
        }
        let attribution = ChangesetRef {
            id: &transaction.changeset,
            author: "source",
            rpc: "ApplySourceRun",
            operation_id: None,
        };
        let mut ops = Vec::new();
        for (index, prior) in priors.rows.iter().enumerate() {
            let owner = prior.owner.as_ref().ok_or_else(blocked)?;
            let kind = pb::EntityKind::try_from(owner.kind).map_err(|_| blocked())?;
            let id = owner.id.as_ref().ok_or_else(blocked)?;
            let value = marker
                .values
                .iter()
                .find(|v| v.observed.as_ref().and_then(|o| o.owner.as_ref()) == Some(owner))
                .ok_or_else(blocked)?;
            if prior.entity == value.entity {
                continue;
            }
            let change = if value.deleted {
                ChangeKind::Delete
            } else {
                ChangeKind::Put
            };
            if transaction.branch.is_none() {
                self.publish_change(
                    change,
                    owner,
                    Some(attribution),
                    Some(&format!("{key}.{index}")),
                )
                .await?;
            }
            self.try_record_revision(
                WriteContext::scoped(transaction.branch.as_deref()).attributed(attribution),
                &Revision {
                    kind,
                    id: id.clone(),
                    change,
                    before: prior.entity.clone(),
                    after: value.entity.clone(),
                },
            )
            .await
            .map_err(|_| blocked())?;
            ops.push(ChangesetOp {
                kind: change,
                entity_ref: owner.clone(),
            });
        }
        self.append_changeset(&ChangesetRecord {
            id: transaction.changeset.clone(),
            author: "source".into(),
            at: super::now_rfc3339(),
            message: "Source reconciliation accepted".into(),
            rpc: "ApplySourceRun".into(),
            branch: transaction.branch.clone(),
            operation_id: None,
            validation: crate::ChangesetValidation::Unvalidated(
                crate::UnvalidatedReason::NoDeclaredCoverage,
            ),
            ops,
        })
        .await?;
        transaction.published = true;
        self.update_transaction(key, revision, transaction).await?;
        Ok(())
    }
}
