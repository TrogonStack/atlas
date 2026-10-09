//! Integration tests against a real NATS server with `JetStream`.
//!
//! The container is started automatically via trogon-atlas-testsupport;
//! no external broker or environment variables are required.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use async_nats::jetstream::{self, kv::Config as KvConfig};
use tokio::task;
use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{ChangeKind, ListFilter, MutationOp, MutationOutcome},
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

fn entity_ref(ns: &str, slug: &str, version: u64) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, slug, version)),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn crud_roundtrip() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    let e = event("shop", "order-placed", 1, "Order placed");

    let etag = store
        .create(kind, &e, WriteContext::baseline())
        .await
        .unwrap()
        .etag;
    assert_ne!(etag, "");
    assert!(matches!(
        store.create(kind, &e, WriteContext::baseline()).await,
        Err(StoreError::AlreadyExists)
    ));

    let stored = store
        .get(kind, &id("shop", "order-placed", 1), None)
        .await
        .unwrap();
    match stored.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => {
            assert_eq!(e.title, "Order placed");
        }
        other => panic!("unexpected entity kind: {other:?}"),
    }
    assert_eq!(stored.etag, etag);

    let updated = event("shop", "order-placed", 1, "Order placed v2");
    let new_etag = store
        .update(kind, &updated, &etag, true, WriteContext::baseline())
        .await
        .unwrap()
        .etag;
    assert_ne!(new_etag, etag);
    assert!(matches!(
        store
            .update(kind, &updated, &etag, true, WriteContext::baseline())
            .await,
        Err(StoreError::EtagMismatch { .. })
    ));

    store
        .delete(
            kind,
            &id("shop", "order-placed", 1),
            Some(&new_etag),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    assert!(matches!(
        store.get(kind, &id("shop", "order-placed", 1), None).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_filters_by_kind_and_namespace() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "a", 1, "A"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .create(
            pb::EntityKind::Event,
            &event("billing", "b", 1, "B"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let all = store.list(ListFilter::default(), None, None).await.unwrap();
    assert_eq!(all.len(), 2);

    let namespaces = vec!["shop".to_string()];
    let filtered = store
        .list(
            ListFilter {
                kinds: &[pb::EntityKind::Event],
                namespaces: &namespaces,
                latest_versions_only: false,
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn change_log_records_and_reads() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let start = store.current_change_seq().await.unwrap();
    let s1 = store
        .record_change(ChangeKind::Put, &entity_ref("shop", "x", 1), None)
        .await
        .unwrap();
    let s2 = store
        .record_change(ChangeKind::Delete, &entity_ref("shop", "x", 1), None)
        .await
        .unwrap();
    assert!(s2 > s1);

    let records = store.read_changes(start, 10).await.unwrap();
    assert_eq!(records.len(), 2);
    assert!(matches!(records[0].kind, ChangeKind::Put));
    assert!(matches!(records[1].kind, ChangeKind::Delete));
    assert_eq!(store.current_change_seq().await.unwrap(), s2);
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_delivers_live_changes() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut rx = store.subscribe();

    let seq = store
        .record_change(ChangeKind::Put, &entity_ref("shop", "live", 1), None)
        .await
        .unwrap();

    let record = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("live change should arrive within 10s")
        .expect("subscription should stay open");
    assert_eq!(record.seq, seq);
    assert!(matches!(record.kind, ChangeKind::Put));
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_rolls_back_on_failure() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("shop", "existing", 1, "Existing"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let ops = vec![
        MutationOp::Put {
            kind,
            entity: event("shop", "new", 1, "New"),
            if_match: None,
            force: true,
        },
        MutationOp::Create {
            kind,
            entity: event("shop", "existing", 1, "Clobber"),
        },
    ];
    let err = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .expect_err("batch must fail");
    let index = match &err {
        StoreError::BatchFailed { index, .. } => *index,
        other => panic!("expected BatchFailed, got: {other:?}"),
    };
    assert_eq!(index, 1);

    assert!(matches!(
        store.get(kind, &id("shop", "new", 1), None).await,
        Err(StoreError::NotFound)
    ));
    let untouched = store
        .get(kind, &id("shop", "existing", 1), None)
        .await
        .unwrap();
    match untouched.entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Existing"),
        other => panic!("unexpected entity: {other:?}"),
    }
}

/// Compensating rollback must restore keys the batch actually wrote, not
/// rewrite every key that appeared in the snapshot. Rewriting an unmutated
/// key (e.g. the Create that failed with AlreadyExists) advances its etag
/// and silently invalidates callers holding the pre-batch revision.
#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_rollback_must_not_advance_unmutated_etags() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    let etag_before = store
        .create(
            kind,
            &event("shop", "etag-hold", 1, "Existing"),
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;

    let ops = vec![
        MutationOp::Put {
            kind,
            entity: event("shop", "etag-new", 1, "New"),
            if_match: None,
            force: true,
        },
        MutationOp::Create {
            kind,
            entity: event("shop", "etag-hold", 1, "Clobber"),
        },
    ];
    let err = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .expect_err("batch must fail on Create AlreadyExists");
    assert!(
        matches!(err, StoreError::BatchFailed { index: 1, .. }),
        "expected BatchFailed at index 1, got {err:?}"
    );

    assert!(matches!(
        store.get(kind, &id("shop", "etag-new", 1), None).await,
        Err(StoreError::NotFound)
    ));
    let after = store
        .get(kind, &id("shop", "etag-hold", 1), None)
        .await
        .expect("existing entity must remain");
    assert_eq!(
        after.etag, etag_before,
        "rollback must not rewrite keys the batch never mutated (etag must stay put)"
    );
    match after.entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Existing"),
        other => panic!("unexpected entity: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get_returns_present_and_absent() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("shop", "present", 1, "Present"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let keys = vec![
        (kind, id("shop", "present", 1)),
        (kind, id("shop", "absent", 1)),
    ];
    let results = store.batch_get(&keys, None).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].is_some(), "present key must be returned");
    assert!(results[1].is_none(), "absent key must be None");
    match &results[0].as_ref().unwrap().entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Present"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_latest_versions_only() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("shop", "evt", 1, "v1"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .create(
            kind,
            &event("shop", "evt", 2, "v2"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let all = store
        .list(trogon_atlas_store::store::ListFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(
        all.len(),
        2,
        "both versions must be returned without filter"
    );

    let latest = store
        .list(
            trogon_atlas_store::store::ListFilter {
                kinds: &[kind],
                namespaces: &[],
                latest_versions_only: true,
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(latest.len(), 1, "only the latest version must be returned");
    match &latest[0].entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "v2"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_multi_kind() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    fn command(ns: &str, slug: &str, version: u64, title: &str) -> pb::Entity {
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Command(pb::Command {
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

    let ops = vec![
        MutationOp::Create {
            kind: pb::EntityKind::Event,
            entity: event("shop", "ordered", 1, "Ordered"),
        },
        MutationOp::Create {
            kind: pb::EntityKind::Command,
            entity: command("shop", "place-order", 1, "Place order"),
        },
    ];
    let outcomes = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .expect("multi-kind batch must succeed");
    assert_eq!(outcomes.len(), 2);

    let ev = store
        .get(pb::EntityKind::Event, &id("shop", "ordered", 1), None)
        .await
        .unwrap();
    let cmd = store
        .get(pb::EntityKind::Command, &id("shop", "place-order", 1), None)
        .await
        .unwrap();
    match ev.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Ordered"),
        other => panic!("unexpected: {other:?}"),
    }
    match cmd.entity.kind {
        Some(pb::entity::Kind::Command(ref c)) => assert_eq!(c.title, "Place order"),
        other => panic!("unexpected: {other:?}"),
    }
}

// Item 18: CAS retry in put_inner under a concurrent writer.
// Two tasks race to put the same key; both must eventually succeed because
// put_inner retries the CAS loop on WrongLastRevision.
#[tokio::test(flavor = "multi_thread")]
async fn put_cas_retry_under_concurrent_writer() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("shop", "concurrent-cas", 1, "v0"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let s1 = store.clone();
    let s2 = store.clone();
    let t1 = task::spawn(async move {
        s1.put(
            kind,
            &event("shop", "concurrent-cas", 1, "writer-1"),
            true,
            WriteContext::baseline(),
        )
        .await
    });
    let t2 = task::spawn(async move {
        s2.put(
            kind,
            &event("shop", "concurrent-cas", 1, "writer-2"),
            true,
            WriteContext::baseline(),
        )
        .await
    });
    let (r1, r2) = tokio::join!(t1, t2);
    r1.expect("task1 panicked").expect("writer-1 put failed");
    r2.expect("task2 panicked").expect("writer-2 put failed");
}

// Item 19: record_change ChangeEventLost error variant compiles and is reachable.
#[test]
fn change_event_lost_variant_exists() {
    let err = StoreError::ChangeEventLost;
    let msg = err.to_string();
    assert!(
        msg.contains("change event"),
        "ChangeEventLost message should be descriptive"
    );
}

// Item 21: read_changes on an empty stream returns an empty vec.
#[tokio::test(flavor = "multi_thread")]
async fn read_changes_on_empty_stream() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let records = store
        .read_changes(0, 100)
        .await
        .expect("read_changes must not error on empty stream");
    assert!(records.is_empty(), "empty stream must return empty vec");
}

// Item 22: unique_suffix is collision-free under concurrent calls.
#[tokio::test(flavor = "multi_thread")]
async fn unique_suffix_is_collision_free() {
    use std::collections::HashSet;
    let mut handles = Vec::new();
    for _ in 0..50 {
        handles.push(task::spawn(async move {
            trogon_atlas_testsupport::unique_suffix_for_test()
        }));
    }
    let mut seen = HashSet::new();
    for h in handles {
        let s = h.await.expect("task panicked");
        assert!(seen.insert(s.clone()), "duplicate suffix: {s}");
    }
}

// Item 8a: batch_apply with two ops touching the same key (Create then Put)
// asserts final state and rollback behavior on subsequent failure.
#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_create_then_put_same_key() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;

    let ops = vec![
        MutationOp::Create {
            kind,
            entity: event("batch-same", "key-cp", 1, "created"),
        },
        MutationOp::Put {
            kind,
            entity: event("batch-same", "key-cp", 1, "overwritten"),
            if_match: None,
            force: true,
        },
    ];
    let outcomes = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .expect("batch must succeed");
    assert_eq!(outcomes.len(), 2);

    let stored = store
        .get(kind, &id("batch-same", "key-cp", 1), None)
        .await
        .unwrap();
    match stored.entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "overwritten"),
        other => panic!("unexpected entity: {other:?}"),
    }

    // A subsequent Create on the same key must fail with AlreadyExists and
    // the batch must roll back without disturbing the committed state.
    let ops2 = vec![MutationOp::Create {
        kind,
        entity: event("batch-same", "key-cp", 1, "clobber"),
    }];
    let err = store
        .batch_apply(&ops2, WriteContext::baseline())
        .await
        .expect_err("duplicate create must fail");
    let (index, source) = match err {
        StoreError::BatchFailed { index, source } => (index, *source),
        other => panic!("expected BatchFailed, got: {other:?}"),
    };
    assert_eq!(index, 0);
    assert!(matches!(source, StoreError::AlreadyExists));

    let stored_after = store
        .get(kind, &id("batch-same", "key-cp", 1), None)
        .await
        .unwrap();
    match stored_after.entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "overwritten"),
        other => panic!("unexpected entity after rollback: {other:?}"),
    }
}

/// When two `update` calls race on the same expected etag, the loser must
/// report `EtagMismatch.found` as the revision that actually won, not the
/// stale pre-CAS read (which equals `expected` and hides the conflict).
#[tokio::test(flavor = "multi_thread")]
async fn update_cas_race_found_must_differ_from_expected() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    let etag0 = store
        .create(
            kind,
            &event("race", "etag-found", 1, "v0"),
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;

    let s1 = store.clone();
    let s2 = store.clone();
    let e1 = etag0.clone();
    let e2 = etag0.clone();
    let t1 = task::spawn(async move {
        s1.update(
            kind,
            &event("race", "etag-found", 1, "writer-a"),
            &e1,
            true,
            WriteContext::baseline(),
        )
        .await
    });
    let t2 = task::spawn(async move {
        s2.update(
            kind,
            &event("race", "etag-found", 1, "writer-b"),
            &e2,
            true,
            WriteContext::baseline(),
        )
        .await
    });

    let (r1, r2) = tokio::join!(t1, t2);
    let r1 = r1.expect("task1 panicked");
    let r2 = r2.expect("task2 panicked");

    let (winner, loser) = match (&r1, &r2) {
        (Ok(w), Err(e)) | (Err(e), Ok(w)) => (w, e),
        other => panic!("expected one Ok and one Err, got {other:?}"),
    };
    assert_ne!(winner.etag, etag0, "winner must advance the etag");

    match loser {
        StoreError::EtagMismatch { expected, found } => {
            assert_eq!(expected, &etag0);
            assert_ne!(
                found, expected,
                "found must be the post-race revision, not the stale pre-CAS \
                 read (which equals expected and hides the conflict)"
            );
            assert_eq!(
                found, &winner.etag,
                "found should report the revision that actually won the CAS"
            );
        }
        other => panic!("expected EtagMismatch, got {other:?}"),
    }
}

/// Conditional batch Put CAS failures must report `EtagMismatch.found` as the
/// live post-race revision (same contract as `update` after the CAS found fix),
/// not an empty string that hides the conflict from callers.
#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_put_cas_race_found_must_differ_from_expected() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    let etag0 = store
        .create(
            kind,
            &event("race", "batch-etag-found", 1, "v0"),
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;

    let s1 = store.clone();
    let s2 = store.clone();
    let e1 = etag0.clone();
    let e2 = etag0.clone();
    let t1 = task::spawn(async move {
        s1.batch_apply(
            &[MutationOp::Put {
                kind,
                entity: event("race", "batch-etag-found", 1, "writer-a"),
                if_match: Some(e1),
                force: true,
            }],
            WriteContext::baseline(),
        )
        .await
    });
    let t2 = task::spawn(async move {
        s2.batch_apply(
            &[MutationOp::Put {
                kind,
                entity: event("race", "batch-etag-found", 1, "writer-b"),
                if_match: Some(e2),
                force: true,
            }],
            WriteContext::baseline(),
        )
        .await
    });

    let (r1, r2) = tokio::join!(t1, t2);
    let r1 = r1.expect("task1 panicked");
    let r2 = r2.expect("task2 panicked");

    let (winner_etag, loser) = match (&r1, &r2) {
        (Ok(outcomes), Err(e)) => {
            let MutationOutcome::Wrote { etag, .. } = &outcomes[0] else {
                panic!("winner must be Wrote, got {outcomes:?}");
            };
            (etag.clone(), e)
        }
        (Err(e), Ok(outcomes)) => {
            let MutationOutcome::Wrote { etag, .. } = &outcomes[0] else {
                panic!("winner must be Wrote, got {outcomes:?}");
            };
            (etag.clone(), e)
        }
        other => panic!("expected one Ok and one Err, got {other:?}"),
    };
    assert_ne!(winner_etag, etag0, "winner must advance the etag");

    let StoreError::BatchFailed { source, .. } = loser else {
        panic!("expected BatchFailed, got {loser:?}");
    };
    match source.as_ref() {
        StoreError::EtagMismatch { expected, found } => {
            assert_eq!(expected, &etag0);
            assert_ne!(
                found, expected,
                "found must be the post-race revision, not the stale pre-CAS read"
            );
            assert_eq!(
                found, &winner_etag,
                "found should report the revision that actually won the CAS"
            );
        }
        other => panic!("expected EtagMismatch source, got {other:?}"),
    }
}

/// `batch_apply` Create over an empty-Put tombstone must succeed (merged
/// view is absent), matching the single-op `create` tombstone path.
#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_create_over_empty_put_tombstone() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("test-{suffix}");
    config.branches_bucket = format!("test-branches-{suffix}");
    config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    config.lease_bucket = format!("test-writer-lease-{suffix}");

    let store = trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .expect("connect");
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("tomb", "batch-empty-put", 1, "live"),
            WriteContext::baseline(),
        )
        .await
        .expect("create live");

    let client = async_nats::connect(&nats.url).await.expect("nats connect");
    let js = jetstream::new(client);
    let kv = js
        .get_key_value(&config.bucket)
        .await
        .expect("bucket must exist");
    let key = trogon_atlas_store::key::entity_key(kind, &id("tomb", "batch-empty-put", 1));
    let entry = kv
        .entry(key.clone())
        .await
        .expect("kv.entry")
        .expect("key must exist");
    kv.update(key, bytes::Bytes::new(), entry.revision)
        .await
        .expect("plant empty-Put tombstone");

    let outcomes = store
        .batch_apply(
            &[MutationOp::Create {
                kind,
                entity: event("tomb", "batch-empty-put", 1, "reborn"),
            }],
            WriteContext::baseline(),
        )
        .await
        .expect("batch create must succeed over empty-Put tombstone");
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], MutationOutcome::Wrote { .. }));

    let got = store
        .get(kind, &id("tomb", "batch-empty-put", 1), None)
        .await
        .expect("reborn entity must be readable");
    match got.entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "reborn"),
        other => panic!("unexpected entity: {other:?}"),
    }
}

