//! Proves `recover_at_startup` and `spawn_sweep` actually consult the
//! writer lease rather than always recovering. See
//! `docs/explanation/single-writer.md` for why a rolling deploy needs
//! this: only the lease holder may safely repair a batch it might still
//! be mid-write on.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::sync::broadcast;
use trogon_atlas_proto as pb;
use trogon_atlas_server::batch_recovery::{recover_at_startup, spawn_sweep, SWEEP_INTERVAL};
use trogon_atlas_store::{
    store::{
        BranchInfo, ChangeRecord, ChangeStreamStats, ListFilter, MutationOp, MutationOutcome,
        Store, Written,
    },
    BatchRecoveryReport, ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef, NatsStore,
    RecoveryPolicy, StoredEntity, WriteContext,
};

/// Wraps a real `NatsStore` but makes `is_writer` a test-controlled flag
/// and counts every `recover_batches` call, so a test can tell whether a
/// gate let a pass through without caring what the (empty) pass found.
struct GatedRecoveryStore {
    inner: NatsStore,
    writer: AtomicBool,
    recover_calls: Arc<AtomicUsize>,
}

impl GatedRecoveryStore {
    async fn new(is_writer: bool) -> Self {
        let nats = trogon_atlas_testsupport::shared().await;
        Self {
            inner: nats.store().await,
            writer: AtomicBool::new(is_writer),
            recover_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Store for GatedRecoveryStore {
    fn is_writer(&self) -> bool {
        self.writer.load(Ordering::Acquire)
    }

    async fn recover_batches(
        &self,
        policy: RecoveryPolicy,
    ) -> trogon_atlas_store::StoreResult<BatchRecoveryReport> {
        self.recover_calls.fetch_add(1, Ordering::AcqRel);
        self.inner.recover_batches(policy).await
    }

    async fn register_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        created_by: &str,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner
            .register_namespace(name, parent, created_by)
            .await
    }

    async fn adopt_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        tenure: &trogon_atlas_store::NamespaceTenure,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner.adopt_namespace(name, parent, tenure).await
    }

    async fn restore_namespace(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        self.inner.restore_namespace(record).await
    }

    async fn release_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.release_namespace(id).await
    }

