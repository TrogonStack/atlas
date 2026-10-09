//! Targeted coverage for `NatsStore` error-mapping arms not already
//! exercised by conformance.rs / nats_integration.rs / branches.rs:
//!
//! - `create` on a branch resurrecting a tombstoned key (merged view treats
//!   a tombstone as absent, so `create` must succeed rather than return
//!   `AlreadyExists`).
//! - `Backend`/`Unavailable` arms via a DEDICATED (non-shared) NATS
//!   container that is stopped mid-test, asserting `get`/`put` on an
//!   already-connected store return `StoreError::Backend` rather than
//!   panicking or hanging past the op timeout.
//! - `read_changes(_, 0)` short-circuits to an empty vec without touching
//!   `JetStream` at all.
//! - `prune_changes` is a documented no-op: it returns `Ok(())` and leaves
//!   the change log untouched (JetStream retention handles pruning
//!   server-side).
//! - `change_stream_stats` reports `None` oldest-timestamp on an empty
//!   stream and populated `messages`/`bytes`/`oldest` after a write.
//! - `batch_apply` (baseline) rollback restoring a `Put`'d key back to its
//!   prior *existing* value (the `kv.update` restore arm, as opposed to the
//!   already-covered "restore a freshly `Create`'d key back to absent via
//!   `kv.delete`" arm).
//! - `batch_apply` with `branch = Some(..)` (`batch_apply_branch`): success
//!   and rollback-on-failure, since branch-scoped batch mutation had zero
//!   prior coverage anywhere in the workspace.
//! - Corrupt-KV-entry tolerance: a raw `async_nats` client (bypassing
//!   `NatsStore` entirely) writes non-protobuf garbage directly into an
//!   entity key. `list()` must report `StoreError::CorruptEntry` for that
//!   key (via `decode_entity_logged`) rather than panicking, and `put_inner`
//!   must tolerate an undecodable prior value (logs and proceeds without a
//!   prior for the system stamp) rather than failing the write.
//!
//! EtagMismatch, NotFound, AlreadyExists (baseline), and the CAS-retry race
//! are already covered by conformance.rs, nats_integration.rs, and
//! branches.rs; this file intentionally does not duplicate them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_nats::jetstream;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{ListFilter, MutationOp},
    NatsStoreConfig, Store, StoreError, WriteContext,
};

