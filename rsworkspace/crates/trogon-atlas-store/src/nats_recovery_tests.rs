#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use trogon_atlas_proto::{Entity, EntityKind, Id};

use super::{entity_key, NatsStore, NatsStoreConfig};
use crate::{
    error::StoreError,
    recovery::{RecoveryAction, RecoveryPolicy, RecoveryResult},
    store::{
        ChangeKind, ChangesetRecord, ChangesetRef, MutationOp, RevisionPage, Store, WriteContext,
    },
};

const AUTHOR: &str = "recovery-tester";
const RPC: &str = "BatchMutate";

async fn isolated_store() -> NatsStore {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
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