    async fn promote_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        self.inner.promote_namespace(id).await
    }

    async fn get_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        self.inner.get_namespace(id).await
    }

    async fn resolve_namespace(
        &self,
        parent: &trogon_atlas_core::OwnerId,
        name: &trogon_atlas_core::NamespaceName,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_core::NamespaceId>> {
        self.inner.resolve_namespace(parent, name).await
    }

    async fn list_namespaces(
        &self,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::NamespaceRecord>> {
        self.inner.list_namespaces().await
    }

    async fn move_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
        new_parent: &trogon_atlas_core::OwnerId,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceRecord> {
        self.inner.move_namespace(id, new_parent).await
    }

    async fn get(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<StoredEntity> {
        self.inner.get(kind, id, branch).await
    }

    async fn batch_get(
        &self,
        keys: &[(pb::EntityKind, pb::Id)],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<Option<StoredEntity>>> {
        self.inner.batch_get(keys, branch).await
    }

    async fn create(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner.create(kind, entity, ctx).await
    }

    async fn put(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner.put(kind, entity, force, ctx).await
    }

    async fn update(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        expected_etag: &str,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        self.inner
            .update(kind, entity, expected_etag, force, ctx)
            .await
    }

    async fn delete(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        expected_etag: Option<&str>,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete(kind, id, expected_etag, ctx).await
    }

    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        self.inner.list(filter, limit, branch).await
    }

    async fn record_change(
        &self,
        kind: ChangeKind,
        entity_ref: &pb::EntityRef,
        changeset: Option<ChangesetRef<'_>>,
    ) -> trogon_atlas_store::StoreResult<u64> {
        self.inner.record_change(kind, entity_ref, changeset).await
    }

    async fn read_changes(
        &self,
        since: u64,
        limit: usize,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangeRecord>> {
        self.inner.read_changes(since, limit).await
    }

    async fn current_change_seq(&self) -> trogon_atlas_store::StoreResult<u64> {
        self.inner.current_change_seq().await
    }

    async fn batch_apply(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        self.inner.batch_apply(ops, ctx).await
    }

    fn subscribe(&self) -> broadcast::Receiver<ChangeRecord> {
        self.inner.subscribe()
    }

    async fn prune_changes(&self, before_seq: u64) -> trogon_atlas_store::StoreResult<()> {
        self.inner.prune_changes(before_seq).await
    }

    async fn change_stream_stats(
        &self,
    ) -> trogon_atlas_store::StoreResult<Option<ChangeStreamStats>> {
        self.inner.change_stream_stats().await
    }

    async fn create_branch(
        &self,
        name: &str,
        doc: &str,
    ) -> trogon_atlas_store::StoreResult<BranchInfo> {
        self.inner.create_branch(name, doc).await
    }

    async fn set_branch_fork_point(
        &self,
        name: &str,
        fork_changeset_id: &str,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner
            .set_branch_fork_point(name, fork_changeset_id)
            .await
    }

    async fn list_branches(&self) -> trogon_atlas_store::StoreResult<Vec<BranchInfo>> {
        self.inner.list_branches().await
    }

    async fn delete_branch(&self, name: &str) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete_branch(name).await
    }

    async fn list_branch_deltas(
        &self,
        branch: &str,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::store::BranchDeltaEntry>> {
        self.inner.list_branch_deltas(branch).await
    }

    async fn land_branch_merge(
        &self,
        branch: &str,
        ops: &[trogon_atlas_store::store::BranchLandOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        self.inner.land_branch_merge(branch, ops, ctx).await
    }

    async fn rebase_branch_deltas(
        &self,
        branch: &str,
        rebases: &[(pb::EntityKind, pb::Id, Option<(pb::Entity, String)>)],
    ) -> trogon_atlas_store::StoreResult<u32> {
        self.inner.rebase_branch_deltas(branch, rebases).await
    }

    async fn resolve_branch_entry(
        &self,
        branch: &str,
        kind: pb::EntityKind,
        id: &pb::Id,
        take_theirs: bool,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner
            .resolve_branch_entry(branch, kind, id, take_theirs)
            .await
    }

    async fn append_changeset(
        &self,
        record: &ChangesetRecord,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.append_changeset(record).await
    }

    async fn get_changeset(&self, id: &str) -> trogon_atlas_store::StoreResult<ChangesetRecord> {
        self.inner.get_changeset(id).await
    }

    async fn list_changesets(
        &self,
        page: ChangesetPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangesetRecord>> {
        self.inner.list_changesets(page).await
    }

    async fn list_entity_revisions(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        page: trogon_atlas_store::RevisionPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::EntityRevisionRecord>> {
        self.inner.list_entity_revisions(kind, id, page).await
    }

    async fn get_entity_revision(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        changeset_id: &str,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::EntityRevisionRecord> {
        self.inner
            .get_entity_revision(kind, id, changeset_id, branch)
            .await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn recover_at_startup_skips_a_non_writer() {
    let store = GatedRecoveryStore::new(false).await;
    let report = recover_at_startup(&store).await;
    assert_eq!(store.recover_calls.load(Ordering::Acquire), 0);
    assert_eq!(report, BatchRecoveryReport::default());
}

#[tokio::test(flavor = "multi_thread")]
async fn recover_at_startup_runs_on_the_writer() {
    let store = GatedRecoveryStore::new(true).await;
    recover_at_startup(&store).await;
    assert_eq!(store.recover_calls.load(Ordering::Acquire), 1);
}

/// `recover_batches` is real NATS I/O, not a virtual-time wait, so a single
/// `yield_now` after `tokio::time::advance` is not enough to guarantee it has
/// run to completion. Yield repeatedly to flush any in-flight call before an
/// assertion depends on its side effect having (or not having) happened.
async fn settle() {
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn sweep_skips_ticks_while_not_the_writer_and_resumes_once_it_is() {
    // The shared NATS testcontainer needs real wall-clock time to start, so
    // time is paused only once the store (and its container) already exist
    // rather than for the whole test via `#[tokio::test(start_paused = true)]`.
    let store = Arc::new(GatedRecoveryStore::new(false).await);
    tokio::time::pause();
    let recover_calls = store.recover_calls.clone();
    let _sweep = spawn_sweep(store.clone());
    // Let the sweep task run its first (immediate) tick and register its
    // real next-tick timer before the clock jumps, or that registration
    // happens post-jump and the interval silently re-anchors to "now".
    settle().await;

    tokio::time::advance(SWEEP_INTERVAL * 3 + std::time::Duration::from_secs(1)).await;
    settle().await;
    assert_eq!(
        recover_calls.load(Ordering::Acquire),
        0,
        "a non-writer must never run the sweep"
    );

    store.writer.store(true, Ordering::Release);
    tokio::time::advance(SWEEP_INTERVAL + std::time::Duration::from_secs(1)).await;
    settle().await;
    assert!(
        recover_calls.load(Ordering::Acquire) >= 1,
        "the writer must resume the sweep once it holds the lease"
    );
}
