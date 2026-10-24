#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use trogon_atlas_core::WriterRole;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    source::{
        SourceDocumentKey, SourcePublication, SourcePublicationObserver, SourcePublicationOutcome,
        SourcePublicationPoint,
    },
    store::{Store, WriteContext},
    NatsStore, NatsStoreConfig, StoreError, StoreMetrics, StoreResult,
};

struct Crash(SourcePublicationPoint);
impl SourcePublicationObserver for Crash {
    fn reached(&self, point: SourcePublicationPoint) -> StoreResult<()> {
        if point == self.0 {
            Err(StoreError::Unavailable(
                "injected source process interruption".into(),
            ))
        } else {
            Ok(())
        }
    }
}
fn config(url: &str) -> NatsStoreConfig {
    let suffix = uuid::Uuid::now_v7().to_string();
    let mut c = NatsStoreConfig::new(url, StoreMetrics::default());
    c.bucket = format!("source-e-{suffix}");
    c.branches_bucket = format!("source-br-{suffix}");
    c.batches_bucket = format!("source-b-{suffix}");
    c.changesets_bucket = format!("source-c-{suffix}");
    c.revisions_bucket = format!("source-r-{suffix}");
    c.namespaces_bucket = format!("source-n-{suffix}");
    c.operations_bucket = format!("source-o-{suffix}");
    c.lease_bucket = format!("source-l-{suffix}");
    c.stream = format!("SOURCE_{}", suffix.replace('-', "_"));
    c.subject_root = format!("source.{suffix}");
    c.lease_duration = std::time::Duration::from_millis(300);
    c.lease_renew_interval = std::time::Duration::from_millis(50);
    c
}
fn entity(title: &str) -> pb::Entity {
    pb::Entity {
        kind: Some(pb::entity::Kind::ReadModel(pb::ReadModel {
            id: Some(pb::Id {
                namespace: "fixture".into(),
                slug: "read-model".into(),
                version: 1,
            }),
            title: title.into(),
            ..Default::default()
        })),
        ..Default::default()
    }
}
async fn case(point: SourcePublicationPoint, branch: Option<&str>) {
    let nats = trogon_atlas_testsupport::shared().await;
    let configuration = config(&nats.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    if let Some(branch) = branch {
        store.create_branch(branch, "").await.unwrap();
    }
    let value = entity("before");
    let id = trogon_atlas_core::refs::entity_id(&value).unwrap().clone();
    let current = store
        .create(
            pb::EntityKind::ReadModel,
            &value,
            WriteContext::scoped(branch),
        )
        .await
        .unwrap();
    let registration = SourceDocumentKey::new("source-publication").unwrap();
    let doc = store
        .source_compare_and_swap(&registration, None, b"control-intent", 128 * 1024 * 1024)
        .await
        .unwrap();
    let owner = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: Some(id.clone()),
    };
    let observed = pb::SourceObservedOwner {
        owner: Some(owner.clone()),
        uid: current.entity.system.as_ref().unwrap().uid.clone(),
        etag: current.etag.clone(),
    };
    let guard = pb::SourceNativeGuard {
        owner: Some(owner.clone()),
        condition: Some(pb::source_native_guard::Condition::Existing(
            observed.clone(),
        )),
    };
    let source = pb::SourceResourceKey {
        registration_id: registration.as_str().into(),
        category: "model".into(),
        key: "read-model".into(),
    };
    let publication = SourcePublication {
        storage_limit: 128 * 1024 * 1024,
        finalization_reserve_bytes: 0,
        registration: registration.clone(),
        control_revision: doc.revision,
        run_id: "run".into(),
        plan_digest: "digest".into(),
        plan: pb::SourceRunPlan {
            branch: branch.map(str::to_owned),
            native_guards: vec![guard.clone()],
            operations: vec![pb::SourceNativeOperation {
                mapping_id: "mapping".into(),
                source: Some(source.clone()),
                operation: Some(pb::source_native_operation::Operation::Put(
                    pb::SourcePutOperation {
                        entity: Some(entity("committed")),
                        guard: Some(guard),
                    },
                )),
            }],
            ..Default::default()
        },
        values: vec![pb::SourceCommittedValue {
            mapping_id: "mapping".into(),
            source: Some(source),
            observed: Some(observed),
            ..Default::default()
        }],
    };
    let mut read_config = configuration.clone();
    read_config.role = WriterRole::Reader;
    let reader = NatsStore::connect_with(read_config).await.unwrap();
    let faulty = store
        .clone()
        .with_source_publication_observer(Arc::new(Crash(point)));
    assert!(faulty.source_publish(&publication).await.is_err());
    let committed = matches!(
        point,
        SourcePublicationPoint::CommitDecisionDurable | SourcePublicationPoint::FenceReleased
    );
    if !matches!(
        point,
        SourcePublicationPoint::IntentDurable | SourcePublicationPoint::FenceReleased
    ) {
        assert!(matches!(
            reader.get(pb::EntityKind::ReadModel, &id, branch).await,
            Err(StoreError::Unavailable(_))
        ));
        assert!(reader.list(Default::default(), None, branch).await.is_err());
        assert!(store
            .update(
                pb::EntityKind::ReadModel,
                &entity("competing"),
                &current.etag,
                false,
                WriteContext::scoped(branch)
            )
            .await
            .is_err());
    }
    drop(faulty);
    drop(store);
    let store = NatsStore::connect_with(configuration).await.unwrap();
    for _ in 0..30 {
        if store.is_writer() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        store.is_writer(),
        "restarted writer must acquire a new lease epoch"
    );
    let outcome = store
        .source_recover(&registration, "run", "digest")
        .await
        .unwrap();
    match outcome {
        SourcePublicationOutcome::Committed(marker) => {
            assert!(committed);
            assert_eq!(
                marker.values[0]
                    .entity
                    .as_ref()
                    .unwrap()
                    .system
                    .as_ref()
                    .unwrap()
                    .uid,
                current.entity.system.as_ref().unwrap().uid
            );
            let accepted = marker.values[0].entity.clone().unwrap();
            let actual = reader
                .get(pb::EntityKind::ReadModel, &id, branch)
                .await
                .unwrap();
            assert_eq!(actual.entity, accepted);
            let later = store
                .update(
                    pb::EntityKind::ReadModel,
                    &entity("later native edit"),
                    &actual.etag,
                    false,
                    WriteContext::scoped(branch),
                )
                .await
                .unwrap();
            let SourcePublicationOutcome::Committed(replayed) = store
                .source_recover(&registration, "run", "digest")
                .await
                .unwrap()
            else {
                panic!("committed replay");
            };
            assert_eq!(marker, replayed);
            assert_eq!(
                reader
                    .get(pb::EntityKind::ReadModel, &id, branch)
                    .await
                    .unwrap()
                    .entity,
                later.entity
            );
        }
        SourcePublicationOutcome::NotCommitted => {
            assert!(!committed);
            assert_eq!(
                reader
                    .get(pb::EntityKind::ReadModel, &id, branch)
                    .await
                    .unwrap()
                    .entity,
                current.entity
            );
        }
    }
}
#[tokio::test]
async fn intent_before_fence_can_settle_without_reverting_native_state() {
    Box::pin(case(SourcePublicationPoint::IntentDurable, None)).await;
}
#[tokio::test]
async fn baseline_and_branch_crash_boundaries_fence_independent_readers_and_retain_exact_commit() {
    for branch in [None, Some("review")] {
        for point in [
            SourcePublicationPoint::IntentDurable,
            SourcePublicationPoint::FenceAcquired,
            SourcePublicationPoint::BeforeNativeWrite(0),
            SourcePublicationPoint::AfterNativeWrite(0),
            SourcePublicationPoint::BeforeCommitDecision,
            SourcePublicationPoint::CommitDecisionDurable,
            SourcePublicationPoint::FenceReleased,
        ] {
            Box::pin(case(point, branch)).await;
        }
    }
}