/// An empty-Put tombstone (delete's conditional sentinel when `kv.delete`
/// never lands) must look absent to `get`/`create`. `create` currently fails
/// with `AlreadyExists` because NATS still sees a Put row.
#[tokio::test(flavor = "multi_thread")]
async fn create_succeeds_over_empty_put_tombstone() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("test-{suffix}");
    config.branches_bucket = format!("test-branches-{suffix}");
    config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    config.lease_bucket = format!("test-writer-lease-{suffix}");

    let store = trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .expect("connect");
    let kind = pb::EntityKind::Event;
    let entity = event("tomb", "empty-put", 1, "live");
    store
        .create(kind, &entity, WriteContext::baseline())
        .await
        .expect("create live");

    let client = async_nats::connect(&nats.url).await.expect("nats connect");
    let js = jetstream::new(client);
    let kv = js
        .get_key_value(&config.bucket)
        .await
        .expect("bucket must exist");
    let key = trogon_atlas_store::key::entity_key(kind, &id("tomb", "empty-put", 1));
    let entry = kv
        .entry(key.clone())
        .await
        .expect("kv.entry")
        .expect("key must exist");
    kv.update(key, bytes::Bytes::new(), entry.revision)
        .await
        .expect("plant empty-Put tombstone");

    assert!(
        matches!(
            store.get(kind, &id("tomb", "empty-put", 1), None).await,
            Err(StoreError::NotFound)
        ),
        "empty-Put must be NotFound from get"
    );

    store
        .create(
            kind,
            &event("tomb", "empty-put", 1, "reborn"),
            WriteContext::baseline(),
        )
        .await
        .expect("create must succeed over an empty-Put tombstone (merged view is absent)");

    let got = store
        .get(kind, &id("tomb", "empty-put", 1), None)
        .await
        .expect("reborn entity must be readable");
    match got.entity.kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "reborn"),
        other => panic!("unexpected entity: {other:?}"),
    }
}