fn event(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_create_resurrects_a_tombstoned_key_instead_of_already_exists() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "resurrect", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("resurrect-branch", "doc")
        .await
        .expect("create_branch");

    // Delete on the branch writes a tombstone delta hiding the baseline row.
    store
        .delete(
            pb::EntityKind::Event,
            &id("shop", "resurrect", 1),
            None,
            WriteContext::branch("resurrect-branch"),
        )
        .await
        .expect("branch delete must succeed");
    assert!(matches!(
        store
            .get(
                pb::EntityKind::Event,
                &id("shop", "resurrect", 1),
                Some("resurrect-branch"),
            )
            .await,
        Err(StoreError::NotFound)
    ));

    // `create` against the same key on the same branch must see the merged
    // view as absent (tombstone == absent) and succeed, NOT AlreadyExists.
    let written = store
        .create(
            pb::EntityKind::Event,
            &event("shop", "resurrect", 1, "Resurrected"),
            WriteContext::branch("resurrect-branch"),
        )
        .await
        .expect("create must resurrect past a tombstone");
    assert!(written.wrote);

    let fetched = store
        .get(
            pb::EntityKind::Event,
            &id("shop", "resurrect", 1),
            Some("resurrect-branch"),
        )
        .await
        .expect("resurrected entity must be readable");
    match fetched.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Resurrected"),
        other => panic!("unexpected: {other:?}"),
    }

    // Baseline is untouched by the branch-scoped resurrection.
    let baseline = store
        .get(pb::EntityKind::Event, &id("shop", "resurrect", 1), None)
        .await
        .expect("baseline get");
    match baseline.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Baseline"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn store_ops_map_to_backend_error_after_container_is_stopped() {
    // A DEDICATED, non-shared container: never touch trogon_atlas_testsupport
    // shared(), since stopping it would break every other test in the
    // binary. This is the only test in the workspace allowed to stop a
    // NATS container.
    let dedicated = trogon_atlas_testsupport::TestNats::start().await;
    let mut config = NatsStoreConfig::new(&dedicated.url);
    config.bucket = "test-stop-bucket".into();
    config.branches_bucket = "test-stop-branches".into();
    config.stream = "TEST_STOP_STREAM".into();
    config.subject_root = "test.stop".into();
    let store = trogon_atlas_store::NatsStore::connect_with(config)
        .await
        .expect("connect must succeed while the container is up");

    // Prove the store works before pulling the rug out.
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "before-stop", 1, "Ok"),
            WriteContext::baseline(),
        )
        .await
        .expect("create must succeed while the container is up");

    dedicated
        .stop()
        .await
        .expect("stopping the dedicated container");

    // Reads and writes against an unreachable backend must map to a
    // reported error (never panic, never hang past the store's own
    // operation timeout).
    let get_result = store
        .get(pb::EntityKind::Event, &id("shop", "before-stop", 1), None)
        .await;
    assert!(
        matches!(
            get_result,
            Err(StoreError::Backend(_) | StoreError::Unavailable(_))
        ),
        "expected Backend/Unavailable, got {get_result:?}"
    );

    let put_result = store
        .create(
            pb::EntityKind::Event,
            &event("shop", "after-stop", 1, "Unreachable"),
            WriteContext::baseline(),
        )
        .await;
    assert!(
        matches!(
            put_result,
            Err(StoreError::Backend(_) | StoreError::Unavailable(_))
        ),
        "expected Backend/Unavailable, got {put_result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_changes_zero_limit_returns_empty_without_hitting_jetstream() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "rc-zero", 1, "X"),
            WriteContext::baseline(),
        )
        .await
        .expect("create");

    let changes = store
        .read_changes(0, 0)
        .await
        .expect("read_changes(_, 0) must not error");
    assert!(
        changes.is_empty(),
        "limit == 0 must short-circuit to an empty vec, got {changes:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn prune_changes_is_a_documented_noop() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "prune-noop", 1, "X"),
            WriteContext::baseline(),
        )
        .await
        .expect("create");

    let before = store
        .read_changes(0, 100)
        .await
        .expect("read_changes before prune");
    assert!(!before.is_empty());

    store
        .prune_changes(before.last().unwrap().seq)
        .await
        .expect("prune_changes must always return Ok");

    let after = store
        .read_changes(0, 100)
        .await
        .expect("read_changes after prune");
    assert_eq!(
        before.len(),
        after.len(),
        "prune_changes is a documented no-op for NatsStore; the change log must be unaffected"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn change_stream_stats_reports_empty_then_populated() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let empty_stats = store
        .change_stream_stats()
        .await
        .expect("change_stream_stats must not error on a fresh stream");
    let empty_stats = empty_stats.expect("NatsStore always returns Some(stats)");
    assert_eq!(empty_stats.messages, 0);
    assert_eq!(
        empty_stats.oldest_message_unix_seconds, None,
        "an empty stream must report no oldest-message timestamp"
    );

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "stats-1", 1, "X"),
            WriteContext::baseline(),
        )
        .await
        .expect("create");

    let populated_stats = store
        .change_stream_stats()
        .await
        .expect("change_stream_stats must not error")
        .expect("NatsStore always returns Some(stats)");
    assert!(
        populated_stats.messages >= 1,
        "expected at least one message, got {populated_stats:?}"
    );
    assert!(populated_stats.bytes > 0);
    assert!(
        populated_stats.oldest_message_unix_seconds.is_some(),
        "a non-empty stream must report an oldest-message timestamp"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_rollback_restores_a_put_key_to_its_prior_existing_value() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("shop", "restore-existing", 1, "Original"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create(
            kind,
            &event("shop", "existing-2", 1, "Existing2"),
            WriteContext::baseline(),
        )
        .await
        .expect("second baseline create");

    // op0 overwrites an EXISTING key (so the rollback snapshot has
    // Some((entity, rev)) and must restore via kv.update, not kv.delete).
    // op1 fails, forcing rollback of op0.
    let ops = vec![
        MutationOp::Put {
            kind,
            entity: event("shop", "restore-existing", 1, "Overwritten"),
            if_match: None,
            force: true,
        },
        MutationOp::Create {
            kind,
            entity: event("shop", "existing-2", 1, "Clobber"),
        },
    ];
    let err = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .expect_err("batch must fail on the AlreadyExists create");
    assert!(matches!(err, StoreError::BatchFailed { index: 1, .. }));

    let restored = store
        .get(kind, &id("shop", "restore-existing", 1), None)
        .await
        .expect("get after rollback");
    match restored.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(
            e.title, "Original",
            "rollback must restore the pre-batch value via kv.update, not leave the overwrite in place"
        ),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_branch_applies_successfully() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create_branch("batch-branch-ok", "doc")
        .await
        .expect("create_branch");

    let ops = vec![
        MutationOp::Create {
            kind,
            entity: event("shop", "branch-batch-a", 1, "A"),
        },
        MutationOp::Create {
            kind,
            entity: event("shop", "branch-batch-b", 1, "B"),
        },
    ];
    let outcomes = store
        .batch_apply(&ops, WriteContext::branch("batch-branch-ok"))
        .await
        .expect("branch batch_apply must succeed");
    assert_eq!(outcomes.len(), 2);
    for outcome in &outcomes {
        assert!(matches!(
            outcome,
            trogon_atlas_store::store::MutationOutcome::Wrote { .. }
        ));
    }

    let a = store
        .get(
            kind,
            &id("shop", "branch-batch-a", 1),
            Some("batch-branch-ok"),
        )
        .await
        .expect("branch get a");
    match a.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "A"),
        other => panic!("unexpected: {other:?}"),
    }

    // Baseline must be untouched by a branch-scoped batch.
    assert!(matches!(
        store
            .get(kind, &id("shop", "branch-batch-a", 1), None)
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_branch_rolls_back_on_failure() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("shop", "branch-rollback-existing", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("batch-branch-rollback", "doc")
        .await
        .expect("create_branch");
    // Seed a pre-existing delta for the second key so the batch's second op
    // (a Create) collides with AlreadyExists and fails.
    store
        .create(
            kind,
            &event("shop", "branch-rollback-existing-delta", 1, "PriorDelta"),
            WriteContext::branch("batch-branch-rollback"),
        )
        .await
        .expect("seed prior branch delta");

    let ops = vec![
        MutationOp::Put {
            kind,
            entity: event(
                "shop",
                "branch-rollback-existing",
                1,
                "Overwritten-on-branch",
            ),
            if_match: None,
            force: true,
        },
        MutationOp::Create {
            kind,
            entity: event("shop", "branch-rollback-existing-delta", 1, "Clobber"),
        },
    ];
    let err = store
        .batch_apply(&ops, WriteContext::branch("batch-branch-rollback"))
        .await
        .expect_err("branch batch must fail on the AlreadyExists create");
    assert!(matches!(err, StoreError::BatchFailed { index: 1, .. }));

    // op0's branch delta must be rolled back to "no delta" (the key had no
    // delta before this batch started), so the merged view falls through to
    // baseline.
    let after = store
        .get(
            kind,
            &id("shop", "branch-rollback-existing", 1),
            Some("batch-branch-rollback"),
        )
        .await
        .expect("get after rollback");
    match after.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(
            e.title, "Baseline",
            "rollback must remove the branch delta created by this failed batch"
        ),
        other => panic!("unexpected: {other:?}"),
    }

    // op1's pre-existing delta must be restored to its pre-batch value.
    let delta_after = store
        .get(
            kind,
            &id("shop", "branch-rollback-existing-delta", 1),
            Some("batch-branch-rollback"),
        )
        .await
        .expect("get delta after rollback");
    match delta_after.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "PriorDelta"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_and_put_tolerate_a_corrupt_kv_entry_via_raw_byte_surgery() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("test-corrupt-{suffix}");
    config.branches_bucket = format!("test-corrupt-branches-{suffix}");
    config.stream = format!("TEST_CORRUPT_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.corrupt.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    config.lease_bucket = format!("test-corrupt-writer-lease-{suffix}");
    let store = trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .expect("connect must succeed");

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "corrupt-me", 1, "Fine"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");

    // Bypass NatsStore entirely: a raw async_nats client writes non-protobuf
    // garbage directly into the entity's KV key, simulating on-disk
    // corruption or a proto-skew write from a different service version.
    let key =
        trogon_atlas_store::key::entity_key(pb::EntityKind::Event, &id("shop", "corrupt-me", 1));
    let client = async_nats::connect(&nats.url).await.expect("nats connect");
    let js = jetstream::new(client);
    let kv = js
        .get_key_value(&config.bucket)
        .await
        .expect("bucket must exist");
    kv.put(&key, b"not a valid protobuf Entity".to_vec().into())
        .await
        .expect("raw byte-surgery put");

    // `list()` must surface CorruptEntry for the poisoned key rather than
    // panicking or silently dropping the whole scan.
    let list_result = store
        .list(ListFilter::default(), None, None)
        .await
        .expect_err("list must surface the corrupt entry as an error");
    assert!(
        matches!(list_result, StoreError::CorruptEntry { .. }),
        "expected CorruptEntry, got {list_result:?}"
    );

    // `put_inner` (via Store::put) must tolerate the undecodable prior value:
    // it logs and proceeds without a prior for the system stamp, rather than
    // failing the write outright.
    let put_result = store
        .put(
            pb::EntityKind::Event,
            &event("shop", "corrupt-me", 1, "Recovered"),
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("put must tolerate a corrupt prior entry and overwrite it");
    assert!(put_result.wrote);

    let recovered = store
        .get(pb::EntityKind::Event, &id("shop", "corrupt-me", 1), None)
        .await
        .expect("get after recovery put");
    match recovered.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Recovered"),
        other => panic!("unexpected: {other:?}"),
    }
}

/// `NatsStore::validate` must reject an entity whose oneof `kind` does not
/// match the requested `EntityKind` with `InvalidArgument`, before ever
/// touching JetStream.
#[tokio::test(flavor = "multi_thread")]
async fn create_rejects_entity_kind_mismatch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let err = store
        .create(
            pb::EntityKind::Command,
            &event("validate-ns", "kind-mismatch", 1, "Title"),
            WriteContext::baseline(),
        )
        .await
        .expect_err("an Event-shaped entity requested as Command must be rejected");
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("does not match request kind")),
        "expected an InvalidArgument kind-mismatch error, got {err:?}"
    );
}

