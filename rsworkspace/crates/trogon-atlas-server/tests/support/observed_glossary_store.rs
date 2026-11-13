use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::broadcast;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{
        BranchInfo, ChangeRecord, ChangeStreamStats, ListFilter, MutationOp, MutationOutcome,
        Store, Written,
    },
    ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef, NatsStore, StoredEntity,
    WriteContext,
};

pub struct ObservedGlossaryStore {
    pub inner: NatsStore,
    pub source_reads: AtomicUsize,
    pub pause_source: AtomicBool,
    pub reached: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}
impl ObservedGlossaryStore {
    pub fn new(inner: NatsStore) -> Self {
        Self {
            inner,
            source_reads: AtomicUsize::new(0),
            pause_source: AtomicBool::new(false),
            reached: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl Store for ObservedGlossaryStore {
    async fn read_marker(
        &self,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::StoreReadMarker> {
        self.inner.read_marker().await
    }

    fn is_writer(&self) -> bool {
        self.inner.is_writer()
    }

    fn writer_status(&self) -> trogon_atlas_store::store::WriterStatus {
        self.inner.writer_status()
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
    async fn delete_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceRecord> {
        self.inner.delete_namespace(id).await
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

    async fn get_protection(
        &self,
        identity: &trogon_atlas_store::EntityIdentity,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::ProtectionState> {
        self.inner.get_protection(identity).await
    }

    async fn put_protection(
        &self,
        record: &trogon_atlas_store::ProtectionRecord,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::ProtectionRevision> {
        self.inner.put_protection(record).await
    }

    async fn delete_protection(
        &self,
        identity: &trogon_atlas_store::EntityIdentity,
        expected: trogon_atlas_store::ProtectionRevision,
    ) -> trogon_atlas_store::StoreResult<()> {
        self.inner.delete_protection(identity, expected).await
    }

    async fn list_protections(
        &self,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::ProtectionListing> {
        self.inner.list_protections().await
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
        if kind == pb::EntityKind::Document {
            self.source_reads.fetch_add(1, Ordering::SeqCst);
            if self.pause_source.swap(false, Ordering::SeqCst) {
                self.reached.notify_one();
                self.release.notified().await;
            }
        }
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

    async fn referenced_targets(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        self.inner.referenced_targets(targets, branch).await
    }

    async fn referenced_targets_with_overlay(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
        overlay: &[(pb::EntityKey, Option<pb::Entity>)],
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        self.inner
            .referenced_targets_with_overlay(targets, branch, overlay)
            .await
    }

    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        self.inner.list(filter, limit, branch).await
    }

    async fn list_bounded(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
        budget: trogon_atlas_store::EntityByteBudget,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        self.inner.list_bounded(filter, limit, branch, budget).await
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
