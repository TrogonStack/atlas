//! In-process gRPC integration tests for the durable changeset log
//! (G1/G13: who changed what, together, and when).
//!
//! Same harness as `snapshot_id.rs`: a real tonic server over an ephemeral
//! JetStream store. The store is shared across tests in this file, so every
//! test writes into its own namespace and filters the log by the entities it
//! owns rather than assuming it is the only writer.

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

fn event(ns: &str, slug: &str, title: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
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

/// Attach the self-asserted author headers. Without the auth stack in front
/// of the service these are the author of record; with it, the authenticated
/// principal wins (see `EventModelServiceImpl::author_from_metadata`).
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

/// Every changeset that touched `namespace`, newest first. Pages until the
/// log is exhausted so a test never depends on how much unrelated traffic
/// the shared store saw.
async fn changesets_touching(
    c: &mut EventModelServiceClient<Channel>,
    namespace: &str,
) -> Vec<pb::Changeset> {
    let mut out = Vec::new();
    let mut page_token = String::new();
    loop {
        let resp = c
            .list_changesets(pb::ListChangesetsRequest {
                page_token: page_token.clone(),
                page_size: 100,
                branch: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        out.extend(resp.changesets.into_iter().filter(|cs| {
            cs.ops.iter().any(|op| {
                op.entity
                    .as_ref()
                    .and_then(|e| e.id.as_ref())
                    .is_some_and(|id| id.namespace == namespace)
            })
        }));
        if resp.next_page_token.is_empty() {
            return out;
        }
        page_token = resp.next_page_token;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_records_one_changeset_with_the_calling_author() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(
        put_req(event("cs-put", "order-placed", "Order placed")),
        "alice",
    ))
    .await
    .unwrap();

    let found = changesets_touching(&mut c, "cs-put").await;
    assert_eq!(found.len(), 1, "one mutation RPC records one changeset");
    let cs = &found[0];
    assert_eq!(cs.author, "alice");
    assert_eq!(cs.rpc, "PutEntity");
    assert!(cs.branch.is_empty(), "a baseline write records no branch");
    assert_ne!(cs.id, "");
    assert_ne!(cs.at, "");
    assert_eq!(cs.ops.len(), 1);
    assert_eq!(cs.ops[0].kind, pb::change_event::Kind::Put as i32);
}

/// The id on a change event must resolve through `GetChangeset`. That join
/// is the whole point of denormalising it onto the event.
#[tokio::test(flavor = "multi_thread")]
async fn change_events_join_back_to_their_changeset() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let before = c
        .list_changes(pb::ListChangesRequest {
            since_token: String::new(),
            scopes: Vec::new(),
            page_size: 1,
        })
        .await
        .unwrap()
        .into_inner()
        .next_token;

    c.put_entity(as_author(
        put_req(event("cs-join", "shipped", "Shipped")),
        "bob",
    ))
    .await
    .unwrap();

    let changes = c
        .list_changes(pb::ListChangesRequest {
            since_token: before,
            scopes: Vec::new(),
            page_size: 100,
        })
        .await
        .unwrap()
        .into_inner();
    let evt = changes
        .events
        .iter()
        .find(|e| {
            e.entity
                .as_ref()
                .and_then(|r| r.id.as_ref())
                .is_some_and(|id| id.namespace == "cs-join")
        })
        .expect("the put must emit a change event");
    assert_eq!(evt.author, "bob", "attribution rides along on the event");
    assert_ne!(evt.changeset_id, "");

    let cs = c
        .get_changeset(pb::GetChangesetRequest {
            id: evt.changeset_id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .changeset
        .expect("the referenced changeset must exist");
    assert_eq!(cs.id, evt.changeset_id);
    assert_eq!(cs.author, "bob");
}

/// One batch is one changeset, not one per op: "what landed together" is the
/// question the log exists to answer.
#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_records_a_single_changeset_for_every_op() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let ops = ["a", "b", "c"]
        .into_iter()
        .map(|slug| pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(put_req(event(
                "cs-batch", slug, slug,
            )))),
        })
        .collect();

    c.batch_mutate(as_author(
        pb::BatchMutateRequest {
            operation_id: String::new(),
            ops,
            validate_only: false,
        },
        "carol",
    ))
    .await
    .unwrap();

    let found = changesets_touching(&mut c, "cs-batch").await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].rpc, "BatchMutate");
    assert_eq!(found[0].author, "carol");
    assert_eq!(found[0].ops.len(), 3);
}