/// `NatsStore::validate` must reject an entity with an empty
/// `id.namespace` with `InvalidArgument`.
#[tokio::test(flavor = "multi_thread")]
async fn create_rejects_empty_namespace() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let err = store
        .create(
            pb::EntityKind::Event,
            &event("", "empty-namespace", 1, "Title"),
            WriteContext::baseline(),
        )
        .await
        .expect_err("an entity with an empty namespace must be rejected");
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("namespace is required")),
        "expected an InvalidArgument namespace error, got {err:?}"
    );
}

/// `NatsStore::validate` must reject an entity with an empty `id.slug` with
/// `InvalidArgument`.
#[tokio::test(flavor = "multi_thread")]
async fn create_rejects_empty_slug() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let err = store
        .create(
            pb::EntityKind::Event,
            &event("validate-ns", "", 1, "Title"),
            WriteContext::baseline(),
        )
        .await
        .expect_err("an entity with an empty slug must be rejected");
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("slug is required")),
        "expected an InvalidArgument slug error, got {err:?}"
    );
}

/// `NatsStore::change_events_lost` and `change_tail_reconnects` are public
/// diagnostic counters; on a healthy freshly-connected store both must read
/// zero (no lost change events, no reconnects yet).
#[tokio::test(flavor = "multi_thread")]
async fn change_diagnostics_counters_start_at_zero_on_a_healthy_store() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("diag-bucket-{suffix}");
    config.branches_bucket = format!("diag-branches-{suffix}");
    config.stream = format!("DIAG_STREAM_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("diag.subject.{suffix}");
    let store = trogon_atlas_store::NatsStore::connect_with(config)
        .await
        .expect("connect_with must succeed against the shared NATS container");

    assert_eq!(store.change_events_lost(), 0);
    assert_eq!(store.change_tail_reconnects(), 0);
}

