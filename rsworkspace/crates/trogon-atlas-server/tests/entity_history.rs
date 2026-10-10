//! In-process gRPC integration tests for per-entity history and revert
//! (G4: `git log <entity>` and undo).
//!
//! Same harness as `changesets.rs`: a real tonic server over an ephemeral
//! JetStream store, shared across the tests in this file, so every test
//! writes into its own namespace.

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

async fn client(endpoint: &str) -> EventModelServiceClient<Channel> {
    EventModelServiceClient::connect(endpoint.to_string())
        .await
        .unwrap()
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug)),
            title: title.into(),
            ..Default::default()
        })),
    }
}

fn title_of(entity: &pb::Entity) -> &str {
    match entity.kind.as_ref() {
        Some(pb::entity::Kind::Event(e)) => e.title.as_str(),
        _ => panic!("expected an Event"),
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

fn entity_ref(ns: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(id(ns, slug)),
    }
}

fn as_author<T>(body: T, name: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-author-name",
        MetadataValue::try_from(name).unwrap(),
    );
    req.metadata_mut().insert(
        "x-trogon-atlas-author-email",
        MetadataValue::try_from(format!("{name}@example.test")).unwrap(),
    );
    req
}

fn on_branch<T>(mut req: tonic::Request<T>, branch: &str) -> tonic::Request<T> {
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    req
}

async fn history(
    c: &mut EventModelServiceClient<Channel>,
    ns: &str,
    slug: &str,
) -> Vec<pb::EntityRevision> {
    c.get_entity_history(pb::GetEntityHistoryRequest {
        r#ref: Some(entity_ref(ns, slug)),
        page_token: String::new(),
        page_size: 100,
    })
    .await
    .unwrap()
    .into_inner()
    .revisions
}

async fn branch_history(
    c: &mut EventModelServiceClient<Channel>,
    branch: &str,
    ns: &str,
    slug: &str,
) -> Vec<pb::EntityRevision> {
    c.get_entity_history(on_branch(
        tonic::Request::new(pb::GetEntityHistoryRequest {
            r#ref: Some(entity_ref(ns, slug)),
            page_token: String::new(),
            page_size: 100,
        }),
        branch,
    ))
    .await
    .unwrap()
    .into_inner()
    .revisions
}

async fn stored_title(
    c: &mut EventModelServiceClient<Channel>,
    ns: &str,
    slug: &str,
) -> Option<String> {
    match c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id(ns, slug)),
        })
        .await
    {
        Ok(resp) => Some(title_of(resp.into_inner().entity.as_ref().unwrap()).to_string()),
        Err(status) if status.code() == tonic::Code::NotFound => None,
        Err(other) => panic!("unexpected get_entity error: {other}"),
    }
}

/// Two edits, two revisions, newest first, each carrying both images. The
/// pre-image of the second is the post-image of the first, but it is stored
/// on the row rather than inferred from the neighbour.
#[tokio::test(flavor = "multi_thread")]
async fn every_write_records_a_revision_with_both_images() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-edit", "order-placed", "Order placed")),
        "alice",
    ))
    .await
    .unwrap();
    c.put_entity(as_author(
        put_req(event("hist-edit", "order-placed", "Order was placed")),
        "bob",
    ))
    .await
    .unwrap();

    let revisions = history(&mut c, "hist-edit", "order-placed").await;
    assert_eq!(revisions.len(), 2, "one revision per attributed write");

    let newest = &revisions[0];
    assert_eq!(newest.author, "bob");
    assert_eq!(newest.rpc, "PutEntity");
    assert_eq!(newest.kind, pb::change_event::Kind::Put as i32);
    assert_eq!(newest.branch, "");
    assert_eq!(
        title_of(newest.before.as_ref().expect("an edit has a pre-image")),
        "Order placed"
    );
    assert_eq!(title_of(newest.after.as_ref().unwrap()), "Order was placed");

    let oldest = &revisions[1];
    assert_eq!(oldest.author, "alice");
    assert!(
        oldest.before.is_none(),
        "the write that created the key has no pre-image"
    );
    assert_eq!(title_of(oldest.after.as_ref().unwrap()), "Order placed");
    assert!(
        newest.changeset_id > oldest.changeset_id,
        "revision ids sort chronologically"
    );
}

/// The entities bucket is `history=1`, so a delete's pre-image survives only
/// because the write path captured it before overwriting.
#[tokio::test(flavor = "multi_thread")]
async fn delete_records_the_content_it_removed() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-del", "cart-emptied", "Cart emptied")),
        "alice",
    ))
    .await
    .unwrap();
    c.delete_entity(as_author(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("hist-del", "cart-emptied")),
            mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
            if_match: String::new(),
        },
        "alice",
    ))
    .await
    .unwrap();

    let revisions = history(&mut c, "hist-del", "cart-emptied").await;
    assert_eq!(revisions.len(), 2);
    let newest = &revisions[0];
    assert_eq!(newest.kind, pb::change_event::Kind::Deleted as i32);
    assert_eq!(newest.rpc, "DeleteEntity");
    assert_eq!(title_of(newest.before.as_ref().unwrap()), "Cart emptied");
    assert!(newest.after.is_none(), "a delete leaves no post-image");
}

