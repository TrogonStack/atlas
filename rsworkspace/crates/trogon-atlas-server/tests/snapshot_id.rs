//! In-process gRPC integration tests for `GetSnapshotId` (G10: addressable
//! whole-model snapshot identity). Same harness as `branching.rs`: a real
//! tonic server over an ephemeral JetStream store.
//!
//! The store is shared across tests in this file, so every test scopes its
//! request to its own namespace. That is not just isolation hygiene: the
//! namespace scope is itself part of the contract (two ids are comparable
//! only over the same scope), so scoping every assertion exercises it.

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

fn with_branch<T>(body: T, branch: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).expect("valid header value"),
    );
    req
}

async fn snapshot(c: &mut EventModelServiceClient<Channel>, ns: &str) -> pb::GetSnapshotIdResponse {
    c.get_snapshot_id(pb::GetSnapshotIdRequest {
        namespaces: vec![ns.into()],
        include_entries: false,
    })
    .await
    .unwrap()
    .into_inner()
}

/// The base case: an id is stable while nothing changes, and moves when
/// anything does. Without both halves it is not a usable change detector.
#[tokio::test(flavor = "multi_thread")]
async fn id_is_stable_until_content_changes() {
    let mut c = client(&start_server().await).await;
    let ns = "snapid-stable";

    let empty = snapshot(&mut c, ns).await;
    assert_eq!(empty.entity_count, 0);
    assert_eq!(empty.snapshot_id.len(), 64);
    assert!(!empty.truncated);

    c.put_entity(put_req(event(ns, "a", "A"))).await.unwrap();
    let one = snapshot(&mut c, ns).await;
    assert_eq!(one.entity_count, 1);
    assert_ne!(one.snapshot_id, empty.snapshot_id);

    // Repeated reads of an unchanged store agree.
    assert_eq!(snapshot(&mut c, ns).await.snapshot_id, one.snapshot_id);

    // A no-op put must not move the id: the content is identical.
    c.put_entity(put_req(event(ns, "a", "A"))).await.unwrap();
    assert_eq!(
        snapshot(&mut c, ns).await.snapshot_id,
        one.snapshot_id,
        "a no-op put rewrites nothing, so the state is the same state"
    );

    // A real edit must move it.
    c.put_entity(put_req(event(ns, "a", "A prime")))
        .await
        .unwrap();
    let edited = snapshot(&mut c, ns).await;
    assert_ne!(edited.snapshot_id, one.snapshot_id);
    assert_eq!(edited.entity_count, 1);

    // Adding an entity moves it again.
    c.put_entity(put_req(event(ns, "b", "B"))).await.unwrap();
    let two = snapshot(&mut c, ns).await;
    assert_ne!(two.snapshot_id, edited.snapshot_id);
    assert_eq!(two.entity_count, 2);

    // Deleting back to the previous set returns to the previous id: the id
    // names a state, not a history.
    c.delete_entity(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: "b".into(),
            version: 1,
        }),
        if_match: String::new(),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
    })
    .await
    .unwrap();
    assert_eq!(snapshot(&mut c, ns).await.snapshot_id, edited.snapshot_id);
}