/// A validate-only batch writes nothing, so it must record nothing. An
/// empty changeset would be noise in the log and a false audit trail.
#[tokio::test(flavor = "multi_thread")]
async fn validate_only_records_nothing() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.batch_mutate(as_author(
        pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(put_req(event(
                    "cs-dry", "never", "Never",
                )))),
            }],
            validate_only: true,
        },
        "dave",
    ))
    .await
    .unwrap();

    assert_eq!(
        changesets_touching(&mut c, "cs-dry").await,
        [] as [trogon_atlas_proto::Changeset; 0]
    );
}

/// Re-putting identical content is a no-op write. It must not manufacture a
/// changeset: nothing changed.
#[tokio::test(flavor = "multi_thread")]
async fn noop_put_records_nothing() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    for _ in 0..2 {
        c.put_entity(as_author(put_req(event("cs-noop", "same", "Same")), "erin"))
            .await
            .unwrap();
    }

    let found = changesets_touching(&mut c, "cs-noop").await;
    assert_eq!(
        found.len(),
        1,
        "the second, content-identical put changed nothing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_records_a_delete_op() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.put_entity(as_author(put_req(event("cs-del", "gone", "Gone")), "frank"))
        .await
        .unwrap();
    c.delete_entity(as_author(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "cs-del".into(),
                slug: "gone".into(),
                version: 1,
            }),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        },
        "frank",
    ))
    .await
    .unwrap();

    let found = changesets_touching(&mut c, "cs-del").await;
    assert_eq!(
        found.len(),
        2,
        "the put and the delete are separate landings"
    );
    assert_eq!(found[0].rpc, "DeleteEntity");
    assert_eq!(found[0].ops[0].kind, pb::change_event::Kind::Deleted as i32);
}

/// Branch writes never reach the change feed, so the changeset log is the
/// only record of who edited a branch. The `branch` field keeps them
/// separable from baseline history.
#[tokio::test(flavor = "multi_thread")]
async fn branch_writes_are_recorded_and_filterable() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    c.create_branch(tonic::Request::new(pb::CreateBranchRequest {
        name: "cs-branch".into(),
        doc: "changeset attribution".into(),
    }))
    .await
    .unwrap();

    c.put_entity(on_branch(
        as_author(put_req(event("cs-onbranch", "draft", "Draft")), "grace"),
        "cs-branch",
    ))
    .await
    .unwrap();

    let scoped = c
        .list_changesets(pb::ListChangesetsRequest {
            page_token: String::new(),
            page_size: 100,
            branch: "cs-branch".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(scoped.changesets.len(), 1);
    assert_eq!(scoped.changesets[0].branch, "cs-branch");
    assert_eq!(scoped.changesets[0].author, "grace");
}

/// Paging must terminate and must not repeat a row, whatever the page size.
#[tokio::test(flavor = "multi_thread")]
async fn paging_covers_the_log_exactly_once() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    for i in 0..5 {
        c.put_entity(as_author(
            put_req(event("cs-page", &format!("e{i}"), "E")),
            "heidi",
        ))
        .await
        .unwrap();
    }

    let mut seen: Vec<String> = Vec::new();
    let mut page_token = String::new();
    loop {
        let resp = c
            .list_changesets(pb::ListChangesetsRequest {
                page_token: page_token.clone(),
                page_size: 2,
                branch: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        seen.extend(resp.changesets.iter().map(|cs| cs.id.clone()));
        if resp.next_page_token.is_empty() {
            break;
        }
        page_token = resp.next_page_token;
    }

    let mut deduped = seen.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(seen.len(), deduped.len(), "paging must not repeat a row");

    let mut descending = seen.clone();
    descending.sort();
    descending.reverse();
    assert_eq!(seen, descending, "the log reads newest first");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_changeset_rejects_an_empty_id_and_reports_a_missing_one() {
    let endpoint = start_server().await;
    let mut c = client(&endpoint).await;

    let err = c
        .get_changeset(pb::GetChangesetRequest { id: String::new() })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let err = c
        .get_changeset(pb::GetChangesetRequest {
            id: "0190a0f1-0000-7000-8000-000000000000".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::NotFound);
}