/// `NatsStoreConfig::validate` (invoked by `connect_with` before any network
/// I/O) must reject an empty `bucket` name with `InvalidArgument`. Uses a
/// bogus URL since validation fails before the store ever attempts to
/// connect.
#[tokio::test(flavor = "multi_thread")]
async fn connect_with_rejects_empty_bucket_name() {
    let mut config = NatsStoreConfig::new("nats://unused.invalid:4222");
    config.bucket = String::new();
    let result = trogon_atlas_store::NatsStore::connect_with(config).await;
    assert!(
        result.is_err(),
        "an empty bucket name must be rejected before connecting"
    );
    let err = result.err().unwrap();
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("bucket") && msg.contains("must not be empty")),
        "expected an InvalidArgument bucket error, got {err:?}"
    );
}

/// `NatsStoreConfig::validate` must reject a bucket name containing a
/// character outside `[A-Za-z0-9_-]` with `InvalidArgument`.
#[tokio::test(flavor = "multi_thread")]
async fn connect_with_rejects_bucket_name_with_invalid_character() {
    let mut config = NatsStoreConfig::new("nats://unused.invalid:4222");
    config.bucket = "bad bucket name".into();
    let result = trogon_atlas_store::NatsStore::connect_with(config).await;
    assert!(
        result.is_err(),
        "a bucket name with a space must be rejected"
    );
    let err = result.err().unwrap();
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("bucket") && msg.contains("invalid character")),
        "expected an InvalidArgument bucket error, got {err:?}"
    );
}

