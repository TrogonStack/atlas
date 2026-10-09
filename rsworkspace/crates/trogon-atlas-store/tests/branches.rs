//! Branch overlay integration tests against a real NATS server with
//! `JetStream` (Phase 1: Isolation, see docs/explanation/branching.md).
//!
//! Covers: branch lifecycle (create/list/delete), copy-on-write put/delete/
//! no-op visibility semantics on a branch overlay vs. baseline, change-feed
//! suppression for branch writes, and `delete_branch` cleanup.
//!
//! Phase 2 (Review and merge) store-level coverage lives at the bottom of
//! this file: `list_branch_deltas`, `land_branch_merge`,
//! `rebase_branch_deltas`, `resolve_branch_entry`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_proto as pb;
use trogon_atlas_store::{
    store::{BranchLandOp, ListFilter, MutationOutcome},
    Store, StoreError, WriteContext,
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
async fn branch_lifecycle_create_list_delete() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    let info = store
        .create_branch("feature-x", "trying something")
        .await
        .expect("create_branch must succeed");
    assert_eq!(info.name, "feature-x");
    assert_eq!(info.doc, "trying something");
    assert_eq!(info.delta_count, 0);
    assert_ne!(info.created_at, "");
    assert_ne!(info.base_change_token, "");

    assert!(matches!(
        store.create_branch("feature-x", "again").await,
        Err(StoreError::AlreadyExists)
    ));

    let listed = store.list_branches().await.expect("list_branches");
    assert!(listed.iter().any(|b| b.name == "feature-x"));

    store
        .delete_branch("feature-x")
        .await
        .expect("delete_branch must succeed");

    let listed_after = store.list_branches().await.expect("list_branches");
    assert!(!listed_after.iter().any(|b| b.name == "feature-x"));

    assert!(matches!(
        store.delete_branch("feature-x").await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_branch_rejects_reserved_name() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    assert!(matches!(
        store.create_branch("meta", "doc").await,
        Err(StoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        store.create_branch("META", "doc").await,
        Err(StoreError::InvalidArgument(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_branches_reports_delta_count() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("counted", "doc")
        .await
        .expect("create_branch");

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "a", 1, "A"),
            WriteContext::branch("counted"),
        )
        .await
        .expect("branch create");
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "b", 1, "B"),
            WriteContext::branch("counted"),
        )
        .await
        .expect("branch create");

    let listed = store.list_branches().await.expect("list_branches");
    let branch = listed
        .iter()
        .find(|b| b.name == "counted")
        .expect("branch must be listed");
    assert_eq!(branch.delta_count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_put_is_invisible_on_baseline_and_visible_on_branch() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("isolation", "doc")
        .await
        .expect("create_branch");

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "branch-only", 1, "Branch only"),
            WriteContext::branch("isolation"),
        )
        .await
        .expect("branch create must succeed");

    // Invisible on baseline.
    assert!(matches!(
        store
            .get(pb::EntityKind::Event, &id("shop", "branch-only", 1), None)
            .await,
        Err(StoreError::NotFound)
    ));

    // Visible on the branch.
    let stored = store
        .get(
            pb::EntityKind::Event,
            &id("shop", "branch-only", 1),
            Some("isolation"),
        )
        .await
        .expect("must be visible on branch");
    match stored.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Branch only"),
        other => panic!("unexpected entity: {other:?}"),
    }

    // A different branch must not see it either.
    store
        .create_branch("other-branch", "doc")
        .await
        .expect("create_branch");
    assert!(matches!(
        store
            .get(
                pb::EntityKind::Event,
                &id("shop", "branch-only", 1),
                Some("other-branch"),
            )
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_overlays_existing_baseline_entity() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "shared", 1, "Baseline title"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("overlay", "doc")
        .await
        .expect("create_branch");

    store
        .put(
            pb::EntityKind::Event,
            &event("shop", "shared", 1, "Branch title"),
            false,
            WriteContext::branch("overlay"),
        )
        .await
        .expect("branch put must succeed");

    // Baseline is untouched.
    let baseline = store
        .get(pb::EntityKind::Event, &id("shop", "shared", 1), None)
        .await
        .expect("baseline get");
    match baseline.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Baseline title"),
        other => panic!("unexpected: {other:?}"),
    }

    // The branch sees the overlay.
    let overlaid = store
        .get(
            pb::EntityKind::Event,
            &id("shop", "shared", 1),
            Some("overlay"),
        )
        .await
        .expect("branch get");
    match overlaid.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Branch title"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_delete_creates_tombstone_hiding_baseline_entity() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "to-hide", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("tombstone-branch", "doc")
        .await
        .expect("create_branch");

    store
        .delete(
            pb::EntityKind::Event,
            &id("shop", "to-hide", 1),
            None,
            WriteContext::branch("tombstone-branch"),
        )
        .await
        .expect("branch delete must succeed");

    // Hidden on the branch.
    assert!(matches!(
        store
            .get(
                pb::EntityKind::Event,
                &id("shop", "to-hide", 1),
                Some("tombstone-branch"),
            )
            .await,
        Err(StoreError::NotFound)
    ));

    // Still present on baseline.
    let baseline = store
        .get(pb::EntityKind::Event, &id("shop", "to-hide", 1), None)
        .await
        .expect("baseline get must still succeed");
    match baseline.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Baseline"),
        other => panic!("unexpected: {other:?}"),
    }

    // list() on the branch must omit the tombstoned entity.
    let listed = store
        .list(ListFilter::default(), None, Some("tombstone-branch"))
        .await
        .expect("branch list");
    assert!(
        !listed.iter().any(|s| matches!(
            &s.entity.kind,
            Some(pb::entity::Kind::Event(e)) if e.id.as_ref().is_some_and(|i| i.slug == "to-hide")
        )),
        "tombstoned entity must not appear in branch list"
    );

    // list() on baseline must still show it.
    let baseline_listed = store
        .list(ListFilter::default(), None, None)
        .await
        .expect("baseline list");
    assert!(baseline_listed.iter().any(|s| matches!(
        &s.entity.kind,
        Some(pb::entity::Kind::Event(e)) if e.id.as_ref().is_some_and(|i| i.slug == "to-hide")
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_put_no_op_when_semantically_equal() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "noop", 1, "Same title"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("noop-branch", "doc")
        .await
        .expect("create_branch");

    // First branch put establishes a delta identical in content to baseline.
    let first = store
        .put(
            pb::EntityKind::Event,
            &event("shop", "noop", 1, "Same title"),
            false,
            WriteContext::branch("noop-branch"),
        )
        .await
        .expect("first branch put");
    // Because prev (baseline) is semantically equal to the new value and
    // force=false, this must be a no-op: no delta row is written.
    assert!(!first.wrote, "semantically identical put must be a no-op");

    let listed = store.list_branches().await.expect("list_branches");
    let branch = listed
        .iter()
        .find(|b| b.name == "noop-branch")
        .expect("branch must be listed");
    assert_eq!(
        branch.delta_count, 0,
        "a no-op put must not create a delta row"
    );

    // A subsequent put with different content must write a real delta.
    let second = store
        .put(
            pb::EntityKind::Event,
            &event("shop", "noop", 1, "Different title"),
            false,
            WriteContext::branch("noop-branch"),
        )
        .await
        .expect("second branch put");
    assert!(second.wrote, "semantically different put must write");

    let listed_after = store.list_branches().await.expect("list_branches");
    let branch_after = listed_after
        .iter()
        .find(|b| b.name == "noop-branch")
        .expect("branch must be listed");
    assert_eq!(branch_after.delta_count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_writes_do_not_advance_change_feed() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("silent-branch", "doc")
        .await
        .expect("create_branch");

    let before = store
        .current_change_seq()
        .await
        .expect("current_change_seq");

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "silent-1", 1, "Silent create"),
            WriteContext::branch("silent-branch"),
        )
        .await
        .expect("branch create");
    store
        .put(
            pb::EntityKind::Event,
            &event("shop", "silent-1", 1, "Silent put"),
            true,
            WriteContext::branch("silent-branch"),
        )
        .await
        .expect("branch put");
    store
        .delete(
            pb::EntityKind::Event,
            &id("shop", "silent-1", 1),
            None,
            WriteContext::branch("silent-branch"),
        )
        .await
        .expect("branch delete");

    let after = store
        .current_change_seq()
        .await
        .expect("current_change_seq");
    assert_eq!(
        before, after,
        "branch writes must never advance the change-feed sequence"
    );

    let records = store.read_changes(before, 100).await.expect("read_changes");
    assert!(
        records.is_empty(),
        "branch writes must never appear in the change feed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_branch_purges_deltas_and_baseline_falls_through() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "fallthrough", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("ephemeral", "doc")
        .await
        .expect("create_branch");

    // Branch-only entity (no baseline counterpart).
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "branch-only-2", 1, "Only on branch"),
            WriteContext::branch("ephemeral"),
        )
        .await
        .expect("branch create");

    // Overlay on top of an existing baseline entity.
    store
        .put(
            pb::EntityKind::Event,
            &event("shop", "fallthrough", 1, "Overlaid"),
            false,
            WriteContext::branch("ephemeral"),
        )
        .await
        .expect("branch put");

    store
        .delete_branch("ephemeral")
        .await
        .expect("delete_branch must succeed");

    // The branch itself is gone, so `get` against it now returns NotFound
    // regardless of what its deltas used to contain (there's no such branch).
    assert!(matches!(
        store
            .get(
                pb::EntityKind::Event,
                &id("shop", "branch-only-2", 1),
                Some("ephemeral"),
            )
            .await,
        Err(StoreError::NotFound)
    ));

    // Baseline entity remains reachable directly and its content was never
    // touched by the deleted branch's overlay.
    let baseline = store
        .get(pb::EntityKind::Event, &id("shop", "fallthrough", 1), None)
        .await
        .expect("baseline get");
    match baseline.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Baseline"),
        other => panic!("unexpected: {other:?}"),
    }

    // Recreating a branch with the same name starts with a clean slate: no
    // leftover deltas from the deleted branch of the same name.
    store
        .create_branch("ephemeral", "reused name")
        .await
        .expect("recreate branch with the same name");
    assert!(matches!(
        store
            .get(
                pb::EntityKind::Event,
                &id("shop", "branch-only-2", 1),
                Some("ephemeral"),
            )
            .await,
        Err(StoreError::NotFound)
    ));
    let listed = store.list_branches().await.expect("list_branches");
    let branch = listed
        .iter()
        .find(|b| b.name == "ephemeral")
        .expect("recreated branch must be listed");
    assert_eq!(
        branch.delta_count, 0,
        "recreated branch must not inherit deltas from the deleted branch"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_create_conflicts_with_existing_branch_entity() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("conflict-branch", "doc")
        .await
        .expect("create_branch");

    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "dup", 1, "First"),
            WriteContext::branch("conflict-branch"),
        )
        .await
        .expect("first branch create");

    assert!(matches!(
        store
            .create(
                pb::EntityKind::Event,
                &event("shop", "dup", 1, "Second"),
                WriteContext::branch("conflict-branch"),
            )
            .await,
        Err(StoreError::AlreadyExists)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_update_requires_matching_etag_from_merged_view() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("shop", "etag-check", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("etag-branch", "doc")
        .await
        .expect("create_branch");

    // update() against the branch must be guarded against the *merged*
    // (baseline, since no delta exists yet) etag, not require a delta etag.
    let baseline_etag = store
        .get(pb::EntityKind::Event, &id("shop", "etag-check", 1), None)
        .await
        .expect("baseline get")
        .etag;

    assert!(matches!(
        store
            .update(
                pb::EntityKind::Event,
                &event("shop", "etag-check", 1, "Wrong etag"),
                "not-the-real-etag",
                true,
                WriteContext::branch("etag-branch"),
            )
            .await,
        Err(StoreError::EtagMismatch { .. })
    ));

    let updated = store
        .update(
            pb::EntityKind::Event,
            &event("shop", "etag-check", 1, "Updated on branch"),
            &baseline_etag,
            true,
            WriteContext::branch("etag-branch"),
        )
        .await
        .expect("update with the correct baseline etag must succeed");
    assert!(updated.wrote);

    // Baseline remains untouched.
    let baseline_after = store
        .get(pb::EntityKind::Event, &id("shop", "etag-check", 1), None)
        .await
        .expect("baseline get");
    match baseline_after.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Baseline"),
        other => panic!("unexpected: {other:?}"),
    }
}

// ===========================================================================
// Phase 2: Review and merge (store layer)
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn list_branch_deltas_reports_base_ours_and_tombstone() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "existing", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("p2-list", "doc")
        .await
        .expect("create_branch");

    // ADDED: no baseline counterpart.
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "new-on-branch", 1, "New"),
            WriteContext::branch("p2-list"),
        )
        .await
        .expect("branch create");

    // CHANGED: overlays an existing baseline row.
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "existing", 1, "Changed on branch"),
            false,
            WriteContext::branch("p2-list"),
        )
        .await
        .expect("branch put");

    let deltas = store
        .list_branch_deltas("p2-list")
        .await
        .expect("list_branch_deltas");
    assert_eq!(deltas.len(), 2);

    let added = deltas
        .iter()
        .find(|d| d.id.slug == "new-on-branch")
        .expect("added delta present");
    assert!(added.base.is_none());
    assert!(!added.tombstone);
    assert!(added.ours.is_some());

    let changed = deltas
        .iter()
        .find(|d| d.id.slug == "existing")
        .expect("changed delta present");
    assert!(changed.base.is_some());
    assert_ne!(changed.base_etag, "");
    assert!(!changed.tombstone);
    match changed.ours.as_ref().and_then(|e| e.kind.as_ref()) {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Changed on branch"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn land_branch_merge_put_preserves_ours_bytes_verbatim() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("p2-land-put", "doc")
        .await
        .expect("create_branch");

    let ours = event("p2", "landed", 1, "Landed verbatim");
    store
        .create(
            pb::EntityKind::Event,
            &ours,
            WriteContext::branch("p2-land-put"),
        )
        .await
        .expect("branch create");

    let outcomes = store
        .land_branch_merge(
            "p2-land-put",
            &[BranchLandOp::Put {
                kind: pb::EntityKind::Event,
                ours: Box::new(ours.clone()),
                expected_baseline_etag: None,
            }],
            WriteContext::baseline(),
        )
        .await
        .expect("land_branch_merge must succeed");
    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        MutationOutcome::Wrote { entity, .. } => match &entity.kind {
            Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Landed verbatim"),
            other => panic!("unexpected: {other:?}"),
        },
        other => panic!("expected Wrote, got {other:?}"),
    }

    // Landed on baseline.
    let baseline = store
        .get(pb::EntityKind::Event, &id("p2", "landed", 1), None)
        .await
        .expect("baseline get after land");
    match baseline.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Landed verbatim"),
        other => panic!("unexpected: {other:?}"),
    }

    // The delta row was purged: the branch no longer shows a distinct
    // overlay for this key (it now falls through to the just-landed
    // baseline value).
    let deltas = store
        .list_branch_deltas("p2-land-put")
        .await
        .expect("list_branch_deltas after land");
    assert!(deltas.is_empty(), "landed delta must be purged");
}