/// Revert is a forward write: the old content comes back, the original
/// changeset stays in the log, and the undo is itself a changeset.
#[tokio::test(flavor = "multi_thread")]
async fn revert_restores_the_previous_content_as_a_new_changeset() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-revert", "paid", "Paid")),
        "alice",
    ))
    .await
    .unwrap();
    c.put_entity(as_author(
        put_req(event("hist-revert", "paid", "Payment captured")),
        "bob",
    ))
    .await
    .unwrap();

    let bad = history(&mut c, "hist-revert", "paid").await[0]
        .changeset_id
        .clone();
    let resp = c
        .revert_changeset(as_author(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: bad.clone(),
                dry_run: false,
            },
            "carol",
        ))
        .await
        .unwrap()
        .into_inner();

    assert!(!resp.dry_run);
    assert_ne!(resp.changeset_id, "");
    assert_eq!(resp.ops.len(), 1);
    assert_eq!(resp.ops[0].kind, pb::change_event::Kind::Put as i32);
    assert_eq!(
        stored_title(&mut c, "hist-revert", "paid").await.as_deref(),
        Some("Paid")
    );

    let revisions = history(&mut c, "hist-revert", "paid").await;
    assert_eq!(revisions.len(), 3, "the undo is recorded, not the erasure");
    assert_eq!(revisions[0].rpc, "RevertChangeset");
    assert_eq!(revisions[0].author, "carol");
    assert_eq!(
        title_of(revisions[0].before.as_ref().unwrap()),
        "Payment captured"
    );
    assert_eq!(title_of(revisions[0].after.as_ref().unwrap()), "Paid");
    assert!(
        c.get_changeset(pb::GetChangesetRequest { id: bad })
            .await
            .is_ok(),
        "the reverted changeset is still in the log"
    );
}

/// The inverse of "this changeset created the key" is a delete.
#[tokio::test(flavor = "multi_thread")]
async fn reverting_a_create_deletes_the_entity() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-uncreate", "typo", "Typoo")),
        "alice",
    ))
    .await
    .unwrap();
    let created = history(&mut c, "hist-uncreate", "typo").await[0]
        .changeset_id
        .clone();

    let resp = c
        .revert_changeset(as_author(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: created,
                dry_run: false,
            },
            "alice",
        ))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.ops.len(), 1);
    assert_eq!(resp.ops[0].kind, pb::change_event::Kind::Deleted as i32);
    assert_eq!(stored_title(&mut c, "hist-uncreate", "typo").await, None);
}

/// Whole-entity granularity means any later edit overlaps, which is the
/// condition git reports as a revert conflict.
#[tokio::test(flavor = "multi_thread")]
async fn revert_refuses_when_the_entity_moved_on() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    for title in ["One", "Two", "Three"] {
        c.put_entity(as_author(
            put_req(event("hist-conflict", "step", title)),
            "alice",
        ))
        .await
        .unwrap();
    }

    let middle = history(&mut c, "hist-conflict", "step").await[1]
        .changeset_id
        .clone();
    let status = c
        .revert_changeset(as_author(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: middle,
                dry_run: false,
            },
            "bob",
        ))
        .await
        .expect_err("reverting behind a newer edit must refuse");

    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(
        status.message().contains("changed after changeset"),
        "message must name the conflict: {}",
        status.message()
    );
    assert_eq!(
        stored_title(&mut c, "hist-conflict", "step")
            .await
            .as_deref(),
        Some("Three"),
        "a refused revert writes nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revert_dry_run_reports_the_inverse_without_writing_it() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-dry", "shipped", "Shipped")),
        "alice",
    ))
    .await
    .unwrap();
    let created = history(&mut c, "hist-dry", "shipped").await[0]
        .changeset_id
        .clone();

    let resp = c
        .revert_changeset(as_author(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: created,
                dry_run: true,
            },
            "alice",
        ))
        .await
        .unwrap()
        .into_inner();

    assert!(resp.dry_run);
    assert_eq!(resp.changeset_id, "");
    assert_eq!(resp.ops.len(), 1);
    assert_eq!(
        stored_title(&mut c, "hist-dry", "shipped").await.as_deref(),
        Some("Shipped"),
        "a dry run touches nothing"
    );
    assert_eq!(history(&mut c, "hist-dry", "shipped").await.len(), 1);
}