struct PauseReader {
    entered: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
    once: std::sync::atomic::AtomicBool,
}
impl SourcePublicationObserver for PauseReader {
    fn reached(&self, _: SourcePublicationPoint) -> StoreResult<()> {
        Ok(())
    }
    fn native_access_admitted(&self) -> StoreResult<()> {
        if self.once.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.wait();
            self.resume.wait();
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_inflight_independent_reader_refuses_a_crossed_publication_epoch() {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    let registration = SourceDocumentKey::new("reader-fence").unwrap();
    let control = store
        .source_compare_and_swap(&registration, None, b"intent", 128 * 1024 * 1024)
        .await
        .unwrap();
    let original = entity("original");
    let owner = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: trogon_atlas_core::refs::entity_id(&original).cloned(),
    };
    let current = store
        .create(
            pb::EntityKind::ReadModel,
            &original,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    let observed = pb::SourceObservedOwner {
        owner: Some(owner.clone()),
        uid: current.entity.system.as_ref().unwrap().uid.clone(),
        etag: current.etag,
    };
    let guard = pb::SourceNativeGuard {
        owner: Some(owner.clone()),
        condition: Some(pb::source_native_guard::Condition::Existing(
            observed.clone(),
        )),
    };
    let source = pb::SourceResourceKey {
        registration_id: registration.as_str().into(),
        category: "native".into(),
        key: "read-model".into(),
    };
    let publication = SourcePublication {
        registration,
        control_revision: control.revision,
        run_id: "publication".into(),
        plan_digest: "digest".into(),
        storage_limit: 128 * 1024 * 1024,
        finalization_reserve_bytes: 0,
        plan: pb::SourceRunPlan {
            native_guards: vec![guard.clone()],
            operations: vec![pb::SourceNativeOperation {
                mapping_id: "mapping".into(),
                source: Some(source.clone()),
                operation: Some(pb::source_native_operation::Operation::Put(
                    pb::SourcePutOperation {
                        entity: Some(entity("after")),
                        guard: Some(guard),
                    },
                )),
            }],
            ..Default::default()
        },
        values: vec![pb::SourceCommittedValue {
            mapping_id: "mapping".into(),
            source: Some(source),
            observed: Some(observed),
            ..Default::default()
        }],
    };
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let mut reader_config = configuration;
    reader_config.role = WriterRole::Reader;
    let reader = NatsStore::connect_with(reader_config)
        .await
        .unwrap()
        .with_source_publication_observer(Arc::new(PauseReader {
            entered: entered.clone(),
            resume: resume.clone(),
            once: std::sync::atomic::AtomicBool::new(true),
        }));
    let id = owner.id.unwrap();
    let reading =
        tokio::spawn(async move { reader.get(pb::EntityKind::ReadModel, &id, None).await });
    entered.wait();
    assert!(matches!(
        store.source_publish(&publication).await.unwrap(),
        SourcePublicationOutcome::Committed(_)
    ));
    resume.wait();
    assert!(matches!(
        reading.await.unwrap(),
        Err(StoreError::Unavailable(_))
    ));
}

async fn publication_fixture(store: &NatsStore, branch: Option<&str>) -> SourcePublication {
    let value = entity("before");
    let id = trogon_atlas_core::refs::entity_id(&value).unwrap().clone();
    let current = match store.get(pb::EntityKind::ReadModel, &id, branch).await {
        Ok(current) => current,
        Err(StoreError::NotFound) => {
            store
                .create(
                    pb::EntityKind::ReadModel,
                    &value,
                    WriteContext::scoped(branch),
                )
                .await
                .unwrap();
            store
                .get(pb::EntityKind::ReadModel, &id, branch)
                .await
                .unwrap()
        }
        Err(error) => panic!("fixture lookup: {error}"),
    };
    let registration = SourceDocumentKey::new("source-publication").unwrap();
    let doc = store
        .source_compare_and_swap(&registration, None, b"control-intent", 128 * 1024 * 1024)
        .await
        .unwrap();
    let owner = pb::EntityRef {
        kind: pb::EntityKind::ReadModel as i32,
        id: Some(id.clone()),
    };
    let observed = pb::SourceObservedOwner {
        owner: Some(owner.clone()),
        uid: current.entity.system.as_ref().unwrap().uid.clone(),
        etag: current.etag.clone(),
    };
    let guard = pb::SourceNativeGuard {
        owner: Some(owner.clone()),
        condition: Some(pb::source_native_guard::Condition::Existing(
            observed.clone(),
        )),
    };
    let source = pb::SourceResourceKey {
        registration_id: registration.as_str().into(),
        category: "model".into(),
        key: "read-model".into(),
    };
    SourcePublication {
        storage_limit: 128 * 1024 * 1024,
        finalization_reserve_bytes: 0,
        registration: registration.clone(),
        control_revision: doc.revision,
        run_id: "run".into(),
        plan_digest: "digest".into(),
        plan: pb::SourceRunPlan {
            branch: branch.map(str::to_owned),
            native_guards: vec![guard.clone()],
            operations: vec![pb::SourceNativeOperation {
                mapping_id: "mapping".into(),
                source: Some(source.clone()),
                operation: Some(pb::source_native_operation::Operation::Put(
                    pb::SourcePutOperation {
                        entity: Some(entity("committed")),
                        guard: Some(guard),
                    },
                )),
            }],
            ..Default::default()
        },
        values: vec![pb::SourceCommittedValue {
            mapping_id: "mapping".into(),
            source: Some(source),
            observed: Some(observed),
            ..Default::default()
        }],
    }
}

#[tokio::test]
async fn reader_resolution_refuses_before_mutation_and_publication_keeps_intended_postimage() {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    let baseline = store
        .create(
            pb::EntityKind::ReadModel,
            &entity("baseline"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store.create_branch("review", "").await.unwrap();
    store
        .update(
            pb::EntityKind::ReadModel,
            &entity("before"),
            &baseline.etag,
            false,
            WriteContext::scoped(Some("review")),
        )
        .await
        .unwrap();
    let mut publication = publication_fixture(&store, Some("review")).await;
    let id = trogon_atlas_core::refs::entity_id(&entity("before"))
        .unwrap()
        .clone();
    let before = store
        .get(pb::EntityKind::ReadModel, &id, Some("review"))
        .await
        .unwrap();
    let mut reader_configuration = configuration;
    reader_configuration.role = WriterRole::Reader;
    let reader = NatsStore::connect_with(reader_configuration).await.unwrap();
    let result = reader
        .resolve_branch_entry("review", pb::EntityKind::ReadModel, &id, true)
        .await;
    let after = store
        .get(pb::EntityKind::ReadModel, &id, Some("review"))
        .await
        .unwrap();
    assert_eq!(
        after.entity, before.entity,
        "Reader must leave exact branch postimage intact"
    );
    assert!(
        matches!(result, Err(StoreError::NotWriter { .. })),
        "Reader must be rejected before accessing branch mutation: {result:?}"
    );
    publication.control_revision = store
        .source_get(&publication.registration)
        .await
        .unwrap()
        .unwrap()
        .revision;
    let SourcePublicationOutcome::Committed(marker) =
        store.source_publish(&publication).await.unwrap()
    else {
        panic!("commit");
    };
    let after = store
        .get(pb::EntityKind::ReadModel, &id, Some("review"))
        .await
        .unwrap();
    assert_eq!(marker.values[0].entity.as_ref(), Some(&after.entity));
    assert_eq!(after.entity.kind, entity("committed").kind);
}

async fn source_usage(configuration: &NatsStoreConfig) -> u64 {
    use futures::StreamExt;
    let client = async_nats::connect(configuration.url.clone())
        .await
        .unwrap();
    let kv = async_nats::jetstream::new(client)
        .get_key_value(format!("{}-sources", configuration.batches_bucket))
        .await
        .unwrap();
    let mut keys = kv.keys().await.unwrap();
    let mut bytes = 0;
    while let Some(key) = keys.next().await {
        bytes += kv
            .get(key.unwrap())
            .await
            .unwrap()
            .map_or(0, |v| v.len() as u64);
    }
    bytes
}

#[tokio::test]
async fn expired_interrupted_publications_reclaim_all_staged_chunks_across_runs() {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    let mut publication = publication_fixture(&store, None).await;
    publication.storage_limit = 100_000;
    let initial = source_usage(&configuration).await;
    for (index, point) in [
        SourcePublicationPoint::PriorChunkDurable(0),
        SourcePublicationPoint::MarkerChunkDurable(0),
        SourcePublicationPoint::BeforeCommitDecision,
        SourcePublicationPoint::BeforeCommitDecision,
    ]
    .into_iter()
    .enumerate()
    {
        publication.run_id = format!("interrupted-{index}");
        let faulty = store
            .clone()
            .with_source_publication_observer(Arc::new(Crash(point)));
        assert!(faulty.source_publish(&publication).await.is_err());
        assert!(matches!(
            store
                .source_recover(
                    &publication.registration,
                    &publication.run_id,
                    &publication.plan_digest
                )
                .await
                .unwrap(),
            SourcePublicationOutcome::NotCommitted
        ));
        store
            .source_expire_transaction(
                &publication.registration,
                &publication.run_id,
                &publication.plan_digest,
            )
            .await
            .unwrap();
        assert!(
            source_usage(&configuration).await <= initial + 256,
            "expiration must reclaim unrecorded marker chunks"
        );
        // Rollback preserves content but advances native revision, so refresh the guard.
        let owner = publication.plan.native_guards[0].owner.clone().unwrap();
        let current = store
            .get(pb::EntityKind::ReadModel, owner.id.as_ref().unwrap(), None)
            .await
            .unwrap();
        let guard = pb::SourceNativeGuard {
            owner: Some(owner.clone()),
            condition: Some(pb::source_native_guard::Condition::Existing(
                pb::SourceObservedOwner {
                    owner: Some(owner),
                    uid: current.entity.system.as_ref().unwrap().uid.clone(),
                    etag: current.etag,
                },
            )),
        };
        publication.plan.native_guards[0] = guard.clone();
        if let Some(pb::source_native_operation::Operation::Put(put)) =
            publication.plan.operations[0].operation.as_mut()
        {
            put.guard = Some(guard);
        }
    }
    let control = store
        .source_compare_and_swap(
            &publication.registration,
            Some(publication.control_revision),
            &vec![9; 20_000],
            publication.storage_limit,
        )
        .await
        .unwrap();
    publication.control_revision = control.revision;
    publication.run_id = "after-expiration".into();
    assert!(
        matches!(
            store.source_publish(&publication).await.unwrap(),
            SourcePublicationOutcome::Committed(_)
        ),
        "reclaimed quota must admit a bounded new document and publication"
    );
    assert!(source_usage(&configuration).await <= publication.storage_limit);
}

struct PausePublication {
    entered: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
}
impl SourcePublicationObserver for PausePublication {
    fn reached(&self, point: SourcePublicationPoint) -> StoreResult<()> {
        if point == SourcePublicationPoint::IntentDurable {
            self.entered.wait();
            self.resume.wait();
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_cas_cannot_overlap_publication_guard_and_reserved_storage() {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    let mut publication = publication_fixture(&store, None).await;
    publication.storage_limit = 100_000;
    if let Some(pb::source_native_operation::Operation::Put(put)) =
        publication.plan.operations[0].operation.as_mut()
    {
        put.entity = Some(entity(&"m".repeat(20_000)));
    }
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let publishing_store =
        store
            .clone()
            .with_source_publication_observer(Arc::new(PausePublication {
                entered: entered.clone(),
                resume: resume.clone(),
            }));
    let publishing_value = publication.clone();
    let publishing =
        tokio::spawn(async move { publishing_store.source_publish(&publishing_value).await });
    entered.wait();
    let cas_store = store.clone();
    let cas_publication = publication.clone();
    let mut cas = tokio::spawn(async move {
        cas_store
            .source_compare_and_swap(
                &cas_publication.registration,
                Some(cas_publication.control_revision),
                &vec![7; 79_500],
                cas_publication.storage_limit,
            )
            .await
    });
    let overlapped = tokio::time::timeout(std::time::Duration::from_millis(100), &mut cas)
        .await
        .is_ok();
    resume.wait();
    let result = publishing.await.unwrap();
    if !overlapped {
        let _ = cas.await.unwrap();
    }
    assert!(
        !overlapped,
        "CAS completed after publication admission while its reserve was outstanding"
    );
    assert!(matches!(
        result.unwrap(),
        SourcePublicationOutcome::Committed(_)
    ));
    assert!(source_usage(&configuration).await <= publication.storage_limit);
}

async fn reopen(configuration: NatsStoreConfig) -> NatsStore {
    let store = NatsStore::connect_with(configuration).await.unwrap();
    for _ in 0..30 {
        if store.is_writer() {
            return store;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("writer takeover");
}

#[tokio::test]
async fn prepared_identity_survives_restart_and_matching_resume_uses_applying_revision() {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    let mut publication = publication_fixture(&store, None).await;
    let interrupted = store
        .clone()
        .with_source_publication_observer(Arc::new(Crash(SourcePublicationPoint::IntentDurable)));
    assert!(interrupted.source_prepare(&publication).await.is_err());
    drop(interrupted);
    drop(store);
    let store = reopen(configuration).await;
    store.source_prepare(&publication).await.unwrap();
    let applying = store
        .source_compare_and_swap(
            &publication.registration,
            Some(publication.control_revision),
            b"applying",
            publication.storage_limit,
        )
        .await
        .unwrap();
    publication.control_revision = applying.revision;
    store.source_prepare(&publication).await.unwrap();
    let mut mismatch = publication.clone();
    mismatch.plan_digest = "different".into();
    assert!(matches!(
        store.source_prepare(&mismatch).await,
        Err(StoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        store.source_publish(&mismatch).await,
        Err(StoreError::InvalidArgument(_))
    ));
    mismatch = publication.clone();
    mismatch.plan.branch = Some("other".into());
    assert!(matches!(
        store.source_publish(&mismatch).await,
        Err(StoreError::InvalidArgument(_))
    ));
    let SourcePublicationOutcome::Committed(marker) =
        store.source_publish(&publication).await.unwrap()
    else {
        panic!("prepared resume must commit");
    };
    let SourcePublicationOutcome::Committed(replayed) =
        store.source_publish(&publication).await.unwrap()
    else {
        panic!("idempotent commit");
    };
    assert_eq!(marker, replayed);
}

#[tokio::test]
async fn failed_applying_cas_or_stale_native_admission_has_definite_noncommit_only_with_prewrite() {
    for stale_native in [false, true] {
        let configuration = config(&trogon_atlas_testsupport::shared().await.url);
        let store = NatsStore::connect_with(configuration).await.unwrap();
        let mut publication = publication_fixture(&store, None).await;
        assert!(
            matches!(
                store
                    .source_recover(
                        &publication.registration,
                        &publication.run_id,
                        &publication.plan_digest
                    )
                    .await,
                Err(StoreError::Unavailable(_))
            ),
            "missing header must remain unknown"
        );
        store.source_prepare(&publication).await.unwrap();
        let changed = store
            .source_compare_and_swap(
                &publication.registration,
                Some(publication.control_revision),
                b"changed",
                publication.storage_limit,
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .source_compare_and_swap(
                    &publication.registration,
                    Some(publication.control_revision),
                    b"applying",
                    publication.storage_limit
                )
                .await,
            Err(StoreError::EtagMismatch { .. })
        ));
        if stale_native {
            publication.control_revision = changed.revision;
            let guard = &publication.plan.native_guards[0];
            let Some(pb::source_native_guard::Condition::Existing(observed)) = &guard.condition
            else {
                panic!("existing");
            };
            store
                .update(
                    pb::EntityKind::ReadModel,
                    &entity("later native edit"),
                    &observed.etag,
                    false,
                    WriteContext::baseline(),
                )
                .await
                .unwrap();
        }
        assert!(
            store.source_publish(&publication).await.is_err(),
            "publish must recheck current guards and revision after prepare"
        );
        assert!(
            store
                .source_expire_transaction(
                    &publication.registration,
                    &publication.run_id,
                    &publication.plan_digest
                )
                .await
                .is_err(),
            "PREWRITE must be settled before expiration"
        );
        assert!(matches!(
            store
                .source_recover(
                    &publication.registration,
                    &publication.run_id,
                    &publication.plan_digest
                )
                .await
                .unwrap(),
            SourcePublicationOutcome::NotCommitted
        ));
        let id = publication.plan.native_guards[0]
            .owner
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .unwrap();
        let current = store
            .get(pb::EntityKind::ReadModel, id, None)
            .await
            .unwrap();
        assert_eq!(
            current.entity.kind,
            entity(if stale_native {
                "later native edit"
            } else {
                "before"
            })
            .kind
        );
        store
            .source_expire_transaction(
                &publication.registration,
                &publication.run_id,
                &publication.plan_digest,
            )
            .await
            .unwrap();
        assert!(
            store
                .source_recover(
                    &publication.registration,
                    &publication.run_id,
                    &publication.plan_digest
                )
                .await
                .is_err(),
            "expired evidence must remain unknown"
        );
    }
}

#[tokio::test]
async fn recovering_prewrite_never_releases_a_different_transactions_busy_gate() {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration).await.unwrap();
    let mut first = publication_fixture(&store, None).await;
    first.run_id = "prewrite-only".into();
    store.source_prepare(&first).await.unwrap();
    let mut second = first.clone();
    second.run_id = "writing".into();
    let faulty = store
        .clone()
        .with_source_publication_observer(Arc::new(Crash(
            SourcePublicationPoint::AfterNativeWrite(0),
        )));
    assert!(faulty.source_publish(&second).await.is_err());
    assert!(matches!(
        store
            .source_recover(&first.registration, &first.run_id, &first.plan_digest)
            .await
            .unwrap(),
        SourcePublicationOutcome::NotCommitted
    ));
    let id = first.plan.native_guards[0]
        .owner
        .as_ref()
        .unwrap()
        .id
        .as_ref()
        .unwrap();
    assert!(
        matches!(
            store.get(pb::EntityKind::ReadModel, id, None).await,
            Err(StoreError::Unavailable(_))
        ),
        "another transaction's fence must stay busy"
    );
    assert!(matches!(
        store
            .source_recover(&second.registration, &second.run_id, &second.plan_digest)
            .await
            .unwrap(),
        SourcePublicationOutcome::NotCommitted
    ));
    assert_eq!(
        store
            .get(pb::EntityKind::ReadModel, id, None)
            .await
            .unwrap()
            .entity
            .kind,
        entity("before").kind
    );
}

fn named_entity(slug: &str, title: &str) -> pb::Entity {
    let mut value = entity(title);
    let Some(pb::entity::Kind::ReadModel(model)) = &mut value.kind else {
        unreachable!();
    };
    model.id.as_mut().unwrap().slug = slug.into();
    value
}

async fn mixed_case(branch: Option<&str>, point: SourcePublicationPoint) {
    let configuration = config(&trogon_atlas_testsupport::shared().await.url);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    for slug in ["read-model", "delete", "restore-tombstone"] {
        store
            .create(
                pb::EntityKind::ReadModel,
                &named_entity(slug, "original"),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    if let Some(branch) = branch {
        store.create_branch(branch, "").await.unwrap();
        let id = trogon_atlas_core::refs::entity_id(&named_entity("restore-tombstone", ""))
            .unwrap()
            .clone();
        store
            .delete(
                pb::EntityKind::ReadModel,
                &id,
                None,
                WriteContext::scoped(Some(branch)),
            )
            .await
            .unwrap();
    }
    let mut publication = publication_fixture(&store, branch).await;
    publication.plan.operations.clear();
    publication.plan.native_guards.clear();
    publication.values.clear();
    let client = async_nats::connect(configuration.url.clone())
        .await
        .unwrap();
    let js = async_nats::jetstream::new(client);
    let kv = js
        .get_key_value(if branch.is_some() {
            &configuration.branches_bucket
        } else {
            &configuration.bucket
        })
        .await
        .unwrap();
    let mut priors = Vec::new();
    let mut baseline = Vec::new();
    for (slug, delete) in [
        ("restore-tombstone", false),
        ("new", false),
        ("read-model", false),
        ("delete", true),
    ] {
        let value = named_entity(slug, "accepted");
        let id = trogon_atlas_core::refs::entity_id(&value).unwrap().clone();
        let owner = pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id.clone()),
        };
        let current = match store.get(pb::EntityKind::ReadModel, &id, branch).await {
            Ok(v) => Some(v),
            Err(StoreError::NotFound) => None,
            Err(error) => panic!("lookup {error}"),
        };
        let observed = pb::SourceObservedOwner {
            owner: Some(owner.clone()),
            uid: current
                .as_ref()
                .and_then(|v| v.entity.system.as_ref())
                .map(|s| s.uid.clone())
                .unwrap_or_default(),
            etag: current.as_ref().map(|v| v.etag.clone()).unwrap_or_default(),
        };
        let guard = pb::SourceNativeGuard {
            owner: Some(owner),
            condition: Some(
                current
                    .as_ref()
                    .map_or(pb::source_native_guard::Condition::Absent(true), |_| {
                        pb::source_native_guard::Condition::Existing(observed.clone())
                    }),
            ),
        };
        let operation = if delete {
            pb::source_native_operation::Operation::Delete(pb::SourceDeleteOperation {
                guard: Some(guard.clone()),
            })
        } else {
            pb::source_native_operation::Operation::Put(pb::SourcePutOperation {
                guard: Some(guard.clone()),
                entity: Some(value),
            })
        };
        let key = branch.map_or_else(
            || trogon_atlas_store::key::entity_key(pb::EntityKind::ReadModel, &id),
            |b| trogon_atlas_store::key::branch_delta_key(b, pb::EntityKind::ReadModel, &id),
        );
        priors.push((
            id.clone(),
            key.clone(),
            kv.get(key).await.unwrap(),
            current.map(|v| v.entity),
        ));
        baseline.push(
            store
                .get(pb::EntityKind::ReadModel, &id, None)
                .await
                .ok()
                .map(|v| v.entity),
        );
        publication.plan.native_guards.push(guard);
        publication.plan.operations.push(pb::SourceNativeOperation {
            mapping_id: slug.into(),
            operation: Some(operation),
            ..Default::default()
        });
        publication.values.push(pb::SourceCommittedValue {
            mapping_id: slug.into(),
            observed: Some(observed),
            ..Default::default()
        });
    }
    if branch.is_some() {
        assert!(priors[0].2.is_some(), "existing tombstone");
        assert!(
            priors[1].2.is_none() && priors[2].2.is_none() && priors[3].2.is_none(),
            "baseline entries have no branch delta"
        );
    }
    let faulty = store
        .clone()
        .with_source_publication_observer(Arc::new(Crash(point)));
    assert!(faulty.source_publish(&publication).await.is_err());
    drop(faulty);
    drop(store);
    let store = reopen(configuration).await;
    let outcome = store
        .source_recover(
            &publication.registration,
            &publication.run_id,
            &publication.plan_digest,
        )
        .await
        .unwrap();
    match outcome {
        SourcePublicationOutcome::NotCommitted => {
            assert_ne!(point, SourcePublicationPoint::CommitDecisionDurable);
            for (id, key, raw, old) in &priors {
                assert_eq!(
                    &kv.get(key).await.unwrap(),
                    raw,
                    "exact raw rollback including tombstone or absent delta: {key}"
                );
                let actual = store
                    .get(pb::EntityKind::ReadModel, id, branch)
                    .await
                    .ok()
                    .map(|v| v.entity);
                assert_eq!(&actual, old, "exact old UID and content");
            }
        }
        SourcePublicationOutcome::Committed(marker) => {
            assert_eq!(point, SourcePublicationPoint::CommitDecisionDurable);
            for ((id, _, _, _), value) in priors.iter().zip(&marker.values) {
                let actual = store
                    .get(pb::EntityKind::ReadModel, id, branch)
                    .await
                    .ok()
                    .map(|v| v.entity);
                assert_eq!(actual, value.entity);
                assert_eq!(value.deleted, id.slug == "delete");
                if let Some(actual) = actual {
                    assert_eq!(actual.kind, named_entity(&id.slug, "accepted").kind);
                }
            }
            let (id, _, _, _) = &priors[2];
            let current = store
                .get(pb::EntityKind::ReadModel, id, branch)
                .await
                .unwrap();
            let later = store
                .update(
                    pb::EntityKind::ReadModel,
                    &named_entity(&id.slug, "later native edit"),
                    &current.etag,
                    false,
                    WriteContext::scoped(branch),
                )
                .await
                .unwrap();
            let SourcePublicationOutcome::Committed(replayed) = store
                .source_recover(
                    &publication.registration,
                    &publication.run_id,
                    &publication.plan_digest,
                )
                .await
                .unwrap()
            else {
                panic!("committed replay");
            };
            assert_eq!(marker, replayed);
            assert_eq!(
                store
                    .get(pb::EntityKind::ReadModel, id, branch)
                    .await
                    .unwrap()
                    .entity,
                later.entity,
                "recovery must preserve later native edit"
            );
        }
    }
    if branch.is_some() {
        for ((id, _, _, _), old) in priors.iter().zip(&baseline) {
            assert_eq!(
                &store
                    .get(pb::EntityKind::ReadModel, id, None)
                    .await
                    .ok()
                    .map(|v| v.entity),
                old,
                "branch publication never mutates baseline"
            );
        }
    }
}

#[tokio::test]
async fn mixed_create_update_delete_rolls_back_exact_priors_and_retains_committed_postimages() {
    for branch in [None, Some("mixed")] {
        for point in [
            SourcePublicationPoint::AfterNativeWrite(2),
            SourcePublicationPoint::AfterNativeWrite(3),
            SourcePublicationPoint::CommitDecisionDurable,
        ] {
            Box::pin(mixed_case(branch, point)).await;
        }
    }
}

struct RegistrationStorageUsage {
    total_bytes: u64,
    prior_bytes: u64,
}

async fn registration_storage_usage(
    configuration: &NatsStoreConfig,
    registration: &SourceDocumentKey,
) -> RegistrationStorageUsage {
    use std::fmt::Write;

    use futures::StreamExt;
    use sha2::{Digest, Sha256};

    let hash = Sha256::digest(registration.as_str().as_bytes())
        .iter()
        .fold(String::new(), |mut output, byte| {
            write!(output, "{byte:02x}").unwrap();
            output
        });
    let root = format!("r.{hash}");
    let prefixes = [
        format!("c.{hash}."),
        format!("t.{hash}."),
        format!("x.t.{hash}."),
    ];
    let client = async_nats::connect(configuration.url.clone())
        .await
        .unwrap();
    let kv = async_nats::jetstream::new(client)
        .get_key_value(format!("{}-sources", configuration.batches_bucket))
        .await
        .unwrap();
    let mut keys = kv.keys().await.unwrap();
    let mut usage = RegistrationStorageUsage {
        total_bytes: 0,
        prior_bytes: 0,
    };
    while let Some(key) = keys.next().await {
        let key = key.unwrap();
        if key != root && !prefixes.iter().any(|prefix| key.starts_with(prefix)) {
            continue;
        }
        let bytes = kv
            .get(&key)
            .await
            .unwrap()
            .map_or(0, |value| value.len() as u64);
        usage.total_bytes += bytes;
        if key.starts_with(&prefixes[2]) && key.contains(".p.") {
            usage.prior_bytes += bytes;
        }
    }
    usage
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publication_accounts_for_encoded_transaction_chunk_references_within_registration_quota() {
    use prost::Message;

    let mut configuration = config(&trogon_atlas_testsupport::shared().await.url);
    configuration.lease_duration = std::time::Duration::from_secs(5);
    let store = NatsStore::connect_with(configuration.clone())
        .await
        .unwrap();
    let registration = SourceDocumentKey::new("transaction-metadata-quota").unwrap();
    let maximum = trogon_atlas_store::source::SOURCE_DOCUMENT_MAX_BYTES as u64;
    let control = store
        .source_compare_and_swap(&registration, None, b"control", maximum)
        .await
        .unwrap();
    let mut publication = SourcePublication {
        registration: registration.clone(),
        control_revision: control.revision,
        run_id: "metadata-probe-0".into(),
        plan_digest: "metadata-boundary".into(),
        plan: pb::SourceRunPlan::default(),
        values: Vec::new(),
        storage_limit: maximum,
        finalization_reserve_bytes: 0,
    };
    let old_title = "a".repeat(496_000);
    let new_title = "b".repeat(old_title.len());
    let mut predicted_values = Vec::new();
    for index in 0..90 {
        let slug = format!("metadata-{index:03}");
        let current = store
            .create(
                pb::EntityKind::ReadModel,
                &named_entity(&slug, &old_title),
                WriteContext::baseline(),
            )
            .await
            .unwrap();
        let id = trogon_atlas_core::refs::entity_id(&current.entity)
            .unwrap()
            .clone();
        let owner = pb::EntityRef {
            kind: pb::EntityKind::ReadModel as i32,
            id: Some(id),
        };
        let observed = pb::SourceObservedOwner {
            owner: Some(owner.clone()),
            uid: current.entity.system.as_ref().unwrap().uid.clone(),
            etag: current.etag,
        };
        let guard = pb::SourceNativeGuard {
            owner: Some(owner),
            condition: Some(pb::source_native_guard::Condition::Existing(
                observed.clone(),
            )),
        };
        publication.plan.native_guards.push(guard.clone());
        publication.plan.operations.push(pb::SourceNativeOperation {
            mapping_id: slug.clone(),
            operation: Some(pb::source_native_operation::Operation::Put(
                pb::SourcePutOperation {
                    entity: Some(named_entity(&slug, &new_title)),
                    guard: Some(guard),
                },
            )),
            ..Default::default()
        });
        let value = pb::SourceCommittedValue {
            mapping_id: slug.clone(),
            observed: Some(observed),
            entity: Some(current.entity.clone()),
            ..Default::default()
        };
        publication.values.push(value.clone());
        let mut predicted = value;
        let mut accepted = named_entity(&slug, &new_title);
        accepted.system = current.entity.system;
        predicted.entity = Some(accepted);
        predicted.observed.as_mut().unwrap().etag = u64::MAX.to_string();
        predicted_values.push(predicted);
    }
    store.source_prepare(&publication).await.unwrap();
    let prepared_bytes = registration_storage_usage(&configuration, &registration)
        .await
        .total_bytes;
    let probe = store
        .clone()
        .with_source_publication_observer(Arc::new(Crash(SourcePublicationPoint::FenceAcquired)));
    let error = probe
        .source_publish(&publication)
        .await
        .expect_err("probe must stop before native writes");
    assert!(
        error
            .to_string()
            .contains("injected source process interruption"),
        "probe must reach admission: {error}"
    );
    let prior_bytes = registration_storage_usage(&configuration, &registration)
        .await
        .prior_bytes;
    assert!(matches!(
        store
            .source_recover(&registration, &publication.run_id, &publication.plan_digest)
            .await
            .unwrap(),
        SourcePublicationOutcome::NotCommitted
    ));
    store
        .source_expire_transaction(&registration, &publication.run_id, &publication.plan_digest)
        .await
        .unwrap();
    let next_run = "metadata-apply-0";
    assert_eq!(publication.run_id.len(), next_run.len());
    publication.run_id = next_run.into();
    let predicted_marker = pb::SourceCommitMarker {
        run_id: publication.run_id.clone(),
        plan_digest: publication.plan_digest.clone(),
        values: predicted_values,
        native_operation_id: uuid::Uuid::now_v7().to_string(),
        committed_at_unix_ms: i64::MAX,
        ..Default::default()
    };
    let payload_bytes = prior_bytes + predicted_marker.encoded_len() as u64;
    assert!(
        payload_bytes > 127 * 1024 * 1024,
        "exercise the allowed large chunk-reference lists"
    );
    // Calibrate at the old admission boundary; the quota invariant is independent of that estimate.
    publication.storage_limit = prepared_bytes + payload_bytes + 65_536;
    assert!(publication.storage_limit <= maximum);
    drop(predicted_marker);
    match store.source_publish(&publication).await {
        Ok(SourcePublicationOutcome::Committed(_)) => {
            let actual = registration_storage_usage(&configuration, &registration)
                .await
                .total_bytes;
            assert!(actual <= publication.storage_limit, "committed registration uses {actual} bytes, exceeding quota {} after transaction metadata growth", publication.storage_limit);
        }
        Err(StoreError::Unavailable(message)) => {
            assert!(
                message.contains("storage budget"),
                "refusal must be quota admission: {message}"
            );
            assert!(
                registration_storage_usage(&configuration, &registration)
                    .await
                    .total_bytes
                    <= publication.storage_limit
            );
            for value in &publication.values {
                let owner = value.observed.as_ref().unwrap().owner.as_ref().unwrap();
                let current = store
                    .get(pb::EntityKind::ReadModel, owner.id.as_ref().unwrap(), None)
                    .await
                    .unwrap();
                assert_eq!(
                    current.entity,
                    *value.entity.as_ref().unwrap(),
                    "quota refusal must precede every native write"
                );
            }
            assert!(matches!(
                store
                    .source_recover(&registration, &publication.run_id, &publication.plan_digest)
                    .await
                    .unwrap(),
                SourcePublicationOutcome::NotCommitted
            ));
        }
        other => panic!(
            "unexpected quota-boundary disposition: {}",
            if other.is_ok() {
                "NotCommitted"
            } else {
                "non-quota error"
            }
        ),
    }
}

async fn source_store_review_configuration() -> NatsStoreConfig {
    let url = match std::env::var("ATLAS_SOURCE_STORE_REVIEW_TEST_NATS_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => trogon_atlas_testsupport::shared().await.url.clone(),
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("external test endpoint must be valid UTF-8")
        }
    };
    let scope = format!("source-store-review-{}", uuid::Uuid::now_v7().simple());
    eprintln!("SOURCE_STORE_REVIEW_SCOPE={scope}");
    let mut configuration = config(&url);
    configuration.bucket = format!("{scope}-entities");
    configuration.branches_bucket = format!("{scope}-branches");
    configuration.changesets_bucket = format!("{scope}-changesets");
    configuration.revisions_bucket = format!("{scope}-revisions");
    configuration.batches_bucket = format!("{scope}-batches");
    configuration.namespaces_bucket = format!("{scope}-namespaces");
    configuration.operations_bucket = format!("{scope}-operations");
    configuration.lease_bucket = format!("{scope}-lease");
    configuration.stream = scope.replace('-', "_").to_uppercase();
    configuration.subject_root = scope;
    configuration
}

async fn source_store_review_nats_client(configuration: &NatsStoreConfig) -> async_nats::Client {
    let options = match std::env::var("ATLAS_SOURCE_STORE_REVIEW_TEST_NATS_PASSWORD") {
        Ok(password) => async_nats::ConnectOptions::new().user_and_password(
            configuration
                .credentials
                .as_ref()
                .expect("external test credentials")
                .user()
                .to_owned(),
            password,
        ),
        Err(std::env::VarError::NotPresent) => async_nats::ConnectOptions::new(),
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("external test credentials must be valid UTF-8")
        }
    };
    options
        .connect(&configuration.url)
        .await
        .expect("scoped NATS fixture connection")
}

#[tokio::test]
async fn source_store_review_budget_refusal_preserves_control_revision_and_original_retry() {
    let configuration = source_store_review_configuration().await;
    let store = NatsStore::connect_with(configuration).await.unwrap();
    let publication = publication_fixture(&store, None).await;
    store.source_prepare(&publication).await.unwrap();
    let before = store
        .source_get(&publication.registration)
        .await
        .unwrap()
        .unwrap();
    let result = store
        .source_compare_and_swap(
            &publication.registration,
            Some(before.revision),
            &vec![5; 4_000],
            4_100,
        )
        .await;
    assert!(
        matches!(result, Err(StoreError::Unavailable(ref message)) if message.contains("storage budget")),
        "must reach physical quota refusal"
    );
    let after = store
        .source_get(&publication.registration)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.bytes, before.bytes);
    assert_eq!(
        after.revision, before.revision,
        "refused control CAS must preserve prepared publication revision"
    );
    let SourcePublicationOutcome::Committed(_) = store.source_publish(&publication).await.unwrap()
    else {
        panic!("quota refusal must not stale the prepared run");
    };
    let retried = store
        .source_compare_and_swap(
            &publication.registration,
            Some(before.revision),
            &vec![5; 4_000],
            publication.storage_limit,
        )
        .await
        .unwrap();
    assert_eq!(retried.bytes, vec![5; 4_000]);
    assert_ne!(retried.revision, before.revision);
}

fn source_store_review_hash(bytes: &[u8]) -> String {
    use std::fmt::Write;

    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut output, byte| {
            write!(output, "{byte:02x}").unwrap();
            output
        })
}

#[tokio::test]
async fn source_store_review_preflight_reclaims_orphans_and_counts_candidate_recreation() {
    for proposed_orphan in [false, true] {
        let configuration = source_store_review_configuration().await;
        let store = NatsStore::connect_with(configuration.clone())
            .await
            .unwrap();
        let registration = SourceDocumentKey::new("document-orphan-quota").unwrap();
        let before = store
            .source_compare_and_swap(&registration, None, &vec![1; 20_000], 128 * 1024 * 1024)
            .await
            .unwrap();
        let proposed = vec![2; 20_000];
        let client = source_store_review_nats_client(&configuration).await;
        let kv = async_nats::jetstream::new(client)
            .get_key_value(format!("{}-sources", configuration.batches_bucket))
            .await
            .unwrap();
        let orphan = if proposed_orphan {
            proposed.clone()
        } else {
            vec![3; 60_000]
        };
        let orphan_key = format!(
            "c.{}.{}",
            source_store_review_hash(registration.as_str().as_bytes()),
            source_store_review_hash(&orphan)
        );
        kv.put(&orphan_key, bytes::Bytes::from(orphan))
            .await
            .unwrap();
        if proposed_orphan {
            let result = store
                .source_compare_and_swap(&registration, Some(before.revision), &proposed, 30_000)
                .await;
            assert!(
                matches!(result, Err(StoreError::Unavailable(ref message)) if message.contains("storage budget")),
                "a proposed orphan will be collected and must be counted again when staged"
            );
            let unchanged = store.source_get(&registration).await.unwrap().unwrap();
            assert_eq!(unchanged.bytes, before.bytes);
            assert_eq!(unchanged.revision, before.revision);
            assert!(
                kv.get(&orphan_key).await.unwrap().is_some(),
                "refusal must precede orphan collection"
            );
        }
        let accepted = store
            .source_compare_and_swap(&registration, Some(before.revision), &proposed, 41_000)
            .await
            .unwrap();
        assert_eq!(accepted.bytes, proposed);
        use futures::StreamExt;
        let mut keys = kv.keys().await.unwrap();
        let mut physical_bytes = 0;
        while let Some(key) = keys.next().await {
            physical_bytes += kv
                .get(key.unwrap())
                .await
                .unwrap()
                .map_or(0, |bytes| bytes.len());
        }
        assert!(physical_bytes <= 41_000);
        if !proposed_orphan {
            assert!(kv.get(orphan_key).await.unwrap().is_none());
        }
    }
}

async fn source_store_review_history(store: &NatsStore, marker: &pb::SourceCommitMarker) {
    use trogon_atlas_store::store::{ChangesetPage, RevisionPage};
    let changesets = store
        .list_changesets(ChangesetPage {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        changesets
            .iter()
            .filter(|record| record.id == marker.native_operation_id)
            .count(),
        1
    );
    let changes = store.read_changes(0, 100).await.unwrap();
    assert_eq!(
        changes
            .iter()
            .filter(|record| record.changeset_id == marker.native_operation_id)
            .count(),
        usize::from(marker.branch.is_none())
    );
    let owner = marker.values[0]
        .observed
        .as_ref()
        .unwrap()
        .owner
        .as_ref()
        .unwrap();
    let revisions = store
        .list_entity_revisions(
            pb::EntityKind::ReadModel,
            owner.id.as_ref().unwrap(),
            RevisionPage {
                limit: 100,
                branch: marker.branch.as_deref(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let accepted = revisions
        .iter()
        .filter(|record| record.changeset_id == marker.native_operation_id)
        .collect::<Vec<_>>();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].after, marker.values[0].entity);
}

#[tokio::test]
async fn source_store_review_publish_replay_finalizes_durable_commit_and_preserves_marker_once() {
    for restart in [false, true] {
        let configuration = source_store_review_configuration().await;
        let store = NatsStore::connect_with(configuration.clone())
            .await
            .unwrap();
        let publication = publication_fixture(&store, None).await;
        let epoch = store.writer_status().epoch;
        let faulty = store
            .clone()
            .with_source_publication_observer(Arc::new(Crash(
                SourcePublicationPoint::CommitDecisionDurable,
            )));
        assert!(faulty.source_publish(&publication).await.is_err());
        let durable_marker = source_store_review_durable_marker(&configuration).await;
        drop(faulty);
        let store = if restart {
            drop(store);
            let reopened = reopen(configuration).await;
            assert!(reopened.writer_status().epoch.get() > epoch.get());
            reopened
        } else {
            store
        };
        let SourcePublicationOutcome::Committed(marker) =
            store.source_publish(&publication).await.unwrap()
        else {
            panic!("durable commit cannot roll back");
        };
        assert_eq!(marker, durable_marker);
        let owner = marker.values[0]
            .observed
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap();
        let current = store
            .get(pb::EntityKind::ReadModel, owner.id.as_ref().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(Some(&current.entity), marker.values[0].entity.as_ref());
        assert_eq!(current.entity.kind, entity("committed").kind);
        source_store_review_history(&store, &marker).await;
        let later = store
            .update(
                pb::EntityKind::ReadModel,
                &entity("later native edit"),
                &current.etag,
                false,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
        for _ in 0..2 {
            let SourcePublicationOutcome::Committed(replayed) =
                store.source_publish(&publication).await.unwrap()
            else {
                panic!("finalized replay");
            };
            assert_eq!(replayed, marker);
            assert_eq!(
                store
                    .get(pb::EntityKind::ReadModel, owner.id.as_ref().unwrap(), None)
                    .await
                    .unwrap()
                    .entity,
                later.entity
            );
            source_store_review_history(&store, &marker).await;
        }
    }
}

#[tokio::test]
async fn source_store_review_committed_replay_never_releases_an_unrelated_busy_gate() {
    let configuration = source_store_review_configuration().await;
    let store = NatsStore::connect_with(configuration).await.unwrap();
    let first = publication_fixture(&store, None).await;
    let SourcePublicationOutcome::Committed(marker) = store.source_publish(&first).await.unwrap()
    else {
        panic!("first commit");
    };
    let mut second = first.clone();
    second.run_id = "unrelated-busy-run".into();
    second.plan.operations.clear();
    second.plan.native_guards.clear();
    second.values.clear();
    let faulty = store
        .clone()
        .with_source_publication_observer(Arc::new(Crash(SourcePublicationPoint::FenceAcquired)));
    assert!(faulty.source_publish(&second).await.is_err());
    let _ = store.source_publish(&first).await;
    let owner = marker.values[0]
        .observed
        .as_ref()
        .unwrap()
        .owner
        .as_ref()
        .unwrap();
    assert!(
        matches!(
            store
                .get(pb::EntityKind::ReadModel, owner.id.as_ref().unwrap(), None)
                .await,
            Err(StoreError::Unavailable(_))
        ),
        "another run's gate must remain busy"
    );
    assert!(matches!(
        store
            .source_recover(&second.registration, &second.run_id, &second.plan_digest)
            .await
            .unwrap(),
        SourcePublicationOutcome::NotCommitted
    ));
    let SourcePublicationOutcome::Committed(replayed) = store.source_publish(&first).await.unwrap()
    else {
        panic!("first remains committed");
    };
    assert_eq!(replayed, marker);
    source_store_review_history(&store, &marker).await;
}

async fn source_store_review_durable_marker(
    configuration: &NatsStoreConfig,
) -> pb::SourceCommitMarker {
    use futures::StreamExt;
    use prost::Message;
    let client = source_store_review_nats_client(configuration).await;
    let kv = async_nats::jetstream::new(client)
        .get_key_value(format!("{}-sources", configuration.batches_bucket))
        .await
        .unwrap();
    let mut keys = kv.keys().await.unwrap();
    let mut chunks = Vec::new();
    while let Some(key) = keys.next().await {
        let key = key.unwrap();
        if key.starts_with("x.t.") && key.contains(".m.") {
            let index = key.rsplit('.').next().unwrap().parse::<usize>().unwrap();
            chunks.push((index, kv.get(key).await.unwrap().unwrap()));
        }
    }
    chunks.sort_by_key(|(index, _)| *index);
    let bytes = chunks
        .into_iter()
        .flat_map(|(_, bytes)| bytes.to_vec())
        .collect::<Vec<_>>();
    pb::SourceCommitMarker::decode(bytes.as_slice()).unwrap()
}

#[tokio::test]
async fn source_store_review_recovery_and_publish_share_finalization_on_baseline_and_branch() {
    for branch in [None, Some("review")] {
        let configuration = source_store_review_configuration().await;
        let store = NatsStore::connect_with(configuration.clone())
            .await
            .unwrap();
        if let Some(branch) = branch {
            store.create_branch(branch, "").await.unwrap();
        }
        let publication = publication_fixture(&store, branch).await;
        let faulty = store
            .clone()
            .with_source_publication_observer(Arc::new(Crash(
                SourcePublicationPoint::CommitDecisionDurable,
            )));
        assert!(faulty.source_publish(&publication).await.is_err());
        let durable_marker = source_store_review_durable_marker(&configuration).await;
        let SourcePublicationOutcome::Committed(marker) = store
            .source_recover(
                &publication.registration,
                &publication.run_id,
                &publication.plan_digest,
            )
            .await
            .unwrap()
        else {
            panic!("committed recovery");
        };
        assert_eq!(marker, durable_marker);
        let owner = marker.values[0]
            .observed
            .as_ref()
            .unwrap()
            .owner
            .as_ref()
            .unwrap();
        assert_eq!(
            store
                .get(
                    pb::EntityKind::ReadModel,
                    owner.id.as_ref().unwrap(),
                    branch
                )
                .await
                .unwrap()
                .entity,
            *marker.values[0].entity.as_ref().unwrap()
        );
        let SourcePublicationOutcome::Committed(replayed) =
            store.source_publish(&publication).await.unwrap()
        else {
            panic!("committed publication replay");
        };
        assert_eq!(replayed, marker);
        let SourcePublicationOutcome::Committed(recovered) = store
            .source_recover(
                &publication.registration,
                &publication.run_id,
                &publication.plan_digest,
            )
            .await
            .unwrap()
        else {
            panic!("committed recovery replay");
        };
        assert_eq!(recovered, marker);
        source_store_review_history(&store, &marker).await;
    }
}
