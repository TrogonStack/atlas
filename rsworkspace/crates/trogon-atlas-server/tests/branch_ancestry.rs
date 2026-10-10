//! In-process gRPC integration tests for branch ancestry (G11): the fork
//! point a branch carries, the baseline changesets that landed behind it, and
//! when `UpdateBranch` is allowed to move the pointer.
//!
//! Each test gets its own isolated store, so `behind` can be asserted exactly
//! rather than "contains".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{metadata::MetadataValue, transport::Channel};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::service::EventModelServiceImpl;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn client() -> EventModelServiceClient<Channel> {
    let endpoint = start_server().await;
    EventModelServiceClient::connect(endpoint).await.unwrap()
}

fn event(slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "shop".into(),
                slug: slug.into(),
                version: 1,
            }),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn put_req(entity: pb::Entity) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: false,
    }
}

fn on_branch<T>(body: T, branch: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    req
}

async fn put(c: &mut EventModelServiceClient<Channel>, slug: &str, title: &str) {
    c.put_entity(put_req(event(slug, title))).await.unwrap();
}

async fn create_branch(c: &mut EventModelServiceClient<Channel>, name: &str) -> pb::BranchInfo {
    c.create_branch(pb::CreateBranchRequest {
        name: name.into(),
        doc: String::new(),
    })
    .await
    .unwrap()
    .into_inner()
    .branch
    .unwrap()
}

async fn diff(c: &mut EventModelServiceClient<Channel>, name: &str) -> pb::DiffBranchResponse {
    c.diff_branch(pb::DiffBranchRequest { name: name.into() })
        .await
        .unwrap()
        .into_inner()
}

/// A branch cut after a baseline write records that write as its fork point,
/// and reports nothing behind it until baseline moves again.
#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_branch_is_up_to_date_with_the_changeset_it_forked_from() {
    let mut c = client().await;
    put(&mut c, "order-placed", "Order Placed").await;

    let info = create_branch(&mut c, "fresh").await;
    assert!(
        !info.fork_changeset_id.is_empty(),
        "a branch cut after an attributed write must record a fork point"
    );

    let ancestry = diff(&mut c, "fresh").await.ancestry.unwrap();
    assert_eq!(ancestry.fork_changeset_id, info.fork_changeset_id);
    assert!(
        ancestry.behind.is_empty(),
        "nothing landed on baseline since the fork"
    );
    assert!(!ancestry.behind_truncated);
}

/// The ancestry question the per-key bases could not answer: is this branch
/// behind baseline, and by how much, in one call.
#[tokio::test(flavor = "multi_thread")]
async fn diff_branch_reports_the_baseline_changesets_that_landed_after_the_fork() {
    let mut c = client().await;
    put(&mut c, "order-placed", "Order Placed").await;
    let info = create_branch(&mut c, "behind").await;

    put(&mut c, "order-shipped", "Order Shipped").await;
    put(&mut c, "order-paid", "Order Paid").await;
    // A write on the branch itself is not baseline history and must not
    // count against the branch as being behind.
    c.put_entity(on_branch(
        put_req(event("order-placed", "Order Placed (branch)")),
        "behind",
    ))
    .await
    .unwrap();

    let ancestry = diff(&mut c, "behind").await.ancestry.unwrap();
    assert_eq!(ancestry.fork_changeset_id, info.fork_changeset_id);
    assert_eq!(
        ancestry.behind.len(),
        2,
        "two baseline writes landed after the fork; the branch write is not one of them"
    );
    assert!(!ancestry.behind_truncated);
    assert!(
        ancestry.behind.iter().all(|cs| cs.branch.is_empty()),
        "only baseline changesets belong in `behind`"
    );
    // Newest first, and every one of them strictly after the fork point.
    assert!(ancestry.behind[0].id > ancestry.behind[1].id);
    assert!(ancestry.behind[1].id > ancestry.fork_changeset_id);
    let titles: Vec<&str> = ancestry
        .behind
        .iter()
        .flat_map(|cs| cs.ops.iter())
        .filter_map(|op| op.entity.as_ref()?.id.as_ref())
        .map(|id| id.slug.as_str())
        .collect();
    assert_eq!(titles, vec!["order-paid", "order-shipped"]);
}