// Item 8b: two concurrent unconditional deletes racing on the same key.
// At most one must succeed; the loser must get EtagMismatch.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_unconditional_deletes_race() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;
    store
        .create(
            kind,
            &event("race", "del-race", 1, "initial"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let s1 = store.clone();
    let s2 = store.clone();
    let t1 = task::spawn(async move {
        s1.delete(
            kind,
            &id("race", "del-race", 1),
            None,
            WriteContext::baseline(),
        )
        .await
    });
    let t2 = task::spawn(async move {
        s2.delete(
            kind,
            &id("race", "del-race", 1),
            None,
            WriteContext::baseline(),
        )
        .await
    });

    let (r1, r2) = tokio::join!(t1, t2);
    let r1 = r1.expect("task1 panicked");
    let r2 = r2.expect("task2 panicked");

    let successes = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
    let losers = [&r1, &r2]
        .iter()
        .filter(|r| {
            matches!(
                r,
                Err(StoreError::EtagMismatch { .. } | StoreError::NotFound)
            )
        })
        .count();

    assert_eq!(
        successes, 1,
        "exactly one concurrent unconditional delete must succeed"
    );
    assert_eq!(
        losers, 1,
        "the losing delete must get EtagMismatch or, if it read after the winner landed, NotFound"
    );

    assert!(
        matches!(
            store.get(kind, &id("race", "del-race", 1), None).await,
            Err(StoreError::NotFound)
        ),
        "key must be absent after one successful delete"
    );
}

// Schema version marker tests

/// (a) Opening a fresh bucket stamps the schema version marker.
#[tokio::test(flavor = "multi_thread")]
async fn schema_marker_written_on_fresh_bucket() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("test-{suffix}");
    config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;

    // Open the store -- this should write the marker.
    trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .expect("fresh bucket open must succeed");

    // Read the marker directly from NATS to confirm it was persisted.
    let client = async_nats::connect(&nats.url).await.expect("nats connect");
    let js = jetstream::new(client);
    let kv = js
        .get_key_value(&config.bucket)
        .await
        .expect("bucket must exist");
    let entry = kv
        .entry("_schema")
        .await
        .expect("kv.entry")
        .expect("_schema key must be present");
    let written = std::str::from_utf8(&entry.value).expect("utf-8");
    assert_eq!(
        written,
        trogon_atlas_store::STORE_SCHEMA_MARKER,
        "_schema marker must equal STORE_SCHEMA_MARKER"
    );
}

