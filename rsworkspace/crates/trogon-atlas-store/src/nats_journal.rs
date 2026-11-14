use std::{collections::BTreeSet, sync::Arc};

use async_nats::jetstream::kv::{Entry as KvEntry, Store as KvStore};
use bytes::Bytes;
use futures::StreamExt;
use prost::Message as _;
use trogon_atlas_proto::{Entity, EntityKind, EntityRef, Id};

use super::{
    decode_entity, kv_entry_timeout, live_entry, now_micros, now_rfc3339,
    rollback_baseline_snapshot, validate_changeset_id, NatsStore, Revision, KV_OP_TIMEOUT,
};
use crate::{
    error::{StoreError, StoreResult},
    recovery::{
        BatchJournalId, BatchRecovery, BatchRecoveryReport, JournalPhase, RecoveryAction,
        RecoveryMode, RecoveryPolicy, RecoveryResult,
    },
    store::{
        ChangeKind, ChangesetOp, ChangesetRecord, ChangesetRef, ChangesetValidation, Store,
        UnvalidatedReason, WriteContext,
    },
};

const JOURNAL_CHUNK_BYTES: usize = 256 * 1024;

#[derive(Clone, PartialEq, ::prost::Message)]
struct JournalHeader {
    #[prost(int32, tag = "1")]
    phase: i32,
    #[prost(string, tag = "2")]
    changeset_id: String,
    #[prost(string, tag = "3")]
    author: String,
    #[prost(string, tag = "4")]
    rpc: String,
    #[prost(int64, tag = "5")]
    updated_at_micros: i64,
    #[prost(uint32, tag = "6")]
    chunk_count: u32,
    #[prost(message, repeated, tag = "7")]
    changes: Vec<JournalChange>,
    /// Reserved for a writer lease to fence recovery with; always 0 until
    /// one exists, so entries written now decode unchanged afterwards.
    #[prost(uint64, tag = "8")]
    writer_epoch: u64,
    /// Key of the operation receipt this batch claims, in the
    /// `trogon-atlas-operations` bucket. Empty when the mutation carried no
    /// `operation_id`. Lets a recovery pass settle a receipt left `Pending`
    /// by a crash, without the original RPC caller present to do it.
    #[prost(string, tag = "9")]
    operation_key: String,
    /// The `operation_id` behind `operation_key`, so a changeset completed
    /// by recovery still names the operation that produced it.
    #[prost(string, tag = "10")]
    operation_id: String,
    /// The caller settles the receipt only after work that follows this
    /// batch (registering protections). Recovery cannot see that work, so
    /// it settles such a receipt not-applied and lets a retry finish it.
    #[prost(bool, tag = "11")]
    settlement_deferred: bool,
    #[prost(uint32, tag = "12")]
    format_version: u32,
    #[prost(string, tag = "13")]
    branch: String,
    #[prost(bool, tag = "14")]
    maintenance: bool,
    #[prost(message, repeated, tag = "15")]
    cleanup: Vec<BranchCleanup>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub(super) struct BranchCleanup {
    #[prost(string, tag = "1")]
    pub(super) key: String,
    #[prost(uint64, tag = "2")]
    pub(super) revision: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub(super) struct JournalChange {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(int32, tag = "2")]
    kind: i32,
    #[prost(message, optional, tag = "3")]
    id: Option<Id>,
    #[prost(bool, tag = "4")]
    deleted: bool,
    #[prost(uint64, tag = "5")]
    etag: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub(super) struct JournalPrior {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(uint64, optional, tag = "2")]
    revision: Option<u64>,
    #[prost(message, optional, tag = "3")]
    entity: Option<Entity>,
    #[prost(bytes = "bytes", optional, tag = "4")]
    raw_payload: Option<Bytes>,
    #[prost(uint32, tag = "5")]
    bucket: u32,
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct JournalChunk {
    #[prost(message, repeated, tag = "1")]
    priors: Vec<JournalPrior>,
}

impl JournalPrior {
    pub(super) fn new(key: String, revision: Option<u64>, entity: Option<Entity>) -> Self {
        Self {
            key,
            revision,
            entity,
            raw_payload: None,
            bucket: 0,
        }
    }

    pub(super) fn branch(key: String, entry: Option<KvEntry>, before: Option<Entity>) -> Self {
        let entry = live_entry(entry);
        let entity = if entry.is_none() { before } else { None };
        Self {
            key,
            revision: entry.as_ref().map(|entry| entry.revision),
            entity,
            raw_payload: entry.map(|entry| entry.value),
            bucket: 1,
        }
    }

    /// True when the key no longer holds this prior image. A key that was
    /// written and then restored to the same image counts as unchanged, so
    /// undoing a batch twice never rewrites a key the first undo restored.
    fn differs_from(&self, current: Option<KvEntry>) -> bool {
        if current.as_ref().map(|entry| entry.revision) == self.revision {
            return false;
        }
        if self.bucket == 1 {
            return match (self.raw_payload.as_ref(), live_entry(current)) {
                (None, None) => false,
                (Some(prior), Some(live)) => prior.as_ref() != live.value.as_ref(),
                _ => true,
            };
        }
        match (self.entity.as_ref(), live_entry(current)) {
            (None, None) => false,
            (Some(prior), Some(live)) => decode_entity(&live.value).ok().as_ref() != Some(prior),
            _ => true,
        }
    }

    fn before(&self) -> Result<Option<Entity>, String> {
        if self.bucket == 1 {
            if let Some(bytes) = &self.raw_payload {
                let delta = super::decode_branch_delta(bytes).map_err(|error| error.to_string())?;
                return Ok((!delta.tombstone).then_some(delta.ours).flatten());
            }
        }
        Ok(self.entity.clone())
    }
}

impl JournalChange {
    pub(super) fn landed(key: String, revision: &Revision, etag: u64) -> Self {
        Self {
            key,
            kind: revision.kind as i32,
            id: Some(revision.id.clone()),
            deleted: matches!(revision.change, ChangeKind::Delete),
            etag,
        }
    }

    fn change_kind(&self) -> ChangeKind {
        if self.deleted {
            ChangeKind::Delete
        } else {
            ChangeKind::Put
        }
    }

    fn entity_kind(&self) -> Option<EntityKind> {
        EntityKind::try_from(self.kind).ok()
    }

    fn entity_ref(&self) -> EntityRef {
        EntityRef {
            kind: self.kind,
            id: self.id.clone(),
        }
    }
}

fn phase_code(phase: JournalPhase, extended: bool) -> i32 {
    let code = match phase {
        JournalPhase::Planned => 1,
        JournalPhase::Aborted => 2,
        JournalPhase::Committed => 3,
        JournalPhase::Published => 4,
    };
    // Old readers reject unknown phases before loading any prior images.
    code + if extended { 100 } else { 0 }
}

fn phase_of(code: i32) -> Option<JournalPhase> {
    match code {
        1 | 101 => Some(JournalPhase::Planned),
        2 | 102 => Some(JournalPhase::Aborted),
        3 | 103 => Some(JournalPhase::Committed),
        4 | 104 => Some(JournalPhase::Published),
        _ => None,
    }
}

impl JournalHeader {
    fn branch(&self) -> Option<&str> {
        (!self.branch.is_empty()).then_some(self.branch.as_str())
    }

    fn format_supported(&self) -> bool {
        match self.format_version {
            0 => {
                (1..=4).contains(&self.phase)
                    && self.branch.is_empty()
                    && !self.maintenance
                    && self.cleanup.is_empty()
            }
            1 => (101..=104).contains(&self.phase),
            _ => false,
        }
    }

    fn attribution(&self) -> Option<ChangesetRef<'_>> {
        if self.changeset_id.is_empty() {
            return None;
        }
        Some(ChangesetRef {
            id: &self.changeset_id,
            author: &self.author,
            rpc: &self.rpc,
            operation_id: (!self.operation_id.is_empty()).then_some(self.operation_id.as_str()),
        })
    }
}

fn validate_priors(header: &JournalHeader, priors: &[JournalPrior]) -> Result<(), String> {
    for prior in priors {
        let valid = match prior.bucket {
            0 => {
                header.branch().is_none()
                    && prior.raw_payload.is_none()
                    && (prior.entity.is_none() || prior.revision.is_some())
                    && crate::key::parse_entity_key(&prior.key).is_some()
            }
            1 => {
                header.format_version == 1
                    && prior.revision.is_some() == prior.raw_payload.is_some()
                    && (prior.raw_payload.is_none() || prior.entity.is_none())
                    && header.branch().is_some_and(|branch| {
                        crate::key::parse_branch_delta_key(branch, &prior.key).is_some()
                            || (header.maintenance
                                && prior.key == crate::key::branch_meta_key(branch))
                    })
            }
            _ => false,
        };
        if !valid {
            return Err("journal contains invalid prior routing".into());
        }
    }
    Ok(())
}

fn chunk_key(id: &BatchJournalId, index: u32) -> String {
    format!("{id}.p.{index}")
}

pub(super) fn notification_id(id: &BatchJournalId, index: usize) -> String {
    format!("{id}.{index}")
}

/// Journal ids owned by a batch or a recovery pass running in this process.
#[derive(Default)]
pub(super) struct InflightJournals(parking_lot::Mutex<BTreeSet<String>>);

pub(super) struct InflightGuard {
    owner: Arc<InflightJournals>,
    id: String,
}

impl InflightJournals {
    fn claim(self: &Arc<Self>, id: &BatchJournalId) -> Option<InflightGuard> {
        let mut held = self.0.lock();
        if !held.insert(id.as_str().to_owned()) {
            return None;
        }
        Some(InflightGuard {
            owner: self.clone(),
            id: id.as_str().to_owned(),
        })
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.owner.0.lock().remove(&self.id);
    }
}

/// Failures a test can inject into the store to prove recovery converges.
#[derive(Debug, Clone, Default)]
pub(super) struct FaultPlan {
    pub(super) fail_write_at: Option<usize>,
    pub(super) crash_after_writes: Option<usize>,
    pub(super) crash_after_commit: bool,
    pub(super) fail_publish: bool,
    pub(super) fail_changeset_append: bool,
    pub(super) fail_rollback: bool,
    pub(super) fail_branch_post_write_read_at: Option<usize>,
    pub(super) fail_commit: bool,
    pub(super) crash_during_publish_finalize: bool,
    #[cfg(test)]
    pub(super) fail_revision_writes: usize,
    #[cfg(test)]
    pub(super) timeout_merge_write_at: Option<usize>,
    #[cfg(test)]
    pub(super) timed_out_merge_writes: usize,
}

pub(super) fn injected(point: &str) -> StoreError {
    StoreError::Unavailable(format!("injected failure: {point}"))
}

/// The durable record of one baseline batch in flight.
pub(super) struct Journal {
    kv: KvStore,
    id: BatchJournalId,
    header: JournalHeader,
    _inflight: InflightGuard,
    crash_during_publish_finalize: bool,
    fail_commit: bool,
}

impl Journal {
    pub(super) fn id(&self) -> &BatchJournalId {
        &self.id
    }

