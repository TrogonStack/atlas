#![allow(clippy::unwrap_used, clippy::expect_used, unused_variables)]
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use tokio::sync::broadcast;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};
use trogon_atlas_store::{
    source::{SourceDocument, SourceDocumentKey, SourceDocumentPage},
    store::{
        BranchInfo, ChangeRecord, ChangeStreamStats, ListFilter, MutationOp, MutationOutcome,
        Store, Written,
    },
    ChangeKind, ChangesetPage, ChangesetRecord, ChangesetRef, StoreError, StoreResult,
    StoredEntity, WriteContext,
};

struct SweepStore {
    visits: Arc<Mutex<Vec<String>>>,
    slow: bool,
}
#[async_trait]
impl Store for SweepStore {
    async fn register_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        created_by: &str,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        panic!("unexpected unrelated Store API")
    }
    async fn adopt_namespace(
        &self,
        name: &trogon_atlas_core::NamespaceName,
        parent: &trogon_atlas_core::OwnerId,
        tenure: &trogon_atlas_store::NamespaceTenure,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        panic!("unexpected unrelated Store API")
    }
    async fn restore_namespace(
        &self,
        record: &trogon_atlas_store::NamespaceRecord,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceClaim> {
        panic!("unexpected unrelated Store API")
    }
    async fn release_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn promote_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        panic!("unexpected unrelated Store API")
    }
    async fn get_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_store::NamespaceRecord>> {
        panic!("unexpected unrelated Store API")
    }
    async fn resolve_namespace(
        &self,
        parent: &trogon_atlas_core::OwnerId,
        name: &trogon_atlas_core::NamespaceName,
    ) -> trogon_atlas_store::StoreResult<Option<trogon_atlas_core::NamespaceId>> {
        panic!("unexpected unrelated Store API")
    }
    async fn list_namespaces(
        &self,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::NamespaceRecord>> {
        panic!("unexpected unrelated Store API")
    }
    async fn move_namespace(
        &self,
        id: &trogon_atlas_core::NamespaceId,
        new_parent: &trogon_atlas_core::OwnerId,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::NamespaceRecord> {
        panic!("unexpected unrelated Store API")
    }
    async fn get(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<StoredEntity> {
        panic!("unexpected unrelated Store API")
    }
    async fn batch_get(
        &self,
        keys: &[(pb::EntityKind, pb::Id)],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<Option<StoredEntity>>> {
        panic!("unexpected unrelated Store API")
    }
    async fn create(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        panic!("unexpected unrelated Store API")
    }
    async fn put(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        panic!("unexpected unrelated Store API")
    }
    async fn update(
        &self,
        kind: pb::EntityKind,
        entity: &pb::Entity,
        expected_etag: &str,
        force: bool,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Written> {
        panic!("unexpected unrelated Store API")
    }
    async fn delete(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        expected_etag: Option<&str>,
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn referenced_targets(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        panic!("unexpected unrelated Store API")
    }
    async fn referenced_targets_with_overlay(
        &self,
        targets: &[pb::EntityKey],
        branch: Option<&str>,
        overlay: &[(pb::EntityKey, Option<pb::Entity>)],
    ) -> trogon_atlas_store::StoreResult<std::collections::HashSet<pb::EntityKey>> {
        panic!("unexpected unrelated Store API")
    }
    async fn list(
        &self,
        filter: ListFilter<'_>,
        limit: Option<usize>,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<Vec<StoredEntity>> {
        panic!("unexpected unrelated Store API")
    }
    async fn record_change(
        &self,
        kind: ChangeKind,
        entity_ref: &pb::EntityRef,
        changeset: Option<ChangesetRef<'_>>,
    ) -> trogon_atlas_store::StoreResult<u64> {
        panic!("unexpected unrelated Store API")
    }
    async fn read_changes(
        &self,
        since: u64,
        limit: usize,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangeRecord>> {
        panic!("unexpected unrelated Store API")
    }
    async fn current_change_seq(&self) -> trogon_atlas_store::StoreResult<u64> {
        panic!("unexpected unrelated Store API")
    }
    async fn batch_apply(
        &self,
        ops: &[MutationOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        panic!("unexpected unrelated Store API")
    }
    fn subscribe(&self) -> broadcast::Receiver<ChangeRecord> {
        let (tx, _) = broadcast::channel(1);
        tx.subscribe()
    }
    async fn prune_changes(&self, before_seq: u64) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn change_stream_stats(
        &self,
    ) -> trogon_atlas_store::StoreResult<Option<ChangeStreamStats>> {
        panic!("unexpected unrelated Store API")
    }
    async fn create_branch(
        &self,
        name: &str,
        doc: &str,
    ) -> trogon_atlas_store::StoreResult<BranchInfo> {
        panic!("unexpected unrelated Store API")
    }
    async fn set_branch_fork_point(
        &self,
        name: &str,
        fork_changeset_id: &str,
    ) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn list_branches(&self) -> trogon_atlas_store::StoreResult<Vec<BranchInfo>> {
        panic!("unexpected unrelated Store API")
    }
    async fn delete_branch(&self, name: &str) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn list_branch_deltas(
        &self,
        branch: &str,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::store::BranchDeltaEntry>> {
        panic!("unexpected unrelated Store API")
    }
    async fn land_branch_merge(
        &self,
        branch: &str,
        ops: &[trogon_atlas_store::store::BranchLandOp],
        ctx: WriteContext<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<MutationOutcome>> {
        panic!("unexpected unrelated Store API")
    }
    async fn rebase_branch_deltas(
        &self,
        branch: &str,
        rebases: &[(pb::EntityKind, pb::Id, Option<(pb::Entity, String)>)],
    ) -> trogon_atlas_store::StoreResult<u32> {
        panic!("unexpected unrelated Store API")
    }
    async fn resolve_branch_entry(
        &self,
        branch: &str,
        kind: pb::EntityKind,
        id: &pb::Id,
        take_theirs: bool,
    ) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn append_changeset(
        &self,
        record: &ChangesetRecord,
    ) -> trogon_atlas_store::StoreResult<()> {
        panic!("unexpected unrelated Store API")
    }
    async fn get_changeset(&self, id: &str) -> trogon_atlas_store::StoreResult<ChangesetRecord> {
        panic!("unexpected unrelated Store API")
    }
    async fn list_changesets(
        &self,
        page: ChangesetPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<ChangesetRecord>> {
        panic!("unexpected unrelated Store API")
    }
    async fn list_entity_revisions(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        page: trogon_atlas_store::RevisionPage<'_>,
    ) -> trogon_atlas_store::StoreResult<Vec<trogon_atlas_store::EntityRevisionRecord>> {
        panic!("unexpected unrelated Store API")
    }
    async fn get_entity_revision(
        &self,
        kind: pb::EntityKind,
        id: &pb::Id,
        changeset_id: &str,
        branch: Option<&str>,
    ) -> trogon_atlas_store::StoreResult<trogon_atlas_store::EntityRevisionRecord> {
        panic!("unexpected unrelated Store API")
    }

    async fn source_list(
        &self,
        after: Option<&str>,
        _limit: usize,
    ) -> StoreResult<SourceDocumentPage> {
        let (keys, next_cursor) = match after {
            None => (
                vec!["first", "second", "third", "fourth"],
                Some("opaque-page-end".to_owned()),
            ),
            Some("opaque-page-end") => (vec!["last"], None),
            _ => panic!("sweeper must preserve the store's opaque cursor"),
        };
        Ok(SourceDocumentPage {
            keys: keys
                .into_iter()
                .map(|s| SourceDocumentKey::new(s).unwrap())
                .collect(),
            next_cursor,
        })
    }
    async fn source_get(&self, key: &SourceDocumentKey) -> StoreResult<Option<SourceDocument>> {
        self.visits.lock().unwrap().push(key.as_str().to_owned());
        if self.slow {
            if key.as_str() == "third" {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(Duration::from_secs(25)).await;
        } else if key.as_str() == "first" {
            return Err(StoreError::Unavailable(
                "persistent registration failure".into(),
            ));
        }
        Ok(None)
    }
}

async fn drain() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn retention_continues_past_a_persistent_registration_failure() {
    let visits = Arc::new(Mutex::new(Vec::new()));
    let store = SweepStore {
        visits: visits.clone(),
        slow: false,
    };
    let _service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    drain().await;
    assert_eq!(
        *visits.lock().unwrap(),
        ["first", "second", "third", "fourth", "last"]
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    drain().await;
    assert_eq!(visits.lock().unwrap().len(), 10);
}

#[tokio::test(start_paused = true)]
async fn retention_resumes_inside_a_page_after_timeout_and_reaches_the_next_page() {
    let visits = Arc::new(Mutex::new(Vec::new()));
    let store = SweepStore {
        visits: visits.clone(),
        slow: true,
    };
    let _service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    drain().await;
    for _ in 0..23 {
        tokio::time::advance(Duration::from_secs(5)).await;
        drain().await;
    }
    assert_eq!(
        *visits.lock().unwrap(),
        ["first", "second", "third", "fourth", "last"]
    );
    tokio::time::advance(Duration::from_secs(5)).await;
    drain().await;
    assert_eq!(visits.lock().unwrap().last().unwrap(), "first");
}
