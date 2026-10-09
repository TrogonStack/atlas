//! Store-level coverage for the branch fork point (G11).
//!
//! A branch's per-key `base`/`base_etag` answers "did THIS key move under
//! me". The fork point answers the question the per-key bases cannot: where
//! the branch as a whole sits in baseline history. These tests pin that it is
//! captured at creation, persists across a reread, moves only when told to,
//! and never silently invents a position.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{ChangesetOp, ChangesetRecord},
    ChangeKind, Store, StoreError,
};

fn record(id: &str, branch: Option<&str>) -> ChangesetRecord {
    ChangesetRecord {
        id: id.into(),
        author: "alice".into(),
        at: "2024-01-01T00:00:00.000Z".into(),
        message: "put event order-placed".into(),
        rpc: "PutEntity".into(),
        branch: branch.map(str::to_owned),
        ops: vec![ChangesetOp {
            kind: ChangeKind::Put,
            entity_ref: pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: "shop".into(),
                    slug: "order-placed".into(),
                    version: 1,
                }),
            },
        }],
        operation_id: None,
    }
}

async fn fork_point_of(store: &impl Store, branch: &str) -> Option<String> {
    store
        .list_branches()
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.name == branch)
        .expect("branch must be registered")
        .fork_changeset_id
}

/// The whole point: a branch created after some baseline history knows which
/// baseline changeset it forked from.
#[tokio::test(flavor = "multi_thread")]
async fn create_branch_captures_the_newest_baseline_changeset() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let older = uuid::Uuid::now_v7().to_string();
    let newest = uuid::Uuid::now_v7().to_string();
    store.append_changeset(&record(&older, None)).await.unwrap();
    store
        .append_changeset(&record(&newest, None))
        .await
        .unwrap();

    let info = store.create_branch("forked", "").await.unwrap();
    assert_eq!(info.fork_changeset_id.as_deref(), Some(newest.as_str()));
    assert_eq!(
        fork_point_of(&store, "forked").await.as_deref(),
        Some(newest.as_str())
    );
}

/// A branch cut from a store with no attributed write history has no fork
/// point. Reporting a bogus one would make every later ancestry answer wrong.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_with_no_baseline_history_behind_it_has_no_fork_point() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let info = store.create_branch("virgin", "").await.unwrap();
    assert_eq!(info.fork_changeset_id, None);
    assert_eq!(fork_point_of(&store, "virgin").await, None);
}

/// Branch changesets are not baseline history, so they must not become a
/// fork point: a branch forked "from" another branch's edit would place it
/// nowhere in baseline history at all.
#[tokio::test(flavor = "multi_thread")]
async fn branch_changesets_never_become_a_fork_point() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let baseline = uuid::Uuid::now_v7().to_string();
    let on_branch = uuid::Uuid::now_v7().to_string();
    store
        .append_changeset(&record(&baseline, None))
        .await
        .unwrap();
    store
        .append_changeset(&record(&on_branch, Some("other")))
        .await
        .unwrap();

    let info = store.create_branch("forked", "").await.unwrap();
    assert_eq!(
        info.fork_changeset_id.as_deref(),
        Some(baseline.as_str()),
        "the newer branch changeset must be skipped in favour of the newest baseline one"
    );
}

/// `UpdateBranch` advances the pointer through this method; the new value has
/// to survive a reread, and nothing else on the metadata row may move.
#[tokio::test(flavor = "multi_thread")]
async fn advancing_the_fork_point_persists_and_leaves_the_rest_of_the_row_alone() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let first = uuid::Uuid::now_v7().to_string();
    store.append_changeset(&record(&first, None)).await.unwrap();
    let created = store
        .create_branch("moving", "why it exists")
        .await
        .unwrap();

    let later = uuid::Uuid::now_v7().to_string();
    store.append_changeset(&record(&later, None)).await.unwrap();
    store.set_branch_fork_point("moving", &later).await.unwrap();

    let after = store
        .list_branches()
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.name == "moving")
        .unwrap();
    assert_eq!(after.fork_changeset_id.as_deref(), Some(later.as_str()));
    assert_eq!(after.doc, "why it exists");
    assert_eq!(after.created_at, created.created_at);
    assert_eq!(after.base_change_token, created.base_change_token);
}

/// The fork point doubles as a KV key elsewhere in the log, so a malformed id
/// is rejected before it reaches the backend rather than being persisted and
/// blowing up on the next read.
#[tokio::test(flavor = "multi_thread")]
async fn advancing_rejects_a_malformed_id_and_an_unknown_branch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store.create_branch("real", "").await.unwrap();

    for bad in ["", "not-a-uuid", "atlas.changes.>", "*"] {
        assert!(
            matches!(
                store.set_branch_fork_point("real", bad).await,
                Err(StoreError::InvalidArgument(_))
            ),
            "fork point {bad:?} must be rejected"
        );
    }

    let well_formed = uuid::Uuid::now_v7().to_string();
    assert!(matches!(
        store.set_branch_fork_point("ghost", &well_formed).await,
        Err(StoreError::NotFound)
    ));
}

/// The default `newest_baseline_changeset` has to see past branch traffic
/// even when a whole page of it sits on top of the newest baseline record.
#[tokio::test(flavor = "multi_thread")]
async fn newest_baseline_changeset_looks_past_branch_traffic() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let baseline = uuid::Uuid::now_v7().to_string();
    store
        .append_changeset(&record(&baseline, None))
        .await
        .unwrap();
    for _ in 0..25 {
        let id = uuid::Uuid::now_v7().to_string();
        store
            .append_changeset(&record(&id, Some("noisy")))
            .await
            .unwrap();
    }

    assert_eq!(
        store.newest_baseline_changeset().await.unwrap().as_deref(),
        Some(baseline.as_str())
    );
}