/// `NatsStoreConfig::validate` must reject a `subject_root` containing a
/// character outside `[A-Za-z0-9_.-]` with `InvalidArgument`. `subject_root`
/// permits `.` (unlike `bucket`/`stream`/`branches_bucket`), so this
/// exercises the dedicated `validate_nats_subject_root` character set.
#[tokio::test(flavor = "multi_thread")]
async fn connect_with_rejects_subject_root_with_invalid_character() {
    let mut config = NatsStoreConfig::new("nats://unused.invalid:4222");
    config.subject_root = "bad/subject".into();
    let result = trogon_atlas_store::NatsStore::connect_with(config).await;
    assert!(
        result.is_err(),
        "a subject_root with a slash must be rejected"
    );
    let err = result.err().unwrap();
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("subject_root") && msg.contains("invalid character")),
        "expected an InvalidArgument subject_root error, got {err:?}"
    );
}

/// `list_branch_merged`'s own `latest_versions_only` dedup (distinct from the
/// baseline `list()` path's dedup, which `nats_integration.rs`'s
/// `list_latest_versions_only` already covers) must keep only the
/// highest-version delta row for a branch-scoped list.
#[tokio::test(flavor = "multi_thread")]
async fn list_branch_merged_latest_versions_only_keeps_highest_version() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("latest-branch", "doc")
        .await
        .expect("create_branch");
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "branch-latest", 1, "v1"),
            WriteContext::branch("latest-branch"),
        )
        .await
        .expect("branch create v1");
    store
        .put(
            pb::EntityKind::Event,
            &event("shop", "branch-latest", 2, "v2"),
            true,
            WriteContext::branch("latest-branch"),
        )
        .await
        .expect("branch put v2 (force, different version key)");

    let latest = store
        .list(
            ListFilter {
                kinds: &[pb::EntityKind::Event],
                namespaces: &[],
                latest_versions_only: true,
            },
            None,
            Some("latest-branch"),
        )
        .await
        .expect("branch-scoped list");
    assert_eq!(
        latest.len(),
        1,
        "only the highest version must survive the branch-merged dedup"
    );
    match latest[0].entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "v2"),
        ref other => panic!("unexpected: {other:?}"),
    }
}