    async fn write_header(&mut self, phase: JournalPhase) -> StoreResult<()> {
        self.header.phase = phase_code(phase, self.header.format_version != 0);
        self.header.updated_at_micros = now_micros();
        let bytes = Bytes::from(self.header.encode_to_vec());
        tokio::time::timeout(KV_OP_TIMEOUT, self.kv.put(self.id.as_str(), bytes))
            .await
            .map_err(|_| StoreError::Backend(format!("journal {} put: timeout", self.id)))?
            .map_err(|e| StoreError::Backend(format!("journal {} put: {e}", self.id)))?;
        Ok(())
    }

    pub(super) async fn abort(&mut self) -> StoreResult<()> {
        self.write_header(JournalPhase::Aborted).await
    }

    pub(super) async fn commit(&mut self, changes: Vec<JournalChange>) -> StoreResult<()> {
        self.header.changes = changes;
        if self.fail_commit {
            return Err(injected("journal commit"));
        }
        self.write_header(JournalPhase::Committed).await
    }

    /// Everything but the changeset is written. An unattributed batch has
    /// no changeset to wait for, so it is done.
    pub(super) async fn published(mut self) -> StoreResult<()> {
        if self.header.attribution().is_none() {
            return self.finish().await;
        }
        self.header.chunk_count = 0;
        self.write_header(JournalPhase::Published).await?;
        if self.crash_during_publish_finalize {
            return Err(injected("publish finalization"));
        }
        purge_chunks(&self.kv, &self.id).await
    }

