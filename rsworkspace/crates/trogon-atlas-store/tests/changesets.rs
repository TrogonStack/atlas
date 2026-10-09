//! Durable changeset log: append, fetch, page, and the attribution that
//! rides along on the change feed.
//!
//! The change feed is a transport with retention; the changeset bucket is
//! the record. These tests pin the properties that split relies on: ids are
//! ordered, appends are idempotent, paging terminates, and a change event
//! carries back the changeset that produced it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{ChangesetOp, ChangesetPage, ChangesetRecord},
    ChangeKind, ChangesetRef, ChangesetScope, Store, StoreError, WriteContext,
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

fn entity_ref(ns: &str, slug: &str, version: u64) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version,
        }),
    }
}

fn record(id: &str, author: &str, branch: Option<&str>, slug: &str) -> ChangesetRecord {
    ChangesetRecord {
        id: id.into(),
        author: author.into(),
        at: "2024-01-01T00:00:00.000Z".into(),
        message: format!("put event {slug}"),
        rpc: "PutEntity".into(),
        branch: branch.map(str::to_owned),
        ops: vec![ChangesetOp {
            kind: ChangeKind::Put,
            entity_ref: entity_ref("shop", slug, 1),
        }],
        operation_id: None,
    }
}

/// UUIDv7 ids sort lexicographically in mint order, which is what the
/// `before` cursor relies on. Generate a batch and assert it holds.
fn minted_ids(n: usize) -> Vec<String> {
    let ids: Vec<String> = (0..n).map(|_| uuid::Uuid::now_v7().to_string()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(
        ids, sorted,
        "UUIDv7 must be monotonic; the changeset cursor depends on it"
    );
    ids
}

#[tokio::test(flavor = "multi_thread")]
async fn append_then_get_roundtrips_every_field() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let id = uuid::Uuid::now_v7().to_string();
    let mut rec = record(&id, "alice", Some("feature-x"), "order-placed");
    rec.ops.push(ChangesetOp {
        kind: ChangeKind::Delete,
        entity_ref: entity_ref("shop", "order-cancelled", 2),
    });

    store.append_changeset(&rec).await.unwrap();
    let got = store.get_changeset(&id).await.unwrap();

    assert_eq!(got.id, rec.id);
    assert_eq!(got.author, "alice");
    assert_eq!(got.at, rec.at);
    assert_eq!(got.message, rec.message);
    assert_eq!(got.rpc, "PutEntity");
    assert_eq!(got.branch.as_deref(), Some("feature-x"));
    assert_eq!(got.ops.len(), 2);
    assert!(matches!(got.ops[0].kind, ChangeKind::Put));
    assert!(matches!(got.ops[1].kind, ChangeKind::Delete));
    assert_eq!(
        got.ops[1].entity_ref,
        entity_ref("shop", "order-cancelled", 2)
    );
}

/// A baseline changeset stores no branch. Round-tripping must not turn the
/// absent branch into an empty-string branch that later filters wrongly.
#[tokio::test(flavor = "multi_thread")]
async fn baseline_changeset_has_no_branch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let id = uuid::Uuid::now_v7().to_string();
    store
        .append_changeset(&record(&id, "bob", None, "baseline-write"))
        .await
        .unwrap();

    let got = store.get_changeset(&id).await.unwrap();
    assert_eq!(got.branch, None);
}