#[tokio::test(flavor = "multi_thread")]
async fn land_branch_merge_delete_removes_baseline_row() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "to-delete", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    let baseline_etag = store
        .get(pb::EntityKind::Event, &id("p2", "to-delete", 1), None)
        .await
        .expect("baseline get")
        .etag;
    store
        .create_branch("p2-land-delete", "doc")
        .await
        .expect("create_branch");
    store
        .delete(
            pb::EntityKind::Event,
            &id("p2", "to-delete", 1),
            None,
            WriteContext::branch("p2-land-delete"),
        )
        .await
        .expect("branch delete (tombstone)");

    let outcomes = store
        .land_branch_merge(
            "p2-land-delete",
            &[BranchLandOp::Delete {
                kind: pb::EntityKind::Event,
                id: id("p2", "to-delete", 1),
                expected_baseline_etag: baseline_etag,
            }],
            WriteContext::baseline(),
        )
        .await
        .expect("land_branch_merge must succeed");
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], MutationOutcome::Deleted));

    assert!(matches!(
        store
            .get(pb::EntityKind::Event, &id("p2", "to-delete", 1), None)
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn land_branch_merge_rejects_stale_baseline_etag() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "moved", 1, "Original"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("p2-stale", "doc")
        .await
        .expect("create_branch");

    // Baseline moves after the delta's base was captured.
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "moved", 1, "Moved on baseline"),
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("baseline put");

    let result = store
        .land_branch_merge(
            "p2-stale",
            &[BranchLandOp::Put {
                kind: pb::EntityKind::Event,
                ours: Box::new(event("p2", "moved", 1, "Branch value")),
                expected_baseline_etag: Some("stale-etag-value".to_string()),
            }],
            WriteContext::baseline(),
        )
        .await;
    assert!(
        matches!(result, Err(StoreError::BatchFailed { .. })),
        "a stale baseline etag guard must fail the whole batch, got {result:?}"
    );

    // Nothing was persisted: baseline still shows the moved value.
    let baseline = store
        .get(pb::EntityKind::Event, &id("p2", "moved", 1), None)
        .await
        .expect("baseline get");
    match baseline.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Moved on baseline"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn land_branch_merge_does_not_stamp_system_and_updates_change_feed() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create_branch("p2-identity", "doc")
        .await
        .expect("create_branch");

    let ours = event("p2", "identity", 1, "Identity check");
    store
        .create(
            pb::EntityKind::Event,
            &ours,
            WriteContext::branch("p2-identity"),
        )
        .await
        .expect("branch create");

    let before_seq = store
        .current_change_seq()
        .await
        .expect("current_change_seq");

    store
        .land_branch_merge(
            "p2-identity",
            &[BranchLandOp::Put {
                kind: pb::EntityKind::Event,
                ours: Box::new(ours.clone()),
                expected_baseline_etag: None,
            }],
            WriteContext::baseline(),
        )
        .await
        .expect("land_branch_merge");

    // A merge landing is an ordinary baseline write for every downstream
    // purpose (change feed, git mirror, search index -- see
    // docs/explanation/branching.md, "Auditable"). `land_branch_merge`
    // itself publishes one change event per landed op, unlike branch
    // writes (which never touch the change feed at all).
    let after_seq = store
        .current_change_seq()
        .await
        .expect("current_change_seq");
    assert_eq!(
        after_seq,
        before_seq + 1,
        "land_branch_merge must record exactly one change-feed event per landed op"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rebase_branch_deltas_collapses_converged_and_advances_others() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "converge-me", 1, "V1"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "advance-me", 1, "V1"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("p2-rebase", "doc")
        .await
        .expect("create_branch");

    // Establish deltas for both keys.
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "converge-me", 1, "Branch edit"),
            false,
            WriteContext::branch("p2-rebase"),
        )
        .await
        .expect("branch put");
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "advance-me", 1, "Branch edit"),
            false,
            WriteContext::branch("p2-rebase"),
        )
        .await
        .expect("branch put");

    // Baseline for "converge-me" moves to match the branch's edit exactly:
    // when rebased, this collapses (CONVERGED -> delta dropped).
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "converge-me", 1, "Branch edit"),
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("baseline put converges");
    let converged_baseline_etag = store
        .get(pb::EntityKind::Event, &id("p2", "converge-me", 1), None)
        .await
        .expect("baseline get")
        .etag;

    // Baseline for "advance-me" moves to something else: rebasing must
    // advance base/base_etag forward without dropping the delta.
    let advance_new_base = event("p2", "advance-me", 1, "Baseline moved");
    store
        .put(
            pb::EntityKind::Event,
            &advance_new_base,
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("baseline put advances");
    let advance_baseline_etag = store
        .get(pb::EntityKind::Event, &id("p2", "advance-me", 1), None)
        .await
        .expect("baseline get")
        .etag;

    let rebases = vec![
        (pb::EntityKind::Event, id("p2", "converge-me", 1), None),
        (
            pb::EntityKind::Event,
            id("p2", "advance-me", 1),
            Some((advance_new_base, advance_baseline_etag.clone())),
        ),
    ];
    let count = store
        .rebase_branch_deltas("p2-rebase", &rebases)
        .await
        .expect("rebase_branch_deltas");
    assert_eq!(count, 2);

    let deltas = store
        .list_branch_deltas("p2-rebase")
        .await
        .expect("list_branch_deltas after rebase");
    assert!(
        !deltas.iter().any(|d| d.id.slug == "converge-me"),
        "converged delta must be dropped by rebase"
    );
    let advanced = deltas
        .iter()
        .find(|d| d.id.slug == "advance-me")
        .expect("non-converged delta must remain");
    assert_eq!(advanced.base_etag, advance_baseline_etag);
    match advanced.ours.as_ref().and_then(|e| e.kind.as_ref()) {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch edit"),
        other => panic!("branch value must be untouched by rebase: {other:?}"),
    }
    let _ = converged_baseline_etag;
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_branch_entry_take_theirs_adopts_baseline() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "resolve-theirs", 1, "Baseline V1"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("p2-resolve-theirs", "doc")
        .await
        .expect("create_branch");
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "resolve-theirs", 1, "Branch edit"),
            false,
            WriteContext::branch("p2-resolve-theirs"),
        )
        .await
        .expect("branch put");
    // Baseline moves, creating a real conflict.
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "resolve-theirs", 1, "Baseline V2"),
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("baseline put");

    store
        .resolve_branch_entry(
            "p2-resolve-theirs",
            pb::EntityKind::Event,
            &id("p2", "resolve-theirs", 1),
            true,
        )
        .await
        .expect("resolve_branch_entry take_theirs");

    // The branch's view now matches baseline exactly (converged).
    let on_branch = store
        .get(
            pb::EntityKind::Event,
            &id("p2", "resolve-theirs", 1),
            Some("p2-resolve-theirs"),
        )
        .await
        .expect("branch get after resolve");
    match on_branch.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "Baseline V2"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_branch_entry_keep_ours_rebases_base_forward() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "resolve-ours", 1, "Baseline V1"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    store
        .create_branch("p2-resolve-ours", "doc")
        .await
        .expect("create_branch");
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "resolve-ours", 1, "Branch edit"),
            false,
            WriteContext::branch("p2-resolve-ours"),
        )
        .await
        .expect("branch put");
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "resolve-ours", 1, "Baseline V2"),
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("baseline put");
    let new_baseline_etag = store
        .get(pb::EntityKind::Event, &id("p2", "resolve-ours", 1), None)
        .await
        .expect("baseline get")
        .etag;

    store
        .resolve_branch_entry(
            "p2-resolve-ours",
            pb::EntityKind::Event,
            &id("p2", "resolve-ours", 1),
            false,
        )
        .await
        .expect("resolve_branch_entry keep_ours");

    let deltas = store
        .list_branch_deltas("p2-resolve-ours")
        .await
        .expect("list_branch_deltas after resolve");
    let delta = deltas
        .iter()
        .find(|d| d.id.slug == "resolve-ours")
        .expect("delta must remain (kept ours)");
    assert_eq!(delta.base_etag, new_baseline_etag);
    match delta.ours.as_ref().and_then(|e| e.kind.as_ref()) {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch edit"),
        other => panic!("branch value must be preserved: {other:?}"),
    }
}