    pub(super) async fn finish(self) -> StoreResult<()> {
        purge_journal(&self.kv, &self.id).await
    }
}

async fn purge_subject(kv: &KvStore, subject: String) -> StoreResult<()> {
    tokio::time::timeout(KV_OP_TIMEOUT, kv.stream.purge().filter(subject.clone()))
        .await
        .map_err(|_| StoreError::Backend(format!("journal purge {subject}: timeout")))?
        .map_err(|e| StoreError::Backend(format!("journal purge {subject}: {e}")))?;
    Ok(())
}

async fn purge_chunks(kv: &KvStore, id: &BatchJournalId) -> StoreResult<()> {
    purge_subject(kv, format!("{}{id}.>", kv.prefix)).await
}

/// Remove a journal entry without leaving a delete marker behind, so a
/// bucket that sees one entry per batch does not grow without bound.
async fn purge_journal(kv: &KvStore, id: &BatchJournalId) -> StoreResult<()> {
    purge_chunks(kv, id).await?;
    purge_subject(kv, format!("{}{id}", kv.prefix)).await
}

fn pack(priors: Vec<JournalPrior>) -> Vec<JournalChunk> {
    let mut chunks: Vec<JournalChunk> = Vec::new();
    let mut current = JournalChunk::default();
    let mut size = 0usize;
    for prior in priors {
        let len = prior.encoded_len();
        if !current.priors.is_empty() && size + len > JOURNAL_CHUNK_BYTES {
            chunks.push(std::mem::take(&mut current));
            size = 0;
        }
        size += len;
        current.priors.push(prior);
    }
    if !current.priors.is_empty() {
        chunks.push(current);
    }
    chunks
}

async fn load_header(kv: &KvStore, id: &BatchJournalId) -> StoreResult<Option<JournalHeader>> {
    let Some(entry) = live_entry(kv_entry_timeout(kv, id.as_str().to_owned()).await?) else {
        return Ok(None);
    };
    JournalHeader::decode(entry.value.as_ref())
        .map(Some)
        .map_err(|e| StoreError::Backend(format!("decode journal {id}: {e}")))
}

/// `None` when a chunk is missing, which only happens when the batch
/// stopped while its journal was still being written, before any entity
/// write.
async fn load_priors(
    kv: &KvStore,
    id: &BatchJournalId,
    chunk_count: u32,
) -> StoreResult<Option<Vec<JournalPrior>>> {
    let mut priors = Vec::new();
    for index in 0..chunk_count {
        let key = chunk_key(id, index);
        let Some(entry) = live_entry(kv_entry_timeout(kv, key.clone()).await?) else {
            return Ok(None);
        };
        let chunk = JournalChunk::decode(entry.value.as_ref())
            .map_err(|e| StoreError::Backend(format!("decode journal chunk {key}: {e}")))?;
        priors.extend(chunk.priors);
    }
    if priors
        .iter()
        .any(|prior| prior.bucket > 1 || (prior.bucket == 0 && prior.raw_payload.is_some()))
    {
        return Err(StoreError::Backend(
            "unsupported batch journal prior format".into(),
        ));
    }
    Ok(Some(priors))
}

async fn list_journal_ids(kv: &KvStore) -> StoreResult<Vec<BatchJournalId>> {
    let mut keys = tokio::time::timeout(KV_OP_TIMEOUT, kv.keys())
        .await
        .map_err(|_| StoreError::Backend("journal kv.keys: timeout".into()))?
        .map_err(|e| StoreError::Backend(format!("journal kv.keys: {e}")))?;
    let mut ids = Vec::new();
    loop {
        let next = tokio::time::timeout(KV_OP_TIMEOUT, keys.next())
            .await
            .map_err(|_| StoreError::Backend("journal kv.keys stream: timeout".into()))?;
        let Some(item) = next else { break };
        let key = item.map_err(|e| StoreError::Backend(format!("journal kv.keys stream: {e}")))?;
        if !key.contains('.') {
            ids.push(BatchJournalId::new(key));
        }
    }
    ids.sort();
    Ok(ids)
}

impl NatsStore {
    pub(super) async fn journal_writes_ready(&self) -> StoreResult<()> {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let info = self
                .batches_kv
                .stream
                .get_info()
                .await
                .map_err(|_| StoreError::Unavailable("batch journal unavailable".into()))?;
            if info.state.messages == 0 {
                return Ok(());
            }
            let mut keys = self
                .batches_kv
                .keys()
                .await
                .map_err(|_| StoreError::Unavailable("batch journal unavailable".into()))?;
            while let Some(key) = keys.next().await {
                let key =
                    key.map_err(|_| StoreError::Unavailable("batch journal unavailable".into()))?;
                if key.contains('.') {
                    continue;
                }
                let id = BatchJournalId::new(key);
                let Some(header) = load_header(&self.batches_kv, &id).await? else {
                    continue;
                };
                if !header.format_supported()
                    || matches!(
                        phase_of(header.phase),
                        Some(JournalPhase::Planned | JournalPhase::Aborted) | None
                    )
                    || (phase_of(header.phase) == Some(JournalPhase::Committed)
                        && !header.cleanup.is_empty())
                {
                    return Err(StoreError::Unavailable(
                        "model writes require batch recovery".into(),
                    ));
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| StoreError::Unavailable("batch journal probe timed out".into()))?
    }

    #[cfg(test)]
    pub(super) fn faults(&self) -> FaultPlan {
        self.faults.lock().clone()
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)]
    pub(super) fn faults(&self) -> FaultPlan {
        FaultPlan::default()
    }