/// (b) Reopening the same bucket with the matching version succeeds.
#[tokio::test(flavor = "multi_thread")]
async fn schema_marker_reopen_matching_version_succeeds() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("test-{suffix}");
    config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;

    // First open writes the marker.
    trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .expect("first open must succeed");

    // Second open must also succeed (marker already present and matching).
    trogon_atlas_store::NatsStore::connect_with(config)
        .await
        .expect("reopen with matching schema version must succeed");
}

/// (c) Opening a bucket whose marker holds a different version fails with `SchemaMismatch`.
#[tokio::test(flavor = "multi_thread")]
async fn schema_marker_mismatch_fails_construction() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let bucket = format!("test-{suffix}");

    // Create the KV bucket manually and write a wrong schema version marker.
    let client = async_nats::connect(&nats.url).await.expect("nats connect");
    let js = jetstream::new(client);
    let kv = js
        .create_key_value(KvConfig {
            bucket: bucket.clone(),
            history: 1,
            ..Default::default()
        })
        .await
        .expect("create kv bucket");
    kv.put("_schema", "eventmodel.vNOTREAL".as_bytes().to_vec().into())
        .await
        .expect("write wrong marker");

    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = bucket;
    config.stream = format!("TEST_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;

    let Err(err) = trogon_atlas_store::NatsStore::connect_with(config).await else {
        panic!("mismatched schema version must prevent opening the store");
    };

    assert!(
        matches!(err, StoreError::SchemaMismatch { .. }),
        "expected SchemaMismatch, got: {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("eventmodel.vNOTREAL"),
        "error must name the bucket version; got: {msg}"
    );
    assert!(
        msg.contains(trogon_atlas_store::STORE_SCHEMA_MARKER),
        "error must name the expected version; got: {msg}"
    );
}