/// `ensure_changes_stream` must refuse to start when every retention knob
/// (`changes_max_age`, `changes_max_msgs`, `changes_max_bytes`) is `None`,
/// rather than silently creating an unbounded change stream.
#[tokio::test(flavor = "multi_thread")]
async fn connect_with_rejects_unbounded_changes_stream_retention() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("unbounded-bucket-{suffix}");
    config.branches_bucket = format!("unbounded-branches-{suffix}");
    config.stream = format!(
        "UNBOUNDED_STREAM_{}",
        suffix.replace('-', "_").to_uppercase()
    );
    config.subject_root = format!("unbounded.subject.{suffix}");
    config.changes_max_age = None;
    config.changes_max_msgs = None;
    config.changes_max_bytes = None;

    let result = trogon_atlas_store::NatsStore::connect_with(config).await;
    assert!(
        result.is_err(),
        "a changes stream with no retention configured at all must be rejected"
    );
    let err = result.err().unwrap();
    assert!(
        matches!(err, StoreError::InvalidArgument(ref msg) if msg.contains("no retention configured")),
        "expected an InvalidArgument retention error, got {err:?}"
    );
}

/// `branch_write_delta`'s "revise signaled a no-op" arm: `Store::put` with
/// `force = false` against a branch delta that is already semantically
/// identical must skip the write (`wrote == false`, existing etag returned)
/// rather than create a new delta revision.
#[tokio::test(flavor = "multi_thread")]
async fn branch_put_without_force_is_a_noop_when_content_is_unchanged() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("noop-branch", "doc")
        .await
        .expect("create_branch");
    let first = store
        .create(
            pb::EntityKind::Event,
            &event("shop", "branch-noop", 1, "Same"),
            WriteContext::branch("noop-branch"),
        )
        .await
        .expect("branch create");
    assert!(first.wrote);

    let second = store
        .put(
            pb::EntityKind::Event,
            &event("shop", "branch-noop", 1, "Same"),
            false,
            WriteContext::branch("noop-branch"),
        )
        .await
        .expect("branch put with identical content must succeed as a no-op");
    assert!(
        !second.wrote,
        "put with semantically identical content and force=false must be a no-op"
    );
    assert_eq!(
        second.etag, first.etag,
        "a no-op put must report the existing etag unchanged"
    );
}

/// `delete`'s branch-scoped path (`branch_write_delta`'s tombstone `revise`
/// closure) must reject a stale `expected_etag` with `EtagMismatch`, mirroring
/// baseline `delete`'s etag-guard behavior, rather than tombstoning
/// unconditionally.
#[tokio::test(flavor = "multi_thread")]
async fn branch_delete_rejects_stale_expected_etag() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("delete-etag-branch", "doc")
        .await
        .expect("create_branch");
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "branch-delete-etag", 1, "Title"),
            WriteContext::branch("delete-etag-branch"),
        )
        .await
        .expect("branch create");

    let result = store
        .delete(
            pb::EntityKind::Event,
            &id("shop", "branch-delete-etag", 1),
            Some("not-the-real-etag"),
            WriteContext::branch("delete-etag-branch"),
        )
        .await;
    assert!(
        matches!(result, Err(StoreError::EtagMismatch { .. })),
        "expected EtagMismatch for a stale branch delete etag, got {result:?}"
    );

    // The entity must still be present: the rejected delete must not have
    // tombstoned it.
    let still_there = store
        .get(
            pb::EntityKind::Event,
            &id("shop", "branch-delete-etag", 1),
            Some("delete-etag-branch"),
        )
        .await;
    assert!(
        still_there.is_ok(),
        "a rejected delete must not tombstone the entity"
    );
}