    /// Record the prior image of every key a baseline batch touches before
    /// the batch writes any of them.
    pub(super) async fn begin_journal(
        &self,
        ctx: WriteContext<'_>,
        priors: Vec<JournalPrior>,
    ) -> StoreResult<Journal> {
        self.begin_scoped_journal(ctx, priors, false, Vec::new())
            .await
    }

    pub(super) async fn begin_scoped_journal(
        &self,
        ctx: WriteContext<'_>,
        priors: Vec<JournalPrior>,
        maintenance: bool,
        cleanup: Vec<BranchCleanup>,
    ) -> StoreResult<Journal> {
        let attribution = ctx
            .changeset()
            .filter(|changeset| validate_changeset_id(changeset.id).is_ok());
        let id = BatchJournalId::new(
            attribution.map_or_else(|| uuid::Uuid::now_v7().to_string(), |c| c.id.to_owned()),
        );
        let inflight = self.inflight_journals.claim(&id).ok_or_else(|| {
            StoreError::Unavailable(format!("batch journal {id} is already in flight"))
        })?;
        let extended = ctx.branch_name().is_some()
            || maintenance
            || !cleanup.is_empty()
            || priors
                .iter()
                .any(|prior| prior.bucket != 0 || prior.raw_payload.is_some());
        let chunks = pack(priors);
        let header = JournalHeader {
            phase: phase_code(JournalPhase::Planned, extended),
            changeset_id: attribution.map(|c| c.id.to_owned()).unwrap_or_default(),
            author: attribution.map(|c| c.author.to_owned()).unwrap_or_default(),
            rpc: attribution.map(|c| c.rpc.to_owned()).unwrap_or_default(),
            updated_at_micros: now_micros(),
            chunk_count: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
            changes: Vec::new(),
            writer_epoch: self.lease.current_epoch().get(),
            operation_key: ctx.operation_key().unwrap_or_default().to_owned(),
            operation_id: attribution
                .and_then(|c| c.operation_id)
                .unwrap_or_default()
                .to_owned(),
            settlement_deferred: ctx.settlement_deferred(),
            format_version: u32::from(extended),
            branch: ctx.branch_name().unwrap_or_default().into(),
            maintenance,
            cleanup,
        };
        let created = tokio::time::timeout(
            KV_OP_TIMEOUT,
            self.batches_kv
                .create(id.as_str(), Bytes::from(header.encode_to_vec())),
        )
        .await
        .map_err(|_| StoreError::Backend(format!("journal {id} create: timeout")))?;
        if let Err(e) = created {
            use async_nats::jetstream::kv::CreateErrorKind;
            if matches!(e.kind(), CreateErrorKind::AlreadyExists) {
                return Err(StoreError::Unavailable(format!(
                    "batch journal {id} is unresolved from an earlier attempt; \
                     run batch recovery before retrying"
                )));
            }
            return Err(StoreError::Backend(format!("journal {id} create: {e}")));
        }
        let journal = Journal {
            kv: self.batches_kv.clone(),
            id,
            header,
            _inflight: inflight,
            crash_during_publish_finalize: self.faults().crash_during_publish_finalize,
            fail_commit: self.faults().fail_commit,
        };
        for (index, chunk) in (0u32..).zip(chunks.iter()) {
            let key = chunk_key(&journal.id, index);
            let written = tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.batches_kv
                    .put(&key, Bytes::from(chunk.encode_to_vec())),
            )
            .await
            .map_err(|_| StoreError::Backend(format!("journal chunk {key}: timeout")))
            .and_then(|r| {
                r.map(|_| ())
                    .map_err(|e| StoreError::Backend(format!("journal chunk {key}: {e}")))
            });
            if let Err(e) = written {
                if let Err(purge) = purge_journal(&self.batches_kv, &journal.id).await {
                    tracing::error!(
                        journal = %journal.id,
                        error = %purge,
                        "journal purge after a failed chunk write failed; recovery will discard it"
                    );
                }
                return Err(e);
            }
        }
        Ok(journal)
    }