/// Appending the same id twice must not duplicate history: a mutation RPC
/// that is retried after a partial failure re-appends the same record.
#[tokio::test(flavor = "multi_thread")]
async fn append_is_idempotent_on_id() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let id = uuid::Uuid::now_v7().to_string();
    let rec = record(&id, "carol", None, "retried");

    store.append_changeset(&rec).await.unwrap();
    store
        .append_changeset(&rec)
        .await
        .expect("re-appending the same changeset id must succeed, not conflict");

    let page = store
        .list_changesets(ChangesetPage {
            before: None,
            limit: 50,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    assert_eq!(
        page.iter().filter(|c| c.id == id).count(),
        1,
        "a retried append must not produce a second row"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_missing_changeset_is_not_found() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let missing = uuid::Uuid::now_v7().to_string();
    assert!(matches!(
        store.get_changeset(&missing).await,
        Err(StoreError::NotFound)
    ));
}

/// An id that is not a UUID must be rejected before it reaches NATS: a key
/// containing a subject wildcard would otherwise be constructible by a
/// caller.
#[tokio::test(flavor = "multi_thread")]
async fn malformed_ids_are_rejected_before_the_backend() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    for bad in [
        "",
        "not-a-uuid",
        "*",
        ">",
        "atlas.changes.>",
        "0190A0F1-0000-7000-8000-000000000000",
    ] {
        assert!(
            matches!(
                store.get_changeset(bad).await,
                Err(StoreError::InvalidArgument(_))
            ),
            "id {bad:?} must be rejected as invalid"
        );
        let mut rec = record("placeholder", "dave", None, "x");
        rec.id = bad.into();
        assert!(
            matches!(
                store.append_changeset(&rec).await,
                Err(StoreError::InvalidArgument(_))
            ),
            "appending id {bad:?} must be rejected as invalid"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_returns_newest_first_and_pages_backwards() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let ids = minted_ids(7);
    for (i, id) in ids.iter().enumerate() {
        store
            .append_changeset(&record(id, "erin", None, &format!("evt-{i}")))
            .await
            .unwrap();
    }

    let first = store
        .list_changesets(ChangesetPage {
            before: None,
            limit: 3,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    let first_ids: Vec<&str> = first.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(first_ids, vec![&ids[6][..], &ids[5][..], &ids[4][..]]);

    let second = store
        .list_changesets(ChangesetPage {
            before: Some(&ids[4]),
            limit: 3,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    let second_ids: Vec<&str> = second.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(second_ids, vec![&ids[3][..], &ids[2][..], &ids[1][..]]);

    let third = store
        .list_changesets(ChangesetPage {
            before: Some(&ids[1]),
            limit: 3,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    assert_eq!(third.len(), 1, "short page signals the end of the log");
    assert_eq!(third[0].id, ids[0]);

    let past_end = store
        .list_changesets(ChangesetPage {
            before: Some(&ids[0]),
            limit: 3,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    assert!(past_end.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn list_filters_by_branch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let ids = minted_ids(4);
    store
        .append_changeset(&record(&ids[0], "erin", None, "base-a"))
        .await
        .unwrap();
    store
        .append_changeset(&record(&ids[1], "erin", Some("feat"), "feat-a"))
        .await
        .unwrap();
    store
        .append_changeset(&record(&ids[2], "erin", Some("other"), "other-a"))
        .await
        .unwrap();
    store
        .append_changeset(&record(&ids[3], "erin", Some("feat"), "feat-b"))
        .await
        .unwrap();

    let feat = store
        .list_changesets(ChangesetPage {
            before: None,
            limit: 50,
            scope: ChangesetScope::Branch("feat"),
        })
        .await
        .unwrap();
    let feat_ids: Vec<&str> = feat.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(feat_ids, vec![&ids[3][..], &ids[1][..]]);

    let all = store
        .list_changesets(ChangesetPage {
            before: None,
            limit: 50,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    assert_eq!(all.len(), 4, "no branch filter means every changeset");
}

/// `limit: 0` must not be read as "unbounded".
#[tokio::test(flavor = "multi_thread")]
async fn zero_limit_returns_at_most_one_row_and_terminates() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    for id in minted_ids(3) {
        store
            .append_changeset(&record(&id, "erin", None, "z"))
            .await
            .unwrap();
    }
    let page = store
        .list_changesets(ChangesetPage {
            before: None,
            limit: 0,
            scope: ChangesetScope::All,
        })
        .await
        .unwrap();
    assert!(page.len() <= 1, "limit 0 must not stream the whole log");
}

/// The attribution the server passes into a write must reach the change
/// feed, so a feed consumer can attribute without a second lookup.
#[tokio::test(flavor = "multi_thread")]
async fn change_events_carry_the_changeset_that_produced_them() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let start = store.current_change_seq().await.unwrap();
    let id = uuid::Uuid::now_v7().to_string();

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "attributed", 1, "Attributed"),
            WriteContext::baseline().attributed(ChangesetRef {
                id: &id,
                author: "frank",
                rpc: "PutEntity",
                operation_id: None,
            }),
        )
        .await
        .unwrap();

    let changes = store.read_changes(start, 10).await.unwrap();
    let last = changes.last().expect("the create must emit a change event");
    assert_eq!(last.changeset_id, id);
    assert_eq!(last.author, "frank");
}

/// A write made outside any mutation RPC (an import, a direct store user)
/// has no principal to attribute, and the empty strings must survive the
/// round trip rather than becoming a fake author.
#[tokio::test(flavor = "multi_thread")]
async fn unattributed_writes_leave_the_attribution_empty() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let start = store.current_change_seq().await.unwrap();

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "unattributed", 1, "Unattributed"),
            WriteContext::baseline(),
        )
        .await
        .unwrap();

    let changes = store.read_changes(start, 10).await.unwrap();
    let last = changes.last().expect("the create must emit a change event");
    assert_eq!(last.changeset_id, "");
    assert_eq!(last.author, "");
}

/// Branch writes never reach the change feed, but the changeset log is the
/// one place "who changed what on this branch" is answerable.
#[tokio::test(flavor = "multi_thread")]
async fn branch_writes_stay_off_the_change_feed_but_are_still_recordable() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store.create_branch("attrib", "doc").await.unwrap();
    let start = store.current_change_seq().await.unwrap();
    let id = uuid::Uuid::now_v7().to_string();

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "on-branch", 1, "On branch"),
            WriteContext::branch("attrib").attributed(ChangesetRef {
                id: &id,
                author: "grace",
                rpc: "PutEntity",
                operation_id: None,
            }),
        )
        .await
        .unwrap();
    store
        .append_changeset(&record(&id, "grace", Some("attrib"), "on-branch"))
        .await
        .unwrap();

    assert_eq!(
        store.current_change_seq().await.unwrap(),
        start,
        "a branch write must not emit a change event"
    );
    let got = store.get_changeset(&id).await.unwrap();
    assert_eq!(got.branch.as_deref(), Some("attrib"));
    assert_eq!(got.author, "grace");
}