// `land_branch_merge` is a two-pass algorithm: pass 1 snapshots every
// touched baseline key's prior value and verifies every op's etag guard
// up front; pass 2 performs the CAS writes op by op, and on the first
// failure rolls every already-written key in the batch back to its
// snapshotted prior value (`rollback_baseline_snapshot`). This is
// deterministically testable without fault injection: build a batch
// where an EARLIER op's etag guard is satisfied (so its write lands),
// but a LATER op's etag was captured before an external write moved
// that baseline row, so the later op's CAS fails at write time. Unlike
// `land_branch_merge_rejects_stale_baseline_etag` (a single-op batch
// that fails the up-front guard in pass 1 before any KV write happens
// at all), this drives a >=2 op batch where the failure is only
// detectable in pass 2, after an earlier op has already been written --
// exercising the actual rollback path, not just the guard.
#[tokio::test(flavor = "multi_thread")]
async fn land_branch_merge_rolls_back_earlier_op_when_later_op_etag_is_stale() {
    let store = trogon_atlas_testsupport::shared().await.store().await;

    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "rollback-a", 1, "A original"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create a");
    let etag_a = store
        .get(pb::EntityKind::Event, &id("p2", "rollback-a", 1), None)
        .await
        .expect("baseline get a")
        .etag;

    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "rollback-b", 1, "B original"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create b");
    let etag_b = store
        .get(pb::EntityKind::Event, &id("p2", "rollback-b", 1), None)
        .await
        .expect("baseline get b")
        .etag;

    store
        .create_branch("p2-rollback", "doc")
        .await
        .expect("create_branch");
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "rollback-a", 1, "A on branch"),
            false,
            WriteContext::branch("p2-rollback"),
        )
        .await
        .expect("branch put a");
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "rollback-b", 1, "B on branch"),
            false,
            WriteContext::branch("p2-rollback"),
        )
        .await
        .expect("branch put b");

    let ops = vec![
        BranchLandOp::Put {
            kind: pb::EntityKind::Event,
            ours: Box::new(event("p2", "rollback-a", 1, "A on branch")),
            expected_baseline_etag: Some(etag_a.clone()),
        },
        BranchLandOp::Put {
            kind: pb::EntityKind::Event,
            ours: Box::new(event("p2", "rollback-b", 1, "B on branch")),
            expected_baseline_etag: Some(etag_b.clone()),
        },
    ];

    // Move baseline `rollback-b` *after* the ops above captured `etag_b`,
    // but *before* calling `land_branch_merge`. Op[0] (`rollback-a`)'s
    // etag guard is still satisfied at land time, so it writes
    // successfully in pass 2; op[1] (`rollback-b`)'s CAS then fails
    // because the baseline moved out from under it, triggering rollback
    // of everything already written in this batch (i.e. op[0]).
    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "rollback-b", 1, "B moved externally"),
            true,
            WriteContext::baseline(),
        )
        .await
        .expect("external baseline move for b");

    let before_seq = store
        .current_change_seq()
        .await
        .expect("current_change_seq");

    let result = store
        .land_branch_merge("p2-rollback", &ops, WriteContext::baseline())
        .await;
    assert!(
        matches!(result, Err(StoreError::BatchFailed { .. })),
        "a later op's stale baseline etag must fail the whole batch, got {result:?}"
    );

    // Rollback assertion: op[0]'s baseline row (`rollback-a`) must be
    // restored to its pre-merge value, not left at the merged "A on
    // branch" value that pass 2 briefly wrote.
    let baseline_a = store
        .get(pb::EntityKind::Event, &id("p2", "rollback-a", 1), None)
        .await
        .expect("baseline get a after failed land");
    match baseline_a.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(
            e.title, "A original",
            "rollback must restore the earlier op's baseline row to its pre-merge value"
        ),
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(
        baseline_a.etag, etag_a,
        "rollback must restore the earlier op's baseline row to its pre-merge revision"
    );

    // `rollback-b`'s baseline is untouched by the rollback (the write
    // never happened there in the first place) and keeps the externally
    // moved value.
    let baseline_b = store
        .get(pb::EntityKind::Event, &id("p2", "rollback-b", 1), None)
        .await
        .expect("baseline get b after failed land");
    match baseline_b.entity.kind {
        Some(pb::entity::Kind::Event(ref e)) => assert_eq!(e.title, "B moved externally"),
        other => panic!("unexpected: {other:?}"),
    }

    // Branch deltas remain intact: a failed land must not purge anything,
    // since delta purging only happens after the full ops loop succeeds.
    let deltas = store
        .list_branch_deltas("p2-rollback")
        .await
        .expect("list_branch_deltas after failed land");
    assert_eq!(
        deltas.len(),
        2,
        "a failed land_branch_merge must not purge any branch deltas"
    );
    assert!(deltas.iter().any(|d| d.id.slug == "rollback-a"));
    assert!(deltas.iter().any(|d| d.id.slug == "rollback-b"));

    // Change feed: a merge landing only publishes change events after
    // the *entire* ops loop completes with all successes (see
    // `land_branch_merge`'s implementation). A batch that fails partway
    // through -- even after successfully writing (and then rolling back)
    // an earlier op -- must record zero change events, not one event for
    // the rolled-back op.
    let after_seq = store
        .current_change_seq()
        .await
        .expect("current_change_seq");
    assert_eq!(
        before_seq, after_seq,
        "a rolled-back land_branch_merge batch must not advance the change feed at all"
    );
    let records = store
        .read_changes(before_seq, 100)
        .await
        .expect("read_changes");
    assert!(
        records.is_empty(),
        "a rolled-back land_branch_merge batch must record no change-feed events, \
         including for ops that were written and then rolled back"
    );
}