    /// Undo a batch that was never reported as applied: restore `undo`
    /// to its prior images and drop the journal. `Some` carries the
    /// `PartialApply` to report when a key could not be restored; the
    /// journal then stays for recovery to finish the undo.
    pub(super) async fn undo_batch(
        &self,
        mut journal: Journal,
        priors: &[JournalPrior],
        undo: &BTreeSet<String>,
    ) -> Option<StoreError> {
        if let Err(error) = journal.abort().await {
            tracing::warn!(journal = %journal.id, %error, "batch journal not marked aborted");
        }
        let undo: Vec<JournalPrior> = priors
            .iter()
            .filter(|prior| undo.contains(&prior.key))
            .cloned()
            .collect();
        let failed = self.roll_back_priors(&undo).await;
        if failed.is_empty() {
            if let Err(error) = self
                .purge_branch_journal_revisions(&journal.header, priors)
                .await
            {
                tracing::warn!(%error, "branch rollback history cleanup remains pending");
                return Some(StoreError::PartialApply {
                    keys: undo.iter().map(|prior| prior.key.clone()).collect(),
                    journal: Some(journal.id.clone()),
                });
            }
            let id = journal.id.clone();
            if let Err(error) = journal.finish().await {
                tracing::warn!(
                    journal = %id,
                    %error,
                    "rolled-back batch journal not released; recovery will release it"
                );
            }
            return None;
        }
        self.config.metrics.record_batch_pending_recovery();
        tracing::error!(
            journal = %journal.id,
            keys = ?failed,
            "batch partially applied; batch recovery will roll it back"
        );
        Some(StoreError::PartialApply {
            keys: failed,
            journal: Some(journal.id.clone()),
        })
    }

    /// Restore every key that no longer holds its prior image. Returns the
    /// keys that could not be restored.
    pub(super) async fn roll_back_priors(&self, priors: &[JournalPrior]) -> Vec<String> {
        if self.faults().fail_rollback {
            return priors.iter().map(|p| p.key.clone()).collect();
        }
        let mut failed = Vec::new();
        let mut moved = std::collections::BTreeMap::new();
        for prior in priors {
            let kv = if prior.bucket == 1 {
                &self.branches_kv
            } else {
                &self.kv
            };
            match kv_entry_timeout(kv, prior.key.clone()).await {
                Ok(entry) => {
                    if prior.bucket == 1 {
                        if prior.differs_from(entry.clone()) {
                            let restore = tokio::time::timeout(KV_OP_TIMEOUT, async {
                                match (prior.raw_payload.as_ref(), live_entry(entry)) {
                                    (Some(bytes), Some(current)) => kv
                                        .update(&prior.key, bytes.clone(), current.revision)
                                        .await
                                        .map(|_| ())
                                        .map_err(|error| error.to_string()),
                                    (Some(bytes), None) => kv
                                        .create(&prior.key, bytes.clone())
                                        .await
                                        .map(|_| ())
                                        .map_err(|error| error.to_string()),
                                    (None, Some(current)) => kv
                                        .delete_expect_revision(&prior.key, Some(current.revision))
                                        .await
                                        .map_err(|error| error.to_string()),
                                    (None, None) => Ok(()),
                                }
                            })
                            .await;
                            if !matches!(restore, Ok(Ok(()))) {
                                failed.push(prior.key.clone());
                            }
                        }
                    } else if prior.differs_from(entry) {
                        moved.insert(
                            prior.key.clone(),
                            prior
                                .entity
                                .clone()
                                .map(|entity| (entity, prior.revision.unwrap_or_default())),
                        );
                    }
                }
                Err(error) => {
                    tracing::error!(key = %prior.key, %error, "rollback: kv.entry failed");
                    failed.push(prior.key.clone());
                }
            }
        }
        failed.extend(rollback_baseline_snapshot(&moved, &self.kv).await);
        failed
    }

    async fn landed_keys(&self, priors: &[JournalPrior]) -> StoreResult<Vec<String>> {
        let mut landed = Vec::new();
        for prior in priors {
            let kv = if prior.bucket == 1 {
                &self.branches_kv
            } else {
                &self.kv
            };
            let entry = kv_entry_timeout(kv, prior.key.clone()).await?;
            if prior.differs_from(entry) {
                landed.push(prior.key.clone());
            }
        }
        Ok(landed)
    }