/// A `BatchMutate` is one changeset, so its inverse is one batch.
#[tokio::test(flavor = "multi_thread")]
async fn reverting_a_batch_undoes_every_op() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-batch", "kept", "Original")),
        "alice",
    ))
    .await
    .unwrap();

    c.batch_mutate(as_author(
        pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put_req(event(
                        "hist-batch",
                        "kept",
                        "Edited",
                    )))),
                },
                pb::BatchMutateOp {
                    op: Some(pb::batch_mutate_op::Op::Put(put_req(event(
                        "hist-batch",
                        "added",
                        "Added",
                    )))),
                },
            ],
            validate_only: false,
        },
        "bob",
    ))
    .await
    .unwrap();

    let batch_id = history(&mut c, "hist-batch", "added").await[0]
        .changeset_id
        .clone();
    let resp = c
        .revert_changeset(as_author(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: batch_id,
                dry_run: false,
            },
            "carol",
        ))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.ops.len(), 2);
    assert_eq!(
        stored_title(&mut c, "hist-batch", "kept").await.as_deref(),
        Some("Original")
    );
    assert_eq!(stored_title(&mut c, "hist-batch", "added").await, None);
}

/// A write that carried no changeset recorded no pre-image, and the
/// entities bucket keeps only one revision, so there is nothing to restore.
/// Refusing is the honest answer.
#[tokio::test(flavor = "multi_thread")]
async fn revert_refuses_an_unknown_changeset() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let status = c
        .revert_changeset(as_author(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: uuid::Uuid::now_v7().to_string(),
                dry_run: false,
            },
            "alice",
        ))
        .await
        .expect_err("a changeset that was never recorded cannot be reverted");
    assert_eq!(status.code(), tonic::Code::NotFound);
}

/// Branch writes never reach baseline, so neither do their revisions. The
/// branch log answers "what has this branch done to the entity".
#[tokio::test(flavor = "multi_thread")]
async fn branch_revisions_stay_off_the_baseline_log() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("hist-branch", "state", "Baseline")),
        "alice",
    ))
    .await
    .unwrap();
    c.create_branch(tonic::Request::new(pb::CreateBranchRequest {
        name: "hist-branch-work".into(),
        doc: String::new(),
    }))
    .await
    .unwrap();
    c.put_entity(on_branch(
        as_author(
            put_req(event("hist-branch", "state", "On the branch")),
            "bob",
        ),
        "hist-branch-work",
    ))
    .await
    .unwrap();

    let baseline = history(&mut c, "hist-branch", "state").await;
    assert_eq!(baseline.len(), 1, "the branch write is invisible here");
    assert_eq!(baseline[0].author, "alice");

    let branch = branch_history(&mut c, "hist-branch-work", "hist-branch", "state").await;
    assert_eq!(branch.len(), 1);
    assert_eq!(branch[0].author, "bob");
    assert_eq!(branch[0].branch, "hist-branch-work");
    assert_eq!(
        title_of(branch[0].before.as_ref().unwrap()),
        "Baseline",
        "copy-on-write records the baseline content it forked from"
    );
    assert_eq!(title_of(branch[0].after.as_ref().unwrap()), "On the branch");
}

/// `page_token` is the changeset id of the oldest revision already seen,
/// exactly as `ListChangesets` pages.
#[tokio::test(flavor = "multi_thread")]
async fn history_pages_backwards_by_changeset_id() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    for title in ["A", "B", "C", "D"] {
        c.put_entity(as_author(
            put_req(event("hist-page", "counter", title)),
            "alice",
        ))
        .await
        .unwrap();
    }

    let first = c
        .get_entity_history(pb::GetEntityHistoryRequest {
            r#ref: Some(entity_ref("hist-page", "counter")),
            page_token: String::new(),
            page_size: 2,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.revisions.len(), 2);
    assert_ne!(first.next_page_token, "");

    let second = c
        .get_entity_history(pb::GetEntityHistoryRequest {
            r#ref: Some(entity_ref("hist-page", "counter")),
            page_token: first.next_page_token.clone(),
            page_size: 2,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.revisions.len(), 2);
    assert!(
        second.revisions[0].changeset_id < first.revisions[1].changeset_id,
        "the cursor is exclusive"
    );
    assert!(
        !second.next_page_token.is_empty(),
        "a full page always yields a cursor; only a short page ends the log"
    );

    let third = c
        .get_entity_history(pb::GetEntityHistoryRequest {
            r#ref: Some(entity_ref("hist-page", "counter")),
            page_token: second.next_page_token.clone(),
            page_size: 2,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        third.revisions,
        [] as [trogon_atlas_proto::EntityRevision; 0]
    );
    assert_eq!(third.next_page_token, "");

    let titles: Vec<&str> = first
        .revisions
        .iter()
        .chain(second.revisions.iter())
        .map(|r| title_of(r.after.as_ref().unwrap()))
        .collect();
    assert_eq!(titles, vec!["D", "C", "B", "A"]);
}

/// An entity nobody has written has no history, which is not an error.
#[tokio::test(flavor = "multi_thread")]
async fn history_of_an_unwritten_entity_is_empty() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let revisions = history(&mut c, "hist-none", "never-written").await;
    assert_eq!(revisions, [] as [trogon_atlas_proto::EntityRevision; 0]);
}