/// A branch cut from a store with no attributed history reports no ancestry
/// rather than claiming the entire log landed after it.
#[tokio::test(flavor = "multi_thread")]
async fn a_branch_with_no_fork_point_reports_empty_ancestry() {
    let mut c = client().await;
    let info = create_branch(&mut c, "pristine").await;
    assert_eq!(info.fork_changeset_id, "");

    put(&mut c, "order-placed", "Order Placed").await;

    let ancestry = diff(&mut c, "pristine").await.ancestry.unwrap();
    assert_eq!(ancestry.fork_changeset_id, "");
    assert!(
        ancestry.behind.is_empty(),
        "with no fork point there is no position to be behind"
    );
}

/// Catching a branch up moves its fork point forward, which is what turns
/// `UpdateBranch` from a per-key rebase into a statement about the branch.
#[tokio::test(flavor = "multi_thread")]
async fn update_branch_advances_the_fork_point_when_the_branch_caught_up() {
    let mut c = client().await;
    put(&mut c, "order-placed", "Order Placed").await;
    let info = create_branch(&mut c, "catching-up").await;

    // Branch edits one key; baseline moves on a different one, so there is
    // no conflict.
    c.put_entity(on_branch(
        put_req(event("order-placed", "Order Placed (branch)")),
        "catching-up",
    ))
    .await
    .unwrap();
    put(&mut c, "order-shipped", "Order Shipped").await;

    let before = diff(&mut c, "catching-up").await.ancestry.unwrap();
    assert_eq!(before.behind.len(), 1);

    let updated = c
        .update_branch(pb::UpdateBranchRequest {
            name: "catching-up".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        updated.conflicts,
        [] as [trogon_atlas_proto::BranchDiffEntry; 0]
    );
    assert!(
        updated.fork_changeset_id > info.fork_changeset_id,
        "the fork point must move forward once the branch is caught up"
    );

    let after = diff(&mut c, "catching-up").await.ancestry.unwrap();
    assert_eq!(after.fork_changeset_id, updated.fork_changeset_id);
    assert!(
        after.behind.is_empty(),
        "a caught-up branch is behind by nothing"
    );
}

/// A branch kept across a merge has had its deltas purged, so there is
/// nothing left to reconcile. Reporting it as behind by its own merge is the
/// one answer that is certainly wrong.
#[tokio::test(flavor = "multi_thread")]
async fn merging_with_keep_branch_catches_the_branch_up_to_its_own_merge() {
    let mut c = client().await;
    put(&mut c, "order-placed", "Order Placed").await;
    let info = create_branch(&mut c, "kept").await;

    c.put_entity(on_branch(
        put_req(event("order-shipped", "Order Shipped (branch)")),
        "kept",
    ))
    .await
    .unwrap();

    let merged = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "kept".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        merged.status,
        pb::merge_branch_response::Status::Applied as i32
    );

    let ancestry = diff(&mut c, "kept").await.ancestry.unwrap();
    assert!(
        ancestry.fork_changeset_id > info.fork_changeset_id,
        "the merge must move the kept branch's fork point forward"
    );
    assert!(
        ancestry.behind.is_empty(),
        "a branch that just merged is not behind its own merge"
    );
}

/// With a conflict still open the branch has NOT reconciled every baseline
/// write, so moving the pointer past them would erase the evidence.
#[tokio::test(flavor = "multi_thread")]
async fn update_branch_leaves_the_fork_point_alone_while_a_conflict_is_open() {
    let mut c = client().await;
    put(&mut c, "order-placed", "Order Placed").await;
    let info = create_branch(&mut c, "conflicted").await;

    // Both sides edit the same key: an edit/edit conflict.
    c.put_entity(on_branch(
        put_req(event("order-placed", "Order Placed (branch)")),
        "conflicted",
    ))
    .await
    .unwrap();
    put(&mut c, "order-placed", "Order Placed (baseline)").await;

    let updated = c
        .update_branch(pb::UpdateBranchRequest {
            name: "conflicted".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(
        !updated.conflicts.is_empty(),
        "the same key edited on both sides must conflict"
    );
    assert_eq!(
        updated.fork_changeset_id, info.fork_changeset_id,
        "an unresolved conflict must pin the fork point where it was"
    );

    let after = diff(&mut c, "conflicted").await.ancestry.unwrap();
    assert_eq!(after.fork_changeset_id, info.fork_changeset_id);
    assert!(
        !after.behind.is_empty(),
        "the unreconciled baseline write must still show as behind"
    );
}