    async fn purge_branch_journal_revisions(
        &self,
        header: &JournalHeader,
        priors: &[JournalPrior],
    ) -> StoreResult<()> {
        let (Some(branch), Some(changeset)) = (header.branch(), header.attribution()) else {
            return Ok(());
        };
        if header.maintenance {
            return Ok(());
        }
        for prior in priors {
            let Some((kind, id)) = crate::key::parse_branch_delta_key(branch, &prior.key) else {
                continue;
            };
            let key = crate::key::revision_key(Some(branch), kind, &id, changeset.id);
            tokio::time::timeout(KV_OP_TIMEOUT, self.revisions_kv.delete(&key))
                .await
                .map_err(|_| StoreError::Unavailable("branch revision cleanup timed out".into()))?
                .map_err(|error| StoreError::Backend(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) async fn cleanup_journal_branches(
        &self,
        cleanup: &[BranchCleanup],
    ) -> StoreResult<()> {
        for item in cleanup {
            let Some(entry) =
                live_entry(kv_entry_timeout(&self.branches_kv, item.key.clone()).await?)
            else {
                continue;
            };
            if entry.revision != item.revision {
                continue;
            }
            tokio::time::timeout(
                KV_OP_TIMEOUT,
                self.branches_kv
                    .purge_expect_revision(&item.key, Some(item.revision)),
            )
            .await
            .map_err(|_| StoreError::Unavailable("branch cleanup timed out".into()))?
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) async fn publish_owned_journal(
        &self,
        journal: Journal,
        priors: &[JournalPrior],
    ) -> StoreResult<()> {
        self.publish_journaled_changes(
            &journal.id,
            journal.header.attribution(),
            &journal.header.changes,
            priors,
            journal.header.branch(),
        )
        .await
        .map_err(StoreError::Backend)?;
        self.cleanup_journal_branches(&journal.header.cleanup)
            .await?;
        journal.published().await
    }

    /// Publish every change and write every revision a committed batch
    /// produced. Returns the keys a later write already replaced.
    pub(super) async fn publish_journaled_changes(
        &self,
        id: &BatchJournalId,
        changeset: Option<ChangesetRef<'_>>,
        changes: &[JournalChange],
        priors: &[JournalPrior],
        branch: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut superseded = Vec::new();
        for (index, change) in changes.iter().enumerate().filter(|_| branch.is_none()) {
            self.publish_change(
                change.change_kind(),
                &change.entity_ref(),
                changeset,
                Some(&notification_id(id, index)),
            )
            .await
            .map_err(|e| format!("notification {index}: {e}"))?;
        }
        let Some(changeset) = changeset else {
            return Ok(superseded);
        };
        let mut last: std::collections::BTreeMap<&str, &JournalChange> =
            std::collections::BTreeMap::new();
        for change in changes {
            last.insert(change.key.as_str(), change);
        }
        for (key, change) in last {
            let (Some(kind), Some(entity_id)) = (change.entity_kind(), change.id.clone()) else {
                return Err(format!("journal change for {key} has no entity reference"));
            };
            let before = priors
                .iter()
                .find(|p| p.key == key)
                .map(JournalPrior::before)
                .transpose()?
                .flatten();
            let kv = if branch.is_some() {
                &self.branches_kv
            } else {
                &self.kv
            };
            let current = kv_entry_timeout(kv, key.to_owned())
                .await
                .map_err(|e| format!("read {key}: {e}"))?;
            let after = if change.deleted {
                let still_present = if branch.is_some() {
                    live_entry(current)
                        .map(|entry| super::decode_branch_delta(&entry.value))
                        .transpose()
                        .map_err(|error| error.to_string())?
                        .is_some_and(|delta| !delta.tombstone && delta.ours.is_some())
                } else {
                    live_entry(current).is_some()
                };
                if still_present {
                    superseded.push(key.to_owned());
                    continue;
                }
                None
            } else {
                match live_entry(current) {
                    Some(entry) if entry.revision == change.etag => {
                        if branch.is_some() {
                            super::decode_branch_delta(&entry.value)
                                .map_err(|error| error.to_string())?
                                .ours
                        } else {
                            Some(
                                decode_entity(&entry.value)
                                    .map_err(|e| format!("decode {key}: {e}"))?,
                            )
                        }
                    }
                    _ => {
                        superseded.push(key.to_owned());
                        continue;
                    }
                }
            };
            let revision = Revision {
                kind,
                id: entity_id,
                change: change.change_kind(),
                before,
                after,
            };
            let ctx = branch
                .map_or_else(WriteContext::baseline, WriteContext::branch)
                .attributed(changeset);
            self.try_record_revision(ctx, &revision)
                .await
                .map_err(|e| format!("revision {key}: {e}"))?;
        }
        Ok(superseded)
    }

    /// Drop the journal of a batch whose changeset just landed, once
    /// nothing else is outstanding for it.
    pub(super) async fn release_published_journal(&self, changeset_id: &str) {
        let id = BatchJournalId::new(changeset_id);
        let Some(_guard) = self.inflight_journals.claim(&id) else {
            return;
        };
        let released = async {
            match load_header(&self.batches_kv, &id).await? {
                Some(header)
                    if header.format_supported()
                        && phase_of(header.phase) == Some(JournalPhase::Published) =>
                {
                    purge_journal(&self.batches_kv, &id).await
                }
                _ => Ok(()),
            }
        }
        .await;
        if let Err(error) = released {
            tracing::warn!(
                journal = %id,
                %error,
                "changeset landed but its batch journal was not released; recovery will release it"
            );
        }
    }

    pub(super) async fn recover_journaled_batches(
        &self,
        policy: RecoveryPolicy,
    ) -> StoreResult<BatchRecoveryReport> {
        let mut report = BatchRecoveryReport::default();
        let min_age_micros = i64::try_from(policy.min_age.as_micros()).unwrap_or(i64::MAX);
        for id in list_journal_ids(&self.batches_kv).await? {
            let Some(_guard) = self.inflight_journals.claim(&id) else {
                report.skipped_in_flight += 1;
                continue;
            };
            let Some(header) = load_header(&self.batches_kv, &id).await? else {
                continue;
            };
            if now_micros().saturating_sub(header.updated_at_micros) < min_age_micros {
                report.skipped_recent += 1;
                continue;
            }
            report
                .batches
                .push(self.recover_batch(id, header, policy.mode).await);
        }
        Ok(report)
    }

    async fn recover_batch(
        &self,
        id: BatchJournalId,
        header: JournalHeader,
        mode: RecoveryMode,
    ) -> BatchRecovery {
        let Some(phase) = phase_of(header.phase).filter(|_| header.format_supported()) else {
            return BatchRecovery {
                journal: id,
                phase: JournalPhase::Planned,
                action: RecoveryAction::RollBack,
                result: RecoveryResult::Unresolved {
                    reason: format!("unknown journal phase {}", header.phase),
                },
                keys: Vec::new(),
                superseded: Vec::new(),
            };
        };
        let action = match phase {
            JournalPhase::Planned | JournalPhase::Aborted => RecoveryAction::RollBack,
            JournalPhase::Committed => RecoveryAction::RollForward,
            JournalPhase::Published => RecoveryAction::CompleteChangeset,
        };
        let mut batch = BatchRecovery {
            journal: id,
            phase,
            action,
            result: RecoveryResult::Pending,
            keys: Vec::new(),
            superseded: Vec::new(),
        };
        let outcome = match action {
            RecoveryAction::RollBack => self.roll_back_batch(&mut batch, &header, mode).await,
            RecoveryAction::RollForward => self.roll_forward_batch(&mut batch, &header, mode).await,
            RecoveryAction::CompleteChangeset => {
                batch.keys = changed_keys(&header.changes);
                match mode {
                    RecoveryMode::Report => Ok(()),
                    RecoveryMode::Repair => {
                        match self.cleanup_journal_branches(&header.cleanup).await {
                            Ok(()) => self.ensure_changeset(&header).await,
                            Err(error) => Err(error.to_string()),
                        }
                    }
                }
            }
        };
        if mode == RecoveryMode::Repair && outcome.is_ok() && !header.operation_key.is_empty() {
            let settled = match action {
                RecoveryAction::RollBack => {
                    self.settle_operation_not_applied(&header.operation_key)
                        .await
                }
                RecoveryAction::RollForward | RecoveryAction::CompleteChangeset
                    if header.settlement_deferred =>
                {
                    self.settle_pending_operation_unfinished(
                        &header.operation_key,
                        &header.changeset_id,
                    )
                    .await
                }
                RecoveryAction::RollForward | RecoveryAction::CompleteChangeset => {
                    self.settle_operation_applied(&header.operation_key, &header.changeset_id)
                        .await
                }
            };
            if let Err(error) = settled {
                // The receipt is a convenience on top of the changeset,
                // which is already durable at this point; a caller that
                // retries the same operation_id gets claimed again rather
                // than replayed, which is safe, just not idempotent once.
                tracing::warn!(
                    journal = %batch.journal,
                    operation_key = %header.operation_key,
                    %error,
                    "batch recovery converged but its operation receipt was not settled"
                );
            }
        }
        batch.result = match (mode, outcome) {
            (_, Err(reason)) => RecoveryResult::Unresolved { reason },
            (RecoveryMode::Report, Ok(())) => RecoveryResult::Pending,
            (RecoveryMode::Repair, Ok(())) => {
                match purge_journal(&self.batches_kv, &batch.journal).await {
                    Ok(()) => RecoveryResult::Converged,
                    Err(e) => RecoveryResult::Unresolved {
                        reason: format!("journal not released: {e}"),
                    },
                }
            }
        };
        match &batch.result {
            RecoveryResult::Unresolved { reason } => tracing::error!(
                journal = %batch.journal,
                phase = %batch.phase,
                action = batch.action.as_str(),
                reason = %reason,
                "batch recovery did not converge"
            ),
            RecoveryResult::Converged => tracing::warn!(
                journal = %batch.journal,
                phase = %batch.phase,
                action = batch.action.as_str(),
                keys = ?batch.keys,
                superseded = ?batch.superseded,
                "batch recovery converged an interrupted batch"
            ),
            RecoveryResult::Pending => {}
        }
        batch
    }

    async fn roll_back_batch(
        &self,
        batch: &mut BatchRecovery,
        header: &JournalHeader,
        mode: RecoveryMode,
    ) -> Result<(), String> {
        let Some(priors) = load_priors(&self.batches_kv, &batch.journal, header.chunk_count)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(());
        };
        validate_priors(header, &priors)?;
        batch.keys = self.landed_keys(&priors).await.map_err(|e| e.to_string())?;
        if mode == RecoveryMode::Report {
            return Ok(());
        }
        let moved: Vec<JournalPrior> = priors
            .iter()
            .filter(|prior| batch.keys.contains(&prior.key))
            .cloned()
            .collect();
        let failed = self.roll_back_priors(&moved).await;
        if failed.is_empty() {
            self.purge_branch_journal_revisions(header, &priors)
                .await
                .map_err(|error| error.to_string())
        } else {
            Err(format!("could not restore {failed:?}"))
        }
    }

    async fn roll_forward_batch(
        &self,
        batch: &mut BatchRecovery,
        header: &JournalHeader,
        mode: RecoveryMode,
    ) -> Result<(), String> {
        batch.keys = changed_keys(&header.changes);
        let priors = load_priors(&self.batches_kv, &batch.journal, header.chunk_count)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "committed journal is missing its prior images".to_owned())?;
        validate_priors(header, &priors)?;
        if mode == RecoveryMode::Report {
            return Ok(());
        }
        batch.superseded = self
            .publish_journaled_changes(
                &batch.journal,
                header.attribution(),
                &header.changes,
                &priors,
                header.branch(),
            )
            .await?;
        self.cleanup_journal_branches(&header.cleanup)
            .await
            .map_err(|error| error.to_string())?;
        self.ensure_changeset(header).await
    }

    /// Append the changeset of a batch that was reported as applied when
    /// the RPC that owned it never did.
    async fn ensure_changeset(&self, header: &JournalHeader) -> Result<(), String> {
        let Some(changeset) = header.attribution() else {
            return Ok(());
        };
        match self.get_changeset(changeset.id).await {
            Ok(_) => return Ok(()),
            Err(StoreError::NotFound) => {}
            Err(e) => return Err(format!("read changeset {}: {e}", changeset.id)),
        }
        let record = ChangesetRecord {
            id: changeset.id.to_owned(),
            author: changeset.author.to_owned(),
            at: now_rfc3339(),
            message: format!(
                "{}: {} change(s), completed by batch recovery",
                changeset.rpc,
                header.changes.len()
            ),
            rpc: changeset.rpc.to_owned(),
            branch: header.branch().map(str::to_owned),
            operation_id: changeset.operation_id.map(str::to_owned),
            validation: ChangesetValidation::Unvalidated(UnvalidatedReason::Recovered),
            ops: header
                .changes
                .iter()
                .map(|change| ChangesetOp {
                    kind: change.change_kind(),
                    entity_ref: change.entity_ref(),
                })
                .collect(),
        };
        self.append_changeset(&record)
            .await
            .map_err(|e| format!("append changeset {}: {e}", changeset.id))
    }
}

fn changed_keys(changes: &[JournalChange]) -> Vec<String> {
    let keys: BTreeSet<&str> = changes.iter().map(|c| c.key.as_str()).collect();
    keys.into_iter().map(str::to_owned).collect()
}

#[cfg(test)]
mod compatibility_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[derive(Clone, PartialEq, prost::Message)]
    struct LegacyPhase {
        #[prost(int32, tag = "1")]
        phase: i32,
    }

    #[test]
    fn extended_journal_phases_are_rejected_by_legacy_readers_before_priors() {
        for phase in [
            JournalPhase::Planned,
            JournalPhase::Aborted,
            JournalPhase::Committed,
            JournalPhase::Published,
        ] {
            let extended = JournalHeader {
                phase: phase_code(phase, true),
                format_version: 1,
                branch: "review".into(),
                ..Default::default()
            };
            let legacy = LegacyPhase::decode(extended.encode_to_vec().as_slice()).unwrap();
            let legacy_action = match legacy.phase {
                1 => Some(JournalPhase::Planned),
                2 => Some(JournalPhase::Aborted),
                3 => Some(JournalPhase::Committed),
                4 => Some(JournalPhase::Published),
                _ => None,
            };
            assert_eq!(legacy_action, None);
            assert_eq!(phase_of(extended.phase), Some(phase));
            assert!(extended.format_supported());
            let baseline = JournalHeader {
                phase: phase_code(phase, false),
                ..Default::default()
            };
            assert!(baseline.format_supported());
            assert_eq!(
                LegacyPhase::decode(baseline.encode_to_vec().as_slice())
                    .unwrap()
                    .phase,
                phase_code(phase, false)
            );
        }
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_headers_and_invalid_prior_routes_remain_unresolved_without_writes() {
        for (phase, version, bucket) in [(999, 2, 0), (1, 0, 1), (101, 1, 9)] {
            let store = super::super::recovery_tests::isolated_store().await;
            let id = BatchJournalId::new(uuid::Uuid::now_v7().to_string());
            let header = JournalHeader {
                phase,
                format_version: version,
                chunk_count: 1,
                ..Default::default()
            };
            let chunk = JournalChunk {
                priors: vec![JournalPrior {
                    key: "event.recovery.untouched.1".into(),
                    bucket,
                    ..Default::default()
                }],
            };
            store
                .batches_kv
                .put(id.as_str(), Bytes::from(header.encode_to_vec()))
                .await
                .unwrap();
            store
                .batches_kv
                .put(chunk_key(&id, 0), Bytes::from(chunk.encode_to_vec()))
                .await
                .unwrap();
            for policy in [RecoveryPolicy::report_all(), RecoveryPolicy::repair_all()] {
                let report = store.recover_batches(policy).await.unwrap();
                assert_eq!(report.batches.len(), 1);
                assert!(matches!(
                    report.batches[0].result,
                    RecoveryResult::Unresolved { .. }
                ));
                assert!(
                    kv_entry_timeout(&store.kv, "event.recovery.untouched.1".into())
                        .await
                        .unwrap()
                        .is_none()
                );
                assert!(kv_entry_timeout(&store.batches_kv, id.as_str().into())
                    .await
                    .unwrap()
                    .is_some());
                assert!(matches!(
                    store.read_marker().await,
                    Err(StoreError::Unavailable(_))
                ));
            }
        }
    }
}
