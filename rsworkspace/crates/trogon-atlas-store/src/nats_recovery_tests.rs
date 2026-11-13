#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use trogon_atlas_proto::{Entity, EntityKind, Id};

use super::{entity_key, NatsStore, NatsStoreConfig};
use crate::{
    error::StoreError,
    metrics::StoreMetrics,
    recovery::{RecoveryAction, RecoveryPolicy, RecoveryResult},
    store::{
        ChangeKind, ChangesetRecord, ChangesetRef, MutationOp, RevisionPage, Store, WriteContext,
    },
};

const AUTHOR: &str = "recovery-tester";
const RPC: &str = "BatchMutate";

pub(super) async fn isolated_store() -> NatsStore {
    isolated_store_with_lease(None).await
}

async fn isolated_store_with_lease(duration: Option<std::time::Duration>) -> NatsStore {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url, StoreMetrics::default());
    config.bucket = format!("rec-{suffix}");
    config.branches_bucket = format!("rec-branches-{suffix}");
    config.changesets_bucket = format!("rec-changesets-{suffix}");
    config.revisions_bucket = format!("rec-revisions-{suffix}");
    config.batches_bucket = format!("rec-batches-{suffix}");
    config.namespaces_bucket = format!("rec-namespaces-{suffix}");
    config.lease_bucket = format!("rec-writer-lease-{suffix}");
    config.stream = format!("REC_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("rec.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    if let Some(duration) = duration {
        config.lease_duration = duration;
        config.lease_renew_interval = duration / 6;
    }
    NatsStore::connect_with(config)
        .await
        .expect("connect isolated store")
}