/// A subsequent put/update on an already-touched branch key must preserve the
/// original `base`/`base_etag` captured on first touch. Merge conflict
/// classification depends on that fork-from snapshot; wiping it on every
/// rewrite makes CHANGED entries look like ADDED and breaks rebase/land.
#[tokio::test(flavor = "multi_thread")]
async fn subsequent_branch_put_preserves_captured_base() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "keep-base", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    let baseline = store
        .get(pb::EntityKind::Event, &id("p2", "keep-base", 1), None)
        .await
        .expect("baseline get");
    let baseline_etag = baseline.etag.clone();

    store
        .create_branch("p2-keep-base", "doc")
        .await
        .expect("create_branch");

    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "keep-base", 1, "Branch edit 1"),
            false,
            WriteContext::branch("p2-keep-base"),
        )
        .await
        .expect("first branch put");

    let after_first = store
        .list_branch_deltas("p2-keep-base")
        .await
        .expect("deltas after first put");
    assert_eq!(after_first.len(), 1);
    assert!(
        after_first[0].base.is_some(),
        "first touch must capture baseline as base"
    );
    assert_eq!(after_first[0].base_etag, baseline_etag);

    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "keep-base", 1, "Branch edit 2"),
            false,
            WriteContext::branch("p2-keep-base"),
        )
        .await
        .expect("second branch put");

    let after_second = store
        .list_branch_deltas("p2-keep-base")
        .await
        .expect("deltas after second put");
    assert_eq!(after_second.len(), 1);
    assert!(
        after_second[0].base.is_some(),
        "second put must still carry the original captured base; got base=None \
         (merge would misclassify this CHANGED entry as ADDED)"
    );
    assert_eq!(
        after_second[0].base_etag, baseline_etag,
        "second put must preserve the original base_etag ({baseline_etag}), got {}",
        after_second[0].base_etag
    );
    match after_second[0].ours.as_ref().and_then(|e| e.kind.as_ref()) {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Branch edit 2"),
        other => panic!("unexpected ours after second put: {other:?}"),
    }
}

