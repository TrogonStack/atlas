//! Conformance test suite for `NatsStore`.
//!
//! Each case uses a unique bucket/stream via `trogon_atlas_testsupport::shared()`
//! so tests are fully isolated even when run in parallel.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    error::StoreError,
    store::{ListFilter, MutationOp, Store},
    WriteContext,
};

fn persona(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Persona(pb::Persona {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version: 1,
            }),
            title: slug.into(),
            ..Default::default()
        })),
    }
}

fn persona_id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_get_roundtrip_stamps_system() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let etag = store
        .create(
            pb::EntityKind::Persona,
            &persona("ns", "alice"),
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;
    assert_ne!(etag, "");

    let got = store
        .get(pb::EntityKind::Persona, &persona_id("ns", "alice"), None)
        .await
        .unwrap();
    assert_eq!(got.etag, etag);
    assert!(
        got.entity.system.is_some(),
        "system stamp must be set on create"
    );
    let uid = got.entity.system.as_ref().unwrap().uid.clone();
    assert!(!uid.is_empty(), "uid must be non-empty");
}

#[tokio::test(flavor = "multi_thread")]
async fn put_overwrite_preserves_uid() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let etag1 = store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "bob"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;
    let got1 = store
        .get(pb::EntityKind::Persona, &persona_id("ns", "bob"), None)
        .await
        .unwrap();
    let uid = got1.entity.system.as_ref().unwrap().uid.clone();

    let etag2 = store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "bob"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;
    assert_ne!(etag1, etag2, "each put must produce a fresh etag");

    let got2 = store
        .get(pb::EntityKind::Persona, &persona_id("ns", "bob"), None)
        .await
        .unwrap();
    assert_eq!(
        got2.entity.system.as_ref().unwrap().uid,
        uid,
        "uid must survive an overwrite put"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn update_cas_success_and_mismatch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let etag1 = store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "carol"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;

    let etag2 = store
        .update(
            pb::EntityKind::Persona,
            &persona("ns", "carol"),
            &etag1,
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap()
        .etag;
    assert_ne!(etag1, etag2);

    let err = store
        .update(
            pb::EntityKind::Persona,
            &persona("ns", "carol"),
            &etag1,
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::EtagMismatch { .. }),
        "stale etag must yield EtagMismatch, got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_wrong_etag() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "dave"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let err = store
        .delete(
            pb::EntityKind::Persona,
            &persona_id("ns", "dave"),
            Some("wrong-etag"),
            WriteContext::baseline(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::EtagMismatch { .. }),
        "wrong etag on delete must yield EtagMismatch, got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_then_get_not_found() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "eve"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .delete(
            pb::EntityKind::Persona,
            &persona_id("ns", "eve"),
            None,
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let err = store
        .get(pb::EntityKind::Persona, &persona_id("ns", "eve"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound),
        "deleted entity must be NotFound, got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_already_deleted_returns_not_found() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "frank"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .delete(
            pb::EntityKind::Persona,
            &persona_id("ns", "frank"),
            None,
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let err = store
        .delete(
            pb::EntityKind::Persona,
            &persona_id("ns", "frank"),
            None,
            WriteContext::baseline(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound),
        "deleting a tombstoned key must return NotFound, got: {err:?}"
    );
}

/// Verifies that the limit applies to post-filter rows, not pre-filter rows.
#[tokio::test(flavor = "multi_thread")]
async fn list_kind_namespace_filters_with_limit() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    for slug in &["a", "b", "c"] {
        store
            .put(
                pb::EntityKind::Persona,
                &persona("shop", slug),
                true,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }
    for slug in &["x", "y"] {
        store
            .put(
                pb::EntityKind::Persona,
                &persona("other", slug),
                true,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    let kinds = vec![pb::EntityKind::Persona];
    let namespaces = vec!["shop".to_string()];
    let filter = ListFilter {
        kinds: &kinds,
        namespaces: &namespaces,
        latest_versions_only: false,
    };

    let result = store.list(filter, Some(2), None).await.unwrap();
    assert_eq!(
        result.len(),
        2,
        "limit must apply to filtered rows; expected 2 shop entities"
    );
    for item in &result {
        let id = trogon_atlas_store::refs::entity_id(&item.entity).unwrap();
        assert_eq!(id.namespace, "shop", "only shop namespace expected");
    }
}

/// `list(..., Some(n))` must return the first `n` entities of the sorted
/// full result. Truncating on `buffer_unordered` completion order before
/// sorting (and before `latest_versions_only`) drops later keys and can
/// omit whole slug groups.
#[tokio::test(flavor = "multi_thread")]
async fn list_limit_with_latest_versions_considers_all_versions_before_cap() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let ns = "limit-latest";

    // Lex key order: aaa@1, aaa@2, zzz@1. With concurrency fetching and an
    // early break at `limit` live rows, the first two completions are
    // typically aaa@1 and aaa@2; `latest_versions_only` then collapses to
    // just aaa@2 and silently drops zzz.
    store
        .put(
            pb::EntityKind::Persona,
            &persona(ns, "aaa"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    // Bump aaa to version 2 via a distinct Id.version row.
    let mut aaa_v2 = persona(ns, "aaa");
    if let Some(pb::entity::Kind::Persona(ref mut p)) = aaa_v2.kind {
        if let Some(ref mut id) = p.id {
            id.version = 2;
        }
    }
    store
        .put(
            pb::EntityKind::Persona,
            &aaa_v2,
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .put(
            pb::EntityKind::Persona,
            &persona(ns, "zzz"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let kinds = vec![pb::EntityKind::Persona];
    let namespaces = vec![ns.to_string()];
    let filter = ListFilter {
        kinds: &kinds,
        namespaces: &namespaces,
        latest_versions_only: true,
    };

    let limited = store.list(filter, Some(2), None).await.unwrap();
    let slugs: Vec<String> = limited
        .iter()
        .map(|s| {
            trogon_atlas_store::refs::entity_id(&s.entity)
                .unwrap()
                .slug
                .clone()
        })
        .collect();
    assert_eq!(
        limited.len(),
        2,
        "expected both latest slug groups (aaa@2 and zzz@1); got slugs {slugs:?}. \
         Early truncate-before-latest drops zzz when only aaa versions fill the limit"
    );
    assert!(
        slugs.iter().any(|s| s == "aaa"),
        "aaa latest must be present, got {slugs:?}"
    );
    assert!(
        slugs.iter().any(|s| s == "zzz"),
        "zzz must be present, got {slugs:?}"
    );
    let aaa = limited
        .iter()
        .find(|s| trogon_atlas_store::refs::entity_id(&s.entity).unwrap().slug == "aaa")
        .unwrap();
    assert_eq!(
        trogon_atlas_store::refs::entity_id(&aaa.entity)
            .unwrap()
            .version,
        2,
        "aaa must be the latest version, not a stale row kept by early truncate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_get() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "p1"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();
    store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "p2"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let keys = vec![
        (pb::EntityKind::Persona, persona_id("ns", "p1")),
        (pb::EntityKind::Persona, persona_id("ns", "missing")),
        (pb::EntityKind::Persona, persona_id("ns", "p2")),
    ];
    let results = store.batch_get(&keys, None).await.unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].is_some(), "p1 must be found");
    assert!(results[1].is_none(), "missing must be absent");
    assert!(results[2].is_some(), "p2 must be found");
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_success() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let ops = vec![
        MutationOp::Create {
            kind: pb::EntityKind::Persona,
            entity: persona("ns", "ba1"),
        },
        MutationOp::Put {
            kind: pb::EntityKind::Persona,
            entity: persona("ns", "ba2"),
            if_match: None,
            force: true,
        },
    ];
    let outcomes = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .unwrap();
    assert_eq!(outcomes.len(), 2);

    store
        .get(pb::EntityKind::Persona, &persona_id("ns", "ba1"), None)
        .await
        .unwrap();
    store
        .get(pb::EntityKind::Persona, &persona_id("ns", "ba2"), None)
        .await
        .unwrap();
}

/// A mid-batch failure must roll back all prior writes in the same batch.
#[tokio::test(flavor = "multi_thread")]
async fn batch_apply_mid_batch_atomicity() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .put(
            pb::EntityKind::Persona,
            &persona("ns", "existing"),
            true,
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let ops = vec![
        MutationOp::Put {
            kind: pb::EntityKind::Persona,
            entity: persona("ns", "new-from-batch"),
            if_match: None,
            force: true,
        },
        MutationOp::Create {
            kind: pb::EntityKind::Persona,
            entity: persona("ns", "existing"),
        },
    ];
    let err = store
        .batch_apply(&ops, WriteContext::baseline())
        .await
        .unwrap_err();
    let (index, source) = match err {
        StoreError::BatchFailed { index, source } => (index, *source),
        other => panic!("expected BatchFailed, got: {other:?}"),
    };
    assert_eq!(index, 1, "failure must report op index 1");
    assert!(
        matches!(source, StoreError::AlreadyExists),
        "expected AlreadyExists, got: {source:?}"
    );

    let not_found = store
        .get(
            pb::EntityKind::Persona,
            &persona_id("ns", "new-from-batch"),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(not_found, StoreError::NotFound),
        "op 0 must be rolled back; expected NotFound, got: {not_found:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn read_changes_ordering() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    for slug in &["r1", "r2", "r3"] {
        store
            .put(
                pb::EntityKind::Persona,
                &persona("ns", slug),
                true,
                WriteContext::baseline(),
            )
            .await
            .unwrap();
    }

    let changes = store.read_changes(0, 100).await.unwrap();
    assert!(
        changes.len() >= 3,
        "expected at least 3 change records, got {}",
        changes.len()
    );
    for w in changes.windows(2) {
        assert!(
            w[0].seq < w[1].seq,
            "changes must be ordered by seq ascending: {} >= {}",
            w[0].seq,
            w[1].seq
        );
    }

    let since = changes[0].seq;
    let after = store.read_changes(since, 100).await.unwrap();
    assert!(
        after.iter().all(|c| c.seq > since),
        "read_changes(since) must exclude seq <= since"
    );
}

/// `since = u64::MAX` cannot advance the change cursor (`since + 1` overflows).
/// The store contract maps that to `InvalidArgument`, not a saturating start.
#[tokio::test(flavor = "multi_thread")]
async fn read_changes_rejects_since_overflow() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let err = store
        .read_changes(u64::MAX, 1)
        .await
        .expect_err("since=u64::MAX must be rejected");
    assert!(
        matches!(err, StoreError::InvalidArgument(_)),
        "since overflow must be InvalidArgument, got: {err:?}"
    );
}