/// (d) `list()` over a populated bucket never returns the schema marker.
#[tokio::test(flavor = "multi_thread")]
async fn schema_marker_excluded_from_list() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    // Write a few entities.
    store
        .create(
            pb::EntityKind::Event,
            &event("marker-list", "a", 1, "A"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .create(
            pb::EntityKind::Event,
            &event("marker-list", "b", 1, "B"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    // Unfiltered list must return exactly the two entities and not the marker.
    let results = store
        .list(ListFilter::default(), None, None)
        .await
        .expect("list must succeed");

    for stored in &results {
        let has_kind = stored.entity.kind.is_some();
        assert!(
            has_kind,
            "every list result must be a real entity with a kind set; \
             schema marker must not appear in list output"
        );
    }

    // The count of results must include exactly the entities we wrote in this
    // store (each test gets an isolated bucket, so only our two are present).
    assert_eq!(
        results.len(),
        2,
        "list must return exactly the entity count, excluding the schema marker"
    );
}

// P1 hardening: batch_apply must reject op slices exceeding BATCH_APPLY_MAX_OPS.
#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_rejects_oversized_batch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let kind = pb::EntityKind::Event;

    // Build 1001 ops (one over the cap of 1000). The entities are distinct so
    // the snapshot phase would not deduplicate them, but the cap fires before
    // any KV interaction.
    let ops: Vec<_> = (0..1001_u64)
        .map(|i| MutationOp::Create {
            kind,
            entity: event("cap-test", &format!("s{i}"), i, "t"),
        })
        .collect();

    let err = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .expect_err("oversized batch must be rejected");

    assert!(
        matches!(err, StoreError::InvalidArgument(_)),
        "oversized batch must return InvalidArgument, got: {err:?}"
    );
}

/// Undecodable change-log payloads must be skipped by `read_changes`, matching
/// the live change-tail behaviour. Hard-failing used to poison ListChanges /
/// StreamChanges backlog and gap-fill while the broadcast continued past the
/// hole, which silently dropped or aborted recovery of later durable records.
#[tokio::test(flavor = "multi_thread")]
async fn read_changes_skips_undecodable_payload_and_continues() {
    let nats = trogon_atlas_testsupport::shared().await;
    let suffix = trogon_atlas_testsupport::unique_suffix_for_test();
    let mut config = NatsStoreConfig::new(&nats.url);
    config.bucket = format!("test-badchg-{suffix}");
    config.branches_bucket = format!("test-badchg-branches-{suffix}");
    config.stream = format!("TEST_BADCHG_{}", suffix.replace('-', "_").to_uppercase());
    config.subject_root = format!("test.badchg.{suffix}");
    config.changes_max_msgs = Some(10_000);
    config.changes_max_age = None;
    config.lease_bucket = format!("test-badchg-writer-lease-{suffix}");
    let store = trogon_atlas_store::NatsStore::connect_with(config.clone())
        .await
        .expect("connect");

    let before = store.current_change_seq().await.expect("seq");

    let s1 = store
        .record_change(ChangeKind::Put, &entity_ref("shop", "good-a", 1), None)
        .await
        .expect("first good change");

    // Inject garbage into the changes stream between two valid records.
    let client = async_nats::connect(&nats.url).await.expect("nats connect");
    let js = jetstream::new(client);
    let ack = js
        .publish(
            format!("{}.put.shop.garbage", config.subject_root),
            Vec::from("not-a-change-event").into(),
        )
        .await
        .expect("publish garbage")
        .await
        .expect("garbage ack");
    let bad_seq = ack.sequence;
    assert!(
        bad_seq > s1,
        "garbage must land after the first good record"
    );

    let s2 = store
        .record_change(ChangeKind::Put, &entity_ref("shop", "good-b", 1), None)
        .await
        .expect("second good change");
    assert!(s2 > bad_seq, "second good change must follow the garbage");

    let records = store
        .read_changes(before, 10)
        .await
        .expect("read_changes must not fail on an undecodable neighbour");
    let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
    assert_eq!(
        seqs,
        vec![s1, s2],
        "undecodable payload must be skipped; both flanking good records must be returned"
    );
}