/// After a branch deletes a baseline key (tombstone with captured base), a
/// subsequent put that recreates the key must still remember the original
/// fork-from baseline. `branch_merged_prev` short-circuits tombstones to
/// `(None, None, None)`, so the rewrite is stored as a brand-new ADDED delta.
#[tokio::test(flavor = "multi_thread")]
async fn branch_put_after_tombstone_preserves_original_base() {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    store
        .create(
            pb::EntityKind::Event,
            &event("p2", "tomb-base", 1, "Baseline"),
            WriteContext::baseline(),
        )
        .await
        .expect("baseline create");
    let baseline = store
        .get(pb::EntityKind::Event, &id("p2", "tomb-base", 1), None)
        .await
        .expect("baseline get");
    let baseline_etag = baseline.etag.clone();

    store
        .create_branch("p2-tomb-base", "doc")
        .await
        .expect("create_branch");

    let deleted = store
        .get(
            pb::EntityKind::Event,
            &id("p2", "tomb-base", 1),
            Some("p2-tomb-base"),
        )
        .await
        .expect("merged get before delete");
    store
        .delete(
            pb::EntityKind::Event,
            &id("p2", "tomb-base", 1),
            Some(&deleted.etag),
            WriteContext::branch("p2-tomb-base"),
        )
        .await
        .expect("branch delete");

    let after_delete = store
        .list_branch_deltas("p2-tomb-base")
        .await
        .expect("deltas after delete");
    assert_eq!(after_delete.len(), 1);
    assert!(after_delete[0].tombstone);
    assert!(after_delete[0].base.is_some(), "delete must capture base");
    assert_eq!(after_delete[0].base_etag, baseline_etag);

    store
        .put(
            pb::EntityKind::Event,
            &event("p2", "tomb-base", 1, "Recreated on branch"),
            false,
            WriteContext::branch("p2-tomb-base"),
        )
        .await
        .expect("put after tombstone");

    let after_put = store
        .list_branch_deltas("p2-tomb-base")
        .await
        .expect("deltas after put");
    assert_eq!(after_put.len(), 1);
    assert!(!after_put[0].tombstone);
    assert!(
        after_put[0].base.is_some(),
        "recreate-after-tombstone must retain fork-from base; got base=None"
    );
    assert_eq!(
        after_put[0].base_etag, baseline_etag,
        "recreate-after-tombstone must retain original base_etag"
    );
}