/// The scope is part of the identity. An id computed over one namespace is
/// not an id for the whole store, and the response must make that so.
#[tokio::test(flavor = "multi_thread")]
async fn namespace_scope_bounds_the_identity() {
    let mut c = client(&start_server().await).await;
    let (left, right) = ("snapid-scope-l", "snapid-scope-r");

    c.put_entity(put_req(event(left, "a", "A"))).await.unwrap();
    let before_right = snapshot(&mut c, left).await;

    // Writing to another namespace must not disturb this namespace's id.
    c.put_entity(put_req(event(right, "a", "A"))).await.unwrap();
    assert_eq!(
        snapshot(&mut c, left).await.snapshot_id,
        before_right.snapshot_id
    );

    // Identical content under different namespaces has different ids: the
    // key participates in the hash, not only the entity content.
    assert_ne!(
        snapshot(&mut c, right).await.snapshot_id,
        before_right.snapshot_id
    );

    // The union is its own scope, distinct from either half.
    let both = c
        .get_snapshot_id(pb::GetSnapshotIdRequest {
            namespaces: vec![left.into(), right.into()],
            include_entries: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(both.entity_count, 2);
    assert_ne!(both.snapshot_id, before_right.snapshot_id);

    // Scope order must not matter.
    let reversed = c
        .get_snapshot_id(pb::GetSnapshotIdRequest {
            namespaces: vec![right.into(), left.into()],
            include_entries: false,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reversed.snapshot_id, both.snapshot_id);
}

/// `include_entries` is what turns "something changed" into "this changed".
#[tokio::test(flavor = "multi_thread")]
async fn entries_locate_the_change() {
    let mut c = client(&start_server().await).await;
    let ns = "snapid-entries";

    c.put_entity(put_req(event(ns, "a", "A"))).await.unwrap();
    c.put_entity(put_req(event(ns, "b", "B"))).await.unwrap();

    let detailed = |c: &mut EventModelServiceClient<Channel>| {
        let ns = ns.to_string();
        let mut c = c.clone();
        async move {
            c.get_snapshot_id(pb::GetSnapshotIdRequest {
                namespaces: vec![ns],
                include_entries: true,
            })
            .await
            .unwrap()
            .into_inner()
        }
    };

    let before = detailed(&mut c).await;
    assert_eq!(before.entries.len(), 2);
    // Sorted by key, as the proto promises.
    let keys: Vec<&str> = before.entries.iter().map(|e| e.key.as_str()).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted);
    for entry in &before.entries {
        assert_eq!(entry.content_hash.len(), 64);
        assert_eq!(
            entry.entity.as_ref().unwrap().kind,
            pb::EntityKind::Event as i32
        );
    }

    c.put_entity(put_req(event(ns, "b", "B prime")))
        .await
        .unwrap();
    let after = detailed(&mut c).await;

    let changed: Vec<&str> = after
        .entries
        .iter()
        .zip(&before.entries)
        .filter(|(a, b)| a.content_hash != b.content_hash)
        .map(|(a, _)| a.key.as_str())
        .collect();
    assert_eq!(changed.len(), 1, "exactly one entity moved");
    assert!(changed[0].contains(".b."), "the edited one: {changed:?}");

    // Omitting entries is the default and must stay cheap.
    assert_eq!(
        snapshot(&mut c, ns).await.entries,
        [] as [trogon_atlas_proto::SnapshotEntry; 0]
    );
}

/// A branch has its own identity, which is what makes the id usable as a
/// cache key for branch-scoped work.
#[tokio::test(flavor = "multi_thread")]
async fn branch_view_has_its_own_identity() {
    let mut c = client(&start_server().await).await;
    let ns = "snapid-branch";
    let branch = "snapid-branch-b";

    c.put_entity(put_req(event(ns, "a", "A"))).await.unwrap();
    let baseline = snapshot(&mut c, ns).await;

    c.create_branch(pb::CreateBranchRequest {
        name: branch.into(),
        doc: "snapshot id test".into(),
    })
    .await
    .unwrap();

    let branch_req = || pb::GetSnapshotIdRequest {
        namespaces: vec![ns.into()],
        include_entries: false,
    };

    // A fresh branch is content-identical to baseline, so its id matches.
    let fresh = c
        .get_snapshot_id(with_branch(branch_req(), branch))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        fresh.snapshot_id, baseline.snapshot_id,
        "an untouched branch is the same state as its baseline"
    );

    // A branch write moves the branch id and leaves baseline alone.
    c.put_entity(with_branch(put_req(event(ns, "a", "A on branch")), branch))
        .await
        .unwrap();
    let diverged = c
        .get_snapshot_id(with_branch(branch_req(), branch))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(diverged.snapshot_id, baseline.snapshot_id);
    assert_eq!(
        snapshot(&mut c, ns).await.snapshot_id,
        baseline.snapshot_id,
        "branch WIP must stay invisible to baseline"
    );
}

/// Identity is over content, not storage bookkeeping: a model restored from
/// a manifest gets fresh uids and created_at stamps, and must still land on
/// the id it was pinned at.
#[tokio::test(flavor = "multi_thread")]
async fn identity_survives_delete_and_recreate() {
    let mut c = client(&start_server().await).await;
    let ns = "snapid-recreate";

    c.put_entity(put_req(event(ns, "a", "A"))).await.unwrap();
    let before = snapshot(&mut c, ns).await;

    c.delete_entity(pb::DeleteEntityRequest {
        operation_id: String::new(),
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: "a".into(),
            version: 1,
        }),
        if_match: String::new(),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
    })
    .await
    .unwrap();
    c.put_entity(put_req(event(ns, "a", "A"))).await.unwrap();

    assert_eq!(
        snapshot(&mut c, ns).await.snapshot_id,
        before.snapshot_id,
        "a new uid is provenance, not content"
    );
}