fn event_id(slug: &str) -> Id {
    Id {
        namespace: "recovery".into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(slug: &str, title: &str) -> Entity {
    Entity {
        system: None,
        kind: Some(trogon_atlas_proto::entity::Kind::Event(
            trogon_atlas_proto::Event {
                id: Some(event_id(slug)),
                title: title.into(),
                ..Default::default()
            },
        )),
    }
}

fn title_of(entity: &Entity) -> &str {
    match &entity.kind {
        Some(trogon_atlas_proto::entity::Kind::Event(event)) => &event.title,
        _ => panic!("not an event"),
    }
}

/// A batch that updates one existing entity, deletes another, and creates
/// two, so every kind of entity write is in flight when a fault fires.
struct Scenario {
    changeset_id: String,
    ops: Vec<MutationOp>,
}

const UPDATED: &str = "updated";
const DELETED: &str = "deleted";
const CREATED_FIRST: &str = "created-first";
const CREATED_SECOND: &str = "created-second";
const BATCH_WRITES: usize = 4;

impl Scenario {
    async fn seed(store: &NatsStore) -> Self {
        store
            .create(
                EntityKind::Event,
                &event(UPDATED, "before"),
                WriteContext::baseline(),
            )
            .await
            .expect("seed updated");
        store
            .create(
                EntityKind::Event,
                &event(DELETED, "doomed"),
                WriteContext::baseline(),
            )
            .await
            .expect("seed deleted");
        Self {
            changeset_id: uuid::Uuid::now_v7().to_string(),
            ops: vec![
                MutationOp::Put {
                    kind: EntityKind::Event,
                    entity: event(UPDATED, "after"),
                    if_match: None,
                    force: false,
                },
                MutationOp::Delete {
                    kind: EntityKind::Event,
                    id: event_id(DELETED),
                    if_match: None,
                },
                MutationOp::Create {
                    kind: EntityKind::Event,
                    entity: event(CREATED_FIRST, "new one"),
                },
                MutationOp::Create {
                    kind: EntityKind::Event,
                    entity: event(CREATED_SECOND, "new two"),
                },
            ],
        }
    }

    fn changeset(&self) -> ChangesetRef<'_> {
        ChangesetRef {
            id: &self.changeset_id,
            author: AUTHOR,
            rpc: RPC,
            operation_id: None,
        }
    }

    async fn apply(&self, store: &NatsStore) -> Result<(), StoreError> {
        store
            .batch_apply(
                &self.ops,
                WriteContext::baseline().attributed(self.changeset()),
            )
            .await
            .map(|_| ())
    }

    /// What the server does after `batch_apply` returns `Ok`.
    async fn append_changeset(&self, store: &NatsStore) -> Result<(), StoreError> {
        store
            .append_changeset(&ChangesetRecord {
                id: self.changeset_id.clone(),
                author: AUTHOR.into(),
                at: chrono::Utc::now().to_rfc3339(),
                message: "batch_mutate: 4 op(s)".into(),
                rpc: RPC.into(),
                branch: None,
                ops: Vec::new(),
                operation_id: None,
                validation: crate::ChangesetValidation::NotRecorded,
            })
            .await
    }

    async fn title(store: &NatsStore, slug: &str) -> Option<String> {
        match store.get(EntityKind::Event, &event_id(slug), None).await {
            Ok(stored) => Some(title_of(&stored.entity).to_owned()),
            Err(StoreError::NotFound) => None,
            Err(e) => panic!("get {slug}: {e}"),
        }
    }

    async fn notifications(&self, store: &NatsStore) -> Vec<(String, bool)> {
        let changes = store.read_changes(0, 1_000).await.expect("read changes");
        changes
            .into_iter()
            .filter(|c| c.changeset_id == self.changeset_id)
            .map(|c| {
                (
                    c.entity_ref.id.map(|id| id.slug).unwrap_or_default(),
                    matches!(c.kind, ChangeKind::Delete),
                )
            })
            .collect()
    }

    async fn revision_count(&self, store: &NatsStore, slug: &str) -> usize {
        store
            .list_entity_revisions(
                EntityKind::Event,
                &event_id(slug),
                RevisionPage {
                    before: None,
                    limit: 100,
                    branch: None,
                },
            )
            .await
            .expect("list revisions")
            .into_iter()
            .filter(|r| r.changeset_id == self.changeset_id)
            .count()
    }

    async fn assert_absent(&self, store: &NatsStore) {
        assert_eq!(Self::title(store, UPDATED).await.as_deref(), Some("before"));
        assert_eq!(Self::title(store, DELETED).await.as_deref(), Some("doomed"));
        assert_eq!(Self::title(store, CREATED_FIRST).await, None);
        assert_eq!(Self::title(store, CREATED_SECOND).await, None);
        assert!(
            self.notifications(store).await.is_empty(),
            "no notification"
        );
        for slug in [UPDATED, DELETED, CREATED_FIRST, CREATED_SECOND] {
            assert_eq!(self.revision_count(store, slug).await, 0, "revision {slug}");
        }
        assert!(matches!(
            store.get_changeset(&self.changeset_id).await,
            Err(StoreError::NotFound)
        ));
        assert_journal_empty(store).await;
    }

    async fn assert_applied(&self, store: &NatsStore) {
        assert_eq!(Self::title(store, UPDATED).await.as_deref(), Some("after"));
        assert_eq!(Self::title(store, DELETED).await, None);
        assert_eq!(
            Self::title(store, CREATED_FIRST).await.as_deref(),
            Some("new one")
        );
        assert_eq!(
            Self::title(store, CREATED_SECOND).await.as_deref(),
            Some("new two")
        );
        let mut notified = self.notifications(store).await;
        notified.sort();
        notified.dedup();
        assert_eq!(
            notified,
            vec![
                (CREATED_FIRST.to_owned(), false),
                (CREATED_SECOND.to_owned(), false),
                (DELETED.to_owned(), true),
                (UPDATED.to_owned(), false),
            ]
        );
        for slug in [UPDATED, DELETED, CREATED_FIRST, CREATED_SECOND] {
            assert_eq!(self.revision_count(store, slug).await, 1, "revision {slug}");
        }
        let changeset = store
            .get_changeset(&self.changeset_id)
            .await
            .expect("changeset recorded");
        assert_eq!(changeset.author, AUTHOR);
        assert_journal_empty(store).await;
    }
}

async fn assert_journal_empty(store: &NatsStore) {
    let report = store
        .recover_batches(RecoveryPolicy::report_all())
        .await
        .expect("report");
    assert!(report.batches.is_empty(), "journal left behind: {report:?}");
}

async fn repair(store: &NatsStore) -> crate::recovery::BatchRecoveryReport {
    let report = store
        .recover_batches(RecoveryPolicy::repair_all())
        .await
        .expect("recover");
    assert!(
        report.is_converged(),
        "recovery did not converge: {report:?}"
    );
    report
}

fn clear_faults(store: &NatsStore) {
    *store.faults.lock() = super::journal::FaultPlan::default();
}

const RECOVERY_BRANCH: &str = "journal-recovery";

async fn seed_branch_batch(store: &NatsStore) -> Scenario {
    let scenario = Scenario::seed(store).await;
    store.create_branch(RECOVERY_BRANCH, "").await.unwrap();
    store
        .put(
            EntityKind::Event,
            &event(UPDATED, "branch-before"),
            false,
            WriteContext::branch(RECOVERY_BRANCH),
        )
        .await
        .unwrap();
    scenario
}

async fn branch_title(store: &NatsStore, slug: &str) -> Option<String> {
    match store
        .get(EntityKind::Event, &event_id(slug), Some(RECOVERY_BRANCH))
        .await
    {
        Ok(stored) => Some(title_of(&stored.entity).to_owned()),
        Err(StoreError::NotFound) => None,
        Err(error) => panic!("get branch {slug}: {error}"),
    }
}

async fn branch_revision_count(store: &NatsStore, scenario: &Scenario, slug: &str) -> usize {
    store
        .list_entity_revisions(
            EntityKind::Event,
            &event_id(slug),
            RevisionPage {
                before: None,
                limit: 100,
                branch: Some(RECOVERY_BRANCH),
            },
        )
        .await
        .unwrap()
        .into_iter()
        .filter(|revision| revision.changeset_id == scenario.changeset_id)
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_journal_crash_before_commit_restores_raw_deltas_and_removes_history() {
    for writes in [2, BATCH_WRITES] {
        let store = isolated_store().await;
        let scenario = seed_branch_batch(&store).await;
        let key = super::branch_delta_key(RECOVERY_BRANCH, EntityKind::Event, &event_id(UPDATED));
        let before = super::kv_entry_timeout(&store.branches_kv, key.clone())
            .await
            .unwrap()
            .unwrap()
            .value;
        store.faults.lock().crash_after_writes = Some(writes);
        store
            .batch_apply(
                &scenario.ops,
                WriteContext::branch(RECOVERY_BRANCH).attributed(scenario.changeset()),
            )
            .await
            .expect_err("interrupted branch batch");
        clear_faults(&store);
        let pending = store
            .recover_batches(RecoveryPolicy::report_all())
            .await
            .unwrap();
        let report = repair(&store).await;
        assert_eq!(
            branch_title(&store, UPDATED).await.as_deref(),
            Some("branch-before")
        );
        assert_eq!(pending.batches.len(), 1);
        assert_eq!(pending.batches[0].action, RecoveryAction::RollBack);
        assert_eq!(report.batches.len(), 1);
        assert_eq!(
            branch_title(&store, DELETED).await.as_deref(),
            Some("doomed")
        );
        assert_eq!(branch_title(&store, CREATED_FIRST).await, None);
        assert_eq!(branch_title(&store, CREATED_SECOND).await, None);
        assert_eq!(
            super::kv_entry_timeout(&store.branches_kv, key)
                .await
                .unwrap()
                .unwrap()
                .value,
            before
        );
        assert_eq!(
            Scenario::title(&store, UPDATED).await.as_deref(),
            Some("before")
        );
        assert_eq!(
            Scenario::title(&store, DELETED).await.as_deref(),
            Some("doomed")
        );
        for slug in [UPDATED, DELETED, CREATED_FIRST, CREATED_SECOND] {
            assert_eq!(branch_revision_count(&store, &scenario, slug).await, 0);
        }
        assert!(matches!(
            store.get_changeset(&scenario.changeset_id).await,
            Err(StoreError::NotFound)
        ));
        assert_journal_empty(&store).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_journal_committed_recovery_keeps_scope_and_emits_no_baseline_notifications() {
    let store = isolated_store().await;
    let scenario = seed_branch_batch(&store).await;
    store.faults.lock().crash_after_commit = true;
    store
        .batch_apply(
            &scenario.ops,
            WriteContext::branch(RECOVERY_BRANCH).attributed(scenario.changeset()),
        )
        .await
        .expect_err("interrupted committed branch batch");
    clear_faults(&store);
    let report = repair(&store).await;
    assert_eq!(
        branch_title(&store, UPDATED).await.as_deref(),
        Some("after")
    );
    assert_eq!(branch_title(&store, DELETED).await, None);
    assert_eq!(
        branch_title(&store, CREATED_FIRST).await.as_deref(),
        Some("new one")
    );
    assert_eq!(
        branch_title(&store, CREATED_SECOND).await.as_deref(),
        Some("new two")
    );
    assert_eq!(
        Scenario::title(&store, UPDATED).await.as_deref(),
        Some("before")
    );
    assert_eq!(scenario.notifications(&store).await.len(), 0);
    for slug in [UPDATED, DELETED, CREATED_FIRST, CREATED_SECOND] {
        assert_eq!(branch_revision_count(&store, &scenario, slug).await, 1);
    }
    assert_eq!(
        store
            .get_changeset(&scenario.changeset_id)
            .await
            .unwrap()
            .branch
            .as_deref(),
        Some(RECOVERY_BRANCH)
    );
    assert_eq!(report.batches.len(), 1);
    assert_eq!(report.batches[0].action, RecoveryAction::RollForward);
    assert_journal_empty(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_write_whose_rollback_also_fails_converges_to_absent_after_recovery() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    {
        let mut faults = store.faults.lock();
        faults.fail_write_at = Some(2);
        faults.fail_rollback = true;
    }

    let err = scenario.apply(&store).await.expect_err("batch must fail");
    let StoreError::BatchFailed { index, source } = err else {
        panic!("expected BatchFailed, got {err}");
    };
    assert_eq!(index, 2);
    let StoreError::PartialApply {
        journal: Some(journal),
        ..
    } = *source
    else {
        panic!("expected PartialApply, got {source}");
    };
    assert_eq!(journal.as_str(), scenario.changeset_id);

    clear_faults(&store);
    let report = repair(&store).await;
    assert_eq!(report.batches.len(), 1);
    assert_eq!(report.batches[0].action, RecoveryAction::RollBack);
    scenario.assert_absent(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_with_an_unknown_outcome_is_rolled_back_inline() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().fail_write_at = Some(2);

    let err = scenario.apply(&store).await.expect_err("batch must fail");
    assert!(
        matches!(err, StoreError::BatchFailed { index: 2, .. }),
        "got {err}"
    );

    clear_faults(&store);
    scenario.assert_absent(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_some_entity_writes_rolls_back_on_recovery() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().crash_after_writes = Some(2);

    scenario.apply(&store).await.expect_err("crash");
    clear_faults(&store);

    let pending = store
        .recover_batches(RecoveryPolicy::report_all())
        .await
        .expect("report");
    assert_eq!(pending.batches.len(), 1, "{pending:?}");
    assert_eq!(pending.batches[0].action, RecoveryAction::RollBack);
    assert_eq!(pending.batches[0].result, RecoveryResult::Pending);
    let mut keys = pending.batches[0].keys.clone();
    keys.sort();
    let mut expected = vec![
        entity_key(EntityKind::Event, &event_id(DELETED)),
        entity_key(EntityKind::Event, &event_id(UPDATED)),
    ];
    expected.sort();
    assert_eq!(keys, expected);
    assert_eq!(
        Scenario::title(&store, UPDATED).await.as_deref(),
        Some("after")
    );

    repair(&store).await;
    scenario.assert_absent(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_every_entity_write_before_commit_rolls_back() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().crash_after_writes = Some(BATCH_WRITES);

    scenario.apply(&store).await.expect_err("crash");
    clear_faults(&store);

    repair(&store).await;
    scenario.assert_absent(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_crash_after_commit_rolls_forward_with_revisions_notifications_and_changeset() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().crash_after_commit = true;

    scenario.apply(&store).await.expect_err("crash");
    clear_faults(&store);

    let report = repair(&store).await;
    assert_eq!(report.batches.len(), 1);
    assert_eq!(report.batches[0].action, RecoveryAction::RollForward);
    scenario.assert_applied(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_notification_is_republished_by_recovery() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().fail_publish = true;

    scenario.apply(&store).await.expect("applied");
    scenario.append_changeset(&store).await.expect("changeset");
    clear_faults(&store);
    assert_eq!(scenario.notifications(&store).await, Vec::new());

    repair(&store).await;
    scenario.assert_applied(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_changeset_append_is_completed_by_recovery() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().fail_changeset_append = true;

    scenario.apply(&store).await.expect("applied");
    scenario
        .append_changeset(&store)
        .await
        .expect_err("append fails");
    clear_faults(&store);

    let report = repair(&store).await;
    assert_eq!(report.batches.len(), 1);
    assert_eq!(report.batches[0].action, RecoveryAction::CompleteChangeset);
    scenario.assert_applied(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_batch_leaves_no_journal() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;

    scenario.apply(&store).await.expect("applied");
    scenario.append_changeset(&store).await.expect("changeset");

    scenario.assert_applied(&store).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn automatic_recovery_leaves_a_batch_another_writer_may_still_run_alone() {
    let store = isolated_store().await;
    let scenario = Scenario::seed(&store).await;
    store.faults.lock().crash_after_writes = Some(2);
    scenario.apply(&store).await.expect_err("crash");
    clear_faults(&store);

    let report = store
        .recover_batches(RecoveryPolicy::automatic())
        .await
        .expect("recover");
    assert_eq!(report.skipped_recent, 1);
    assert!(!report.is_converged());
    assert_eq!(
        Scenario::title(&store, UPDATED).await.as_deref(),
        Some("after")
    );

    repair(&store).await;
    scenario.assert_absent(&store).await;
}

async fn commit_losing_its_changeset(store: &NatsStore, deferred: bool) -> (Scenario, String) {
    let scenario = Scenario::seed(store).await;
    let key = format!("recovery-op-{}", scenario.changeset_id);
    store
        .claim_operation(&key, "digest", RPC, None)
        .await
        .expect("claim");
    store.faults.lock().fail_changeset_append = true;
    let changeset = ChangesetRef {
        operation_id: Some("op-recovery"),
        ..scenario.changeset()
    };
    let ctx = WriteContext::baseline()
        .attributed(changeset)
        .with_operation_key(&key);
    let ctx = if deferred {
        ctx.with_deferred_settlement()
    } else {
        ctx
    };
    store
        .batch_apply(&scenario.ops, ctx)
        .await
        .expect("applied");
    scenario
        .append_changeset(store)
        .await
        .expect_err("append fails");
    clear_faults(store);
    (scenario, key)
}

async fn crash_after_a_claimed_commit(store: &NatsStore, deferred: bool) -> (Scenario, String) {
    let (scenario, key) = commit_losing_its_changeset(store, deferred).await;
    repair(store).await;
    (scenario, key)
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_settles_a_landed_batch_applied_and_names_its_operation() {
    let store = isolated_store().await;
    let (scenario, key) = crash_after_a_claimed_commit(&store, false).await;

    let record = store.get_operation(&key).await.unwrap().unwrap();
    assert_eq!(record.status, crate::store::OperationStatus::Applied);
    let changeset = store.get_changeset(&scenario.changeset_id).await.unwrap();
    assert_eq!(changeset.operation_id.as_deref(), Some("op-recovery"));
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_leaves_a_deferred_settlement_retryable() {
    let store = isolated_store().await;
    let (scenario, key) = crash_after_a_claimed_commit(&store, true).await;

    scenario.assert_applied(&store).await;
    let record = store.get_operation(&key).await.unwrap().unwrap();
    assert_eq!(record.status, crate::store::OperationStatus::NotApplied);
    assert_eq!(record.changeset_id, scenario.changeset_id);
    let changeset = store.get_changeset(&scenario.changeset_id).await.unwrap();
    assert_eq!(changeset.operation_id.as_deref(), Some("op-recovery"));
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_keeps_a_deferred_settlement_the_request_already_finished() {
    let store = isolated_store().await;
    let (scenario, key) = commit_losing_its_changeset(&store, true).await;
    store
        .settle_operation_applied(&key, &scenario.changeset_id)
        .await
        .expect("settle");

    repair(&store).await;

    let record = store.get_operation(&key).await.unwrap().unwrap();
    assert_eq!(record.status, crate::store::OperationStatus::Applied);
    assert_eq!(record.changeset_id, scenario.changeset_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn uncommitted_journals_block_later_model_writes_until_recovery() {
    for branch in [None, Some(RECOVERY_BRANCH)] {
        let store = isolated_store().await;
        let scenario = seed_branch_batch(&store).await;
        store.faults.lock().crash_after_writes = Some(2);
        store
            .batch_apply(
                &scenario.ops,
                WriteContext::scoped(branch).attributed(scenario.changeset()),
            )
            .await
            .expect_err("interrupted batch");
        clear_faults(&store);
        assert!(matches!(
            store.read_marker().await,
            Err(StoreError::Unavailable(_))
        ));
        let result = store
            .put(
                EntityKind::Event,
                &event(UPDATED, "later"),
                false,
                WriteContext::scoped(branch),
            )
            .await;
        assert!(
            matches!(result, Err(StoreError::Unavailable(_))),
            "later write escaped unresolved rollback: {result:?}"
        );
        repair(&store).await;
        store
            .put(
                EntityKind::Event,
                &event(UPDATED, "later"),
                false,
                WriteContext::scoped(branch),
            )
            .await
            .unwrap();
        assert_eq!(
            title_of(
                &store
                    .get(EntityKind::Event, &event_id(UPDATED), branch)
                    .await
                    .unwrap()
                    .entity
            ),
            "later"
        );
        store.read_marker().await.unwrap();
    }
}

async fn raw_branch_rows(store: &NatsStore) -> std::collections::BTreeMap<String, bytes::Bytes> {
    let mut rows = std::collections::BTreeMap::new();
    for key in [
        super::branch_meta_key(RECOVERY_BRANCH),
        super::branch_delta_key(RECOVERY_BRANCH, EntityKind::Event, &event_id(UPDATED)),
        super::branch_delta_key(RECOVERY_BRANCH, EntityKind::Event, &event_id(DELETED)),
    ] {
        if let Some(entry) = super::live_entry(
            super::kv_entry_timeout(&store.branches_kv, key.clone())
                .await
                .unwrap(),
        ) {
            rows.insert(key, entry.value);
        }
    }
    rows
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_maintenance_journals_recover_rebase_and_delete_at_every_durable_decision() {
    for delete in [false, true] {
        for committed in [false, true] {
            let store = isolated_store().await;
            seed_branch_batch(&store).await;
            store
                .put(
                    EntityKind::Event,
                    &event(DELETED, "branch-second"),
                    false,
                    WriteContext::branch(RECOVERY_BRANCH),
                )
                .await
                .unwrap();
            let before = raw_branch_rows(&store).await;
            if committed {
                store.faults.lock().crash_after_commit = true;
            } else {
                store.faults.lock().crash_after_writes = Some(if delete { 3 } else { 1 });
            }
            let result = if delete {
                store.delete_branch(RECOVERY_BRANCH).await
            } else {
                store
                    .rebase_branch_deltas(
                        RECOVERY_BRANCH,
                        &[
                            (
                                EntityKind::Event,
                                event_id(UPDATED),
                                Some((event(UPDATED, "rebased"), "rebased-etag".into())),
                            ),
                            (EntityKind::Event, event_id(DELETED), None),
                        ],
                    )
                    .await
                    .map(|_| ())
            };
            result.expect_err("interrupted maintenance");
            clear_faults(&store);
            assert!(matches!(
                store.read_marker().await,
                Err(StoreError::Unavailable(_))
            ));
            let report = repair(&store).await;
            assert_eq!(report.batches.len(), 1);
            assert_eq!(
                report.batches[0].action,
                if committed {
                    RecoveryAction::RollForward
                } else {
                    RecoveryAction::RollBack
                }
            );
            if !committed {
                assert_eq!(raw_branch_rows(&store).await, before);
                assert_eq!(
                    branch_title(&store, UPDATED).await.as_deref(),
                    Some("branch-before")
                );
                assert_eq!(
                    branch_title(&store, DELETED).await.as_deref(),
                    Some("branch-second")
                );
            } else if delete {
                assert!(raw_branch_rows(&store).await.is_empty());
                assert!(store
                    .list_branches()
                    .await
                    .unwrap()
                    .iter()
                    .all(|info| info.name != RECOVERY_BRANCH));
            } else {
                let deltas = store.list_branch_deltas(RECOVERY_BRANCH).await.unwrap();
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].base_etag, "rebased-etag");
                assert_eq!(title_of(deltas[0].base.as_ref().unwrap()), "rebased");
                assert_eq!(
                    branch_title(&store, UPDATED).await.as_deref(),
                    Some("branch-before")
                );
                assert_eq!(
                    branch_title(&store, DELETED).await.as_deref(),
                    Some("doomed")
                );
            }
            assert_eq!(
                Scenario::title(&store, UPDATED).await.as_deref(),
                Some("before")
            );
            assert_eq!(
                Scenario::title(&store, DELETED).await.as_deref(),
                Some("doomed")
            );
            store.read_marker().await.unwrap();
        }
    }
}

async fn branch_landing_ops(store: &NatsStore) -> Vec<crate::store::BranchLandOp> {
    vec![
        crate::store::BranchLandOp::Put {
            kind: EntityKind::Event,
            ours: Box::new(
                store
                    .get(EntityKind::Event, &event_id(UPDATED), Some(RECOVERY_BRANCH))
                    .await
                    .unwrap()
                    .entity,
            ),
            base: None,
            expected_baseline_etag: Some(
                store
                    .get(EntityKind::Event, &event_id(UPDATED), None)
                    .await
                    .unwrap()
                    .etag,
            ),
        },
        crate::store::BranchLandOp::Delete {
            kind: EntityKind::Event,
            id: event_id(DELETED),
            expected_baseline_etag: store
                .get(EntityKind::Event, &event_id(DELETED), None)
                .await
                .unwrap()
                .etag,
        },
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_merge_journals_roll_back_baseline_or_roll_forward_cleanup() {
    for committed in [false, true] {
        let store = isolated_store().await;
        let scenario = seed_branch_batch(&store).await;
        store
            .delete(
                EntityKind::Event,
                &event_id(DELETED),
                None,
                WriteContext::branch(RECOVERY_BRANCH),
            )
            .await
            .unwrap();
        let before = raw_branch_rows(&store).await;
        let ops = branch_landing_ops(&store).await;
        if committed {
            store.faults.lock().crash_after_commit = true;
        } else {
            store.faults.lock().crash_after_writes = Some(1);
        }
        store
            .land_branch_merge(
                RECOVERY_BRANCH,
                &ops,
                WriteContext::baseline().attributed(scenario.changeset()),
            )
            .await
            .expect_err("interrupted merge");
        clear_faults(&store);
        assert!(matches!(
            store.read_marker().await,
            Err(StoreError::Unavailable(_))
        ));
        if committed {
            assert!(matches!(
                store
                    .put(
                        EntityKind::Event,
                        &event(UPDATED, "later"),
                        false,
                        WriteContext::baseline()
                    )
                    .await,
                Err(StoreError::Unavailable(_))
            ));
        }
        let report = repair(&store).await;
        assert_eq!(report.batches.len(), 1);
        if committed {
            assert_eq!(
                Scenario::title(&store, UPDATED).await.as_deref(),
                Some("branch-before")
            );
            assert_eq!(Scenario::title(&store, DELETED).await, None);
            assert!(store
                .list_branch_deltas(RECOVERY_BRANCH)
                .await
                .unwrap()
                .is_empty());
            assert_eq!(
                store
                    .get_changeset(&scenario.changeset_id)
                    .await
                    .unwrap()
                    .branch,
                None
            );
        } else {
            assert_eq!(
                Scenario::title(&store, UPDATED).await.as_deref(),
                Some("before")
            );
            assert_eq!(
                Scenario::title(&store, DELETED).await.as_deref(),
                Some("doomed")
            );
            assert_eq!(raw_branch_rows(&store).await, before);
            assert!(matches!(
                store.get_changeset(&scenario.changeset_id).await,
                Err(StoreError::NotFound)
            ));
        }
        assert!(store
            .list_branches()
            .await
            .unwrap()
            .iter()
            .any(|info| info.name == RECOVERY_BRANCH));
        store.read_marker().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn committed_merge_cleanup_preserves_a_newer_raw_delta_revision() {
    let store = isolated_store().await;
    let scenario = seed_branch_batch(&store).await;
    let ops = branch_landing_ops(&store).await;
    store.faults.lock().crash_after_commit = true;
    store
        .land_branch_merge(
            RECOVERY_BRANCH,
            &ops,
            WriteContext::baseline().attributed(scenario.changeset()),
        )
        .await
        .expect_err("interrupted merge");
    clear_faults(&store);
    let key = super::branch_delta_key(RECOVERY_BRANCH, EntityKind::Event, &event_id(UPDATED));
    let old = super::kv_entry_timeout(&store.branches_kv, key.clone())
        .await
        .unwrap()
        .unwrap();
    let mut delta = super::decode_branch_delta(&old.value).unwrap();
    delta.ours = Some(event(UPDATED, "newer-delta"));
    let bytes = super::encode_branch_delta(&delta);
    store
        .branches_kv
        .update(&key, bytes.clone(), old.revision)
        .await
        .unwrap();
    repair(&store).await;
    assert_eq!(
        super::kv_entry_timeout(&store.branches_kv, key)
            .await
            .unwrap()
            .unwrap()
            .value,
        bytes
    );
    assert_eq!(
        branch_title(&store, UPDATED).await.as_deref(),
        Some("newer-delta")
    );
    assert_eq!(
        Scenario::title(&store, UPDATED).await.as_deref(),
        Some("branch-before")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn standby_recovers_a_foreign_interrupted_branch_batch_after_real_lease_takeover() {
    let store = isolated_store_with_lease(Some(std::time::Duration::from_millis(600))).await;
    let scenario = seed_branch_batch(&store).await;
    let mut configuration = store.config.as_ref().clone();
    configuration.role = trogon_atlas_core::WriterRole::Standby;
    let standby = NatsStore::connect_with(configuration).await.unwrap();
    assert!(!standby.is_writer());
    store.faults.lock().crash_after_writes = Some(2);
    store
        .batch_apply(
            &scenario.ops,
            WriteContext::branch(RECOVERY_BRANCH).attributed(scenario.changeset()),
        )
        .await
        .expect_err("writer interrupted");
    assert!(matches!(
        standby.read_marker().await,
        Err(StoreError::Unavailable(_))
    ));
    drop(store);
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    assert!(standby.is_writer());
    assert_eq!(standby.writer_status().epoch.get(), 2);
    assert!(matches!(
        standby
            .put(
                EntityKind::Event,
                &event(UPDATED, "later"),
                false,
                WriteContext::branch(RECOVERY_BRANCH)
            )
            .await,
        Err(StoreError::Unavailable(_))
    ));
    let report = repair(&standby).await;
    assert_eq!(report.batches.len(), 1);
    assert_eq!(
        branch_title(&standby, UPDATED).await.as_deref(),
        Some("branch-before")
    );
    assert_eq!(
        branch_title(&standby, DELETED).await.as_deref(),
        Some("doomed")
    );
    standby.read_marker().await.unwrap();
    standby
        .put(
            EntityKind::Event,
            &event(UPDATED, "after-takeover"),
            false,
            WriteContext::branch(RECOVERY_BRANCH),
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_journal_finalization_keeps_committed_history_recoverable() {
    for branch in [None, Some(RECOVERY_BRANCH)] {
        let store = isolated_store().await;
        let scenario = seed_branch_batch(&store).await;
        store.faults.lock().crash_during_publish_finalize = true;
        store
            .batch_apply(
                &scenario.ops,
                WriteContext::scoped(branch).attributed(scenario.changeset()),
            )
            .await
            .unwrap();
        clear_faults(&store);
        assert!(matches!(
            store.read_marker().await,
            Err(StoreError::Unavailable(_))
        ));
        let report = repair(&store).await;
        assert_eq!(report.batches.len(), 1);
        assert_eq!(
            store
                .get_changeset(&scenario.changeset_id)
                .await
                .unwrap()
                .branch
                .as_deref(),
            branch
        );
        let updated = store
            .get(EntityKind::Event, &event_id(UPDATED), branch)
            .await
            .unwrap();
        assert_eq!(title_of(&updated.entity), "after");
        assert_eq!(
            store
                .list_entity_revisions(
                    EntityKind::Event,
                    &event_id(UPDATED),
                    RevisionPage {
                        branch,
                        before: None,
                        limit: 100
                    }
                )
                .await
                .unwrap()
                .into_iter()
                .filter(|r| r.changeset_id == scenario.changeset_id)
                .count(),
            1
        );
        store.read_marker().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_journal_restores_an_existing_tombstone_and_its_baseline_snapshot() {
    let store = isolated_store().await;
    seed_branch_batch(&store).await;
    store
        .delete(
            EntityKind::Event,
            &event_id(UPDATED),
            None,
            WriteContext::branch(RECOVERY_BRANCH),
        )
        .await
        .unwrap();
    let key = super::branch_delta_key(RECOVERY_BRANCH, EntityKind::Event, &event_id(UPDATED));
    let before = super::kv_entry_timeout(&store.branches_kv, key.clone())
        .await
        .unwrap()
        .unwrap()
        .value;
    let delta = super::decode_branch_delta(&before).unwrap();
    assert!(delta.tombstone);
    assert_eq!(title_of(delta.base.as_ref().unwrap()), "before");
    assert_ne!(delta.base_etag, "");
    store.faults.lock().crash_after_writes = Some(1);
    store
        .batch_apply(
            &[MutationOp::Put {
                kind: EntityKind::Event,
                entity: event(UPDATED, "resurrected"),
                if_match: None,
                force: false,
            }],
            WriteContext::branch(RECOVERY_BRANCH),
        )
        .await
        .expect_err("interrupted resurrection");
    clear_faults(&store);
    repair(&store).await;
    assert_eq!(
        super::kv_entry_timeout(&store.branches_kv, key)
            .await
            .unwrap()
            .unwrap()
            .value,
        before
    );
    assert!(matches!(
        store
            .get(EntityKind::Event, &event_id(UPDATED), Some(RECOVERY_BRANCH))
            .await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        Scenario::title(&store, UPDATED).await.as_deref(),
        Some("before")
    );
    store.read_marker().await.unwrap();
}
