//! In-process gRPC integration tests for the ownership axis of
//! authorization: a principal bound to a `parent` in the namespace registry.
//!
//! Same harness as `tests/namespace_scope.rs`: a real tonic server with the
//! real `BearerAuth` + `AuthzLayer` stack over an ephemeral `JetStream`
//! store.
//!
//! Ownership differs from the write allow-list in two ways, and both are what
//! these tests are for:
//!
//! * It constrains **reads**. An allow-list bounds the blast radius of a
//!   caller you already trust to see everything; ownership is a tenancy
//!   boundary, and a tenancy boundary that leaks on read is not one.
//! * It is resolved from the **registry at runtime**, not from the token
//!   file. Moving a namespace has to take effect without a restart.
//!
//! Covers: read isolation across owners, write denial across owners,
//! claim-on-first-write, an unbound principal seeing everything (which is
//! what makes the feature opt-in), registration under the caller's own
//! parent, and a move taking effect immediately.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{
    metadata::MetadataValue, service::interceptor::InterceptedService, transport::Channel, Code,
};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    service::EventModelServiceImpl,
};

/// Writer bound to owner `acme`.
const ACME_TOKEN: &str = "own-acme-token";
/// Writer bound to owner `beta`.
const BETA_TOKEN: &str = "own-beta-token";
/// Writer with no `parent`: the shape of every principal in a registry
/// written before ownership existed.
const UNBOUND_TOKEN: &str = "own-unbound-token";
/// Admin with no `parent`. Moving a namespace is Admin-only.
const ADMIN_TOKEN: &str = "own-admin-token";
/// Agent delegated by owner `alice`: holds rights over `@alice:*` owned
/// branches, and only those.
const ALICE_AGENT_TOKEN: &str = "own-alice-agent-token";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.acme-writer]
role = "writer"
tokens = ["{ACME_TOKEN}"]
parent = "acme"

[principals.beta-writer]
role = "writer"
tokens = ["{BETA_TOKEN}"]
parent = "beta"

[principals.unbound-writer]
role = "writer"
tokens = ["{UNBOUND_TOKEN}"]

[principals.admin-svc]
role = "admin"
tokens = ["{ADMIN_TOKEN}"]

[principals.alice-agent]
role = "writer"
kind = "agent"
tokens = ["{ALICE_AGENT_TOKEN}"]
parent = "alice"
namespaces = ["alice-*"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// One server per test. Ownership state is global to a store, so tests that
/// claim the same namespace name under different owners must not share one.
async fn client() -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc = EventModelServiceServer::new(svc);

    let auth = BearerAuth::new(Arc::new(make_registry()));
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new().layer(AuthzLayer).service(grpc),
        auth,
    );

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

fn event(ns: &str, slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
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

fn authed<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
    );
    req
}

/// Like `authed`, but targets a branch: for `PutEntityRequest`, which carries
/// no branch field of its own and relies on the `x-trogon-atlas-branch` header
/// instead.
fn authed_on_branch<T>(body: T, token: &str, branch: &str) -> tonic::Request<T> {
    let mut req = authed(body, token);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    req
}

async fn namespaces_seen(
    c: &mut EventModelServiceClient<Channel>,
    token: &str,
) -> Vec<(String, String)> {
    let resp = c
        .list_namespaces(authed(pb::ListNamespacesRequest {}, token))
        .await
        .unwrap()
        .into_inner();
    let mut seen: Vec<(String, String)> = resp
        .namespaces
        .into_iter()
        .map(|ns| (ns.id, ns.parent))
        .collect();
    seen.sort();
    seen
}

// ---------------------------------------------------------------------------
// Claim on first write
// ---------------------------------------------------------------------------

/// The bootstrap problem: a tenant with an empty registry must be able to
/// start writing without an operator declaring namespaces first.
#[tokio::test(flavor = "multi_thread")]
async fn a_bound_writer_claims_the_namespace_it_first_writes() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("claimed", "placed")), ACME_TOKEN))
        .await
        .expect("writing into an unclaimed namespace must claim it, not fail");

    assert_eq!(
        namespaces_seen(&mut c, ACME_TOKEN).await,
        vec![("claimed".to_string(), "acme".to_string())],
        "the namespace must now be registered to the writer's parent",
    );
}

/// The claim is durable and repeated writes do not disturb it.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_writes_do_not_re_register() {
    let mut c = client().await;

    for slug in ["one", "two", "three"] {
        c.put_entity(authed(put_req(event("stable", slug)), ACME_TOKEN))
            .await
            .unwrap();
    }

    assert_eq!(
        namespaces_seen(&mut c, ACME_TOKEN).await,
        vec![("stable".to_string(), "acme".to_string())],
    );
}

// ---------------------------------------------------------------------------
// Write isolation
// ---------------------------------------------------------------------------

/// The write half of the tenancy boundary. `beta` cannot write into a
/// namespace `acme` claimed, no matter that its own allow-list is empty.
#[tokio::test(flavor = "multi_thread")]
async fn a_bound_writer_cannot_write_into_another_owners_namespace() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("contested", "mine")), ACME_TOKEN))
        .await
        .unwrap();

    let err = c
        .put_entity(authed(put_req(event("contested", "yours")), BETA_TOKEN))
        .await
        .expect_err("a cross-owner write must be denied");
    assert_eq!(err.code(), Code::PermissionDenied);
    assert!(
        !err.message().contains("acme"),
        "the denial must not name the holder: a caller that can ask this \
         question for any name could enumerate its neighbours one guess at a \
         time: {}",
        err.message(),
    );
    assert!(
        err.message().contains("register a namespace of your own"),
        "the denial should still say how to get unstuck: {}",
        err.message(),
    );
}

/// An unbound principal is the pre-ownership shape and must be unaffected,
/// which is the property that makes this safe to ship to existing
/// deployments.
#[tokio::test(flavor = "multi_thread")]
async fn an_unbound_writer_is_not_constrained_by_ownership() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("shared", "claimed")), ACME_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("shared", "also")), UNBOUND_TOKEN))
        .await
        .expect("a principal with no parent must not be fenced out");
}

// ---------------------------------------------------------------------------
// Read isolation
// ---------------------------------------------------------------------------

/// The read half, and the reason ownership could not be expressed as another
/// allow-list: each tenant's snapshot contains only its own entities.
#[tokio::test(flavor = "multi_thread")]
async fn owners_cannot_read_each_others_entities() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("acme-only", "secret")), ACME_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("beta-only", "secret")), BETA_TOKEN))
        .await
        .unwrap();

    let acme_list = c
        .list_entities(authed(
            pb::ListEntitiesRequest {
                kinds: vec![pb::EntityKind::Event as i32],
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let acme_namespaces: Vec<String> = acme_list
        .entities
        .iter()
        .filter_map(|e| match e.kind.as_ref() {
            Some(pb::entity::Kind::Event(ev)) => ev.id.as_ref().map(|id| id.namespace.clone()),
            _ => None,
        })
        .collect();
    assert!(
        acme_namespaces.iter().all(|ns| ns == "acme-only"),
        "acme must see only its own entities, saw {acme_namespaces:?}",
    );
    assert!(!acme_namespaces.is_empty(), "acme must see its own entity");

    assert_eq!(
        namespaces_seen(&mut c, ACME_TOKEN).await,
        vec![("acme-only".to_string(), "acme".to_string())],
    );
    assert_eq!(
        namespaces_seen(&mut c, BETA_TOKEN).await,
        vec![("beta-only".to_string(), "beta".to_string())],
    );
}

/// Full-text search is the one read that does not go through the snapshot,
/// so it needs its own boundary test. Before the lens was folded into the
/// namespace filter it answered from an index spanning every namespace, and
/// a bound caller got the other owner's titles and excerpts back.
#[tokio::test(flavor = "multi_thread")]
async fn search_does_not_cross_the_owner_boundary() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("acme-only", "widget")), ACME_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("beta-only", "widget")), BETA_TOKEN))
        .await
        .unwrap();

    let namespaces_in = |resp: pb::SearchEntitiesResponse| -> Vec<String> {
        resp.results
            .iter()
            .filter_map(|r| match r.entity.as_ref()?.kind.as_ref()? {
                pb::entity::Kind::Event(ev) => ev.id.as_ref().map(|id| id.namespace.clone()),
                _ => None,
            })
            .collect()
    };

    let search =
        |c: &mut EventModelServiceClient<Channel>, token: &'static str, ns: Vec<String>| {
            let mut c = c.clone();
            async move {
                c.search_entities(authed(
                    pb::SearchEntitiesRequest {
                        query: "widget".into(),
                        namespaces: ns,
                        limit: 50,
                        ..Default::default()
                    },
                    token,
                ))
                .await
                .unwrap()
                .into_inner()
            }
        };

    let acme = namespaces_in(search(&mut c, ACME_TOKEN, Vec::new()).await);
    assert_eq!(
        acme,
        vec!["acme-only".to_string()],
        "acme must see only its own hit, saw {acme:?}",
    );

    let beta = namespaces_in(search(&mut c, BETA_TOKEN, Vec::new()).await);
    assert_eq!(
        beta,
        vec!["beta-only".to_string()],
        "beta must see only its own hit, saw {beta:?}",
    );

    // Naming the other owner's namespace explicitly must not widen the lens;
    // it narrows to the empty intersection, which is not the same as the
    // empty filter that means "every namespace".
    let targeted = namespaces_in(search(&mut c, ACME_TOKEN, vec!["beta-only".into()]).await);
    assert!(
        targeted.is_empty(),
        "asking for another owner's namespace by name must return nothing, saw {targeted:?}",
    );

    let unbound = namespaces_in(search(&mut c, UNBOUND_TOKEN, Vec::new()).await);
    assert!(
        unbound.contains(&"acme-only".to_string()) && unbound.contains(&"beta-only".to_string()),
        "an unbound caller still searches the whole store, saw {unbound:?}",
    );
}

/// An unbound reader still sees the whole store, including both tenants.
#[tokio::test(flavor = "multi_thread")]
async fn an_unbound_reader_sees_every_owner() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("from-acme", "x")), ACME_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("from-beta", "x")), BETA_TOKEN))
        .await
        .unwrap();

    let seen = namespaces_seen(&mut c, UNBOUND_TOKEN).await;
    assert!(seen.contains(&("from-acme".to_string(), "acme".to_string())));
    assert!(seen.contains(&("from-beta".to_string(), "beta".to_string())));
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// The escape from first-come-first-served on bare names: the second tenant
/// to want `orders` registers explicitly and gets a minted id.
#[tokio::test(flavor = "multi_thread")]
async fn two_owners_can_register_the_same_name() {
    let mut c = client().await;

    let acme = c
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "orders".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let beta = c
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "orders".into(),
                parent: String::new(),
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();

    let acme = acme.namespace.unwrap();
    let beta = beta.namespace.unwrap();
    assert_eq!(acme.name, "orders");
    assert_eq!(beta.name, "orders");
    assert_eq!(acme.parent, "acme");
    assert_eq!(beta.parent, "beta");
    assert_ne!(acme.id, beta.id, "the ids must differ or the keys collide");
    assert!(acme.id.starts_with("ns_"));
    assert!(beta.id.starts_with("ns_"));
}

/// A bound caller may not register into somebody else's subtree, even though
/// registering is a Writer-level operation.
#[tokio::test(flavor = "multi_thread")]
async fn a_bound_caller_cannot_register_under_another_parent() {
    let mut c = client().await;

    let err = c
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "smuggled".into(),
                parent: "beta".into(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect_err("registering under another parent must be denied");
    assert_eq!(err.code(), Code::PermissionDenied);
}

/// Registering the same name twice under one owner returns the same id
/// rather than minting a second namespace.
#[tokio::test(flavor = "multi_thread")]
async fn registering_twice_reports_not_created() {
    let mut c = client().await;

    let first = c
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "idem".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    let second = c
        .register_namespace(authed(
            pb::RegisterNamespaceRequest {
                name: "idem".into(),
                parent: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.namespace.unwrap().id, second.namespace.unwrap().id,);
}

// ---------------------------------------------------------------------------
// Moving
// ---------------------------------------------------------------------------

/// The second hard requirement, end to end: a move is a metadata edit that
/// takes effect on the next request, with no entity rewritten and no
/// restart.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_transfers_visibility_immediately() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("movable", "x")), ACME_TOKEN))
        .await
        .unwrap();
    assert_eq!(
        namespaces_seen(&mut c, BETA_TOKEN).await,
        vec![],
        "beta must not see it before the move",
    );

    c.move_namespace(authed(
        pb::MoveNamespaceRequest {
            id: "movable".into(),
            parent: "beta".into(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .expect("an admin may move a namespace");

    assert_eq!(
        namespaces_seen(&mut c, BETA_TOKEN).await,
        vec![("movable".to_string(), "beta".to_string())],
        "beta must see it on the very next request",
    );
    assert_eq!(
        namespaces_seen(&mut c, ACME_TOKEN).await,
        vec![],
        "acme must stop seeing it",
    );

    // The entity itself was never rewritten: it is still reachable under the
    // same id, now by its new owner.
    c.put_entity(authed(put_req(event("movable", "y")), BETA_TOKEN))
        .await
        .expect("the new owner may write into the moved namespace");
    let err = c
        .put_entity(authed(put_req(event("movable", "z")), ACME_TOKEN))
        .await
        .expect_err("the old owner must lose write access");
    assert_eq!(err.code(), Code::PermissionDenied);
}

/// A move crosses a tenancy boundary, so no principal bound inside one may
/// perform it. Enforced by the role map rather than by the handler.
#[tokio::test(flavor = "multi_thread")]
async fn a_writer_may_not_move_a_namespace() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("not-yours", "x")), ACME_TOKEN))
        .await
        .unwrap();

    let err = c
        .move_namespace(authed(
            pb::MoveNamespaceRequest {
                id: "not-yours".into(),
                parent: "acme".into(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect_err("MoveNamespace is Admin-only");
    assert_eq!(err.code(), Code::PermissionDenied);
}

// ---------------------------------------------------------------------------
// Direct-by-key reads
// ---------------------------------------------------------------------------
//
// Every read that goes through the snapshot inherits the owner filter for
// free. These do not: they address the store by exact key, so each one needs
// its own gate, and each one is a complete bypass of the tenancy boundary if
// it is missing.

fn key(ns: &str, slug: &str) -> pb::EntityRef {
    pb::EntityRef {
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }),
    }
}

/// Knowing the exact key must not be enough. `GetEntity` never consults the
/// snapshot, so without its own gate an owner could read the whole store one
/// key at a time.
#[tokio::test(flavor = "multi_thread")]
async fn a_key_lookup_does_not_cross_the_owner_boundary() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("acme-vault", "secret")), ACME_TOKEN))
        .await
        .unwrap();

    // The owner reads it fine.
    c.get_entity(authed(
        pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(pb::Id {
                namespace: "acme-vault".into(),
                slug: "secret".into(),
                version: 1,
            }),
        },
        ACME_TOKEN,
    ))
    .await
    .expect("the owner must be able to read its own entity");

    let err = c
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: "acme-vault".into(),
                    slug: "secret".into(),
                    version: 1,
                }),
            },
            BETA_TOKEN,
        ))
        .await
        .expect_err("another owner must not read it by exact key");

    // NOT_FOUND, not PERMISSION_DENIED: the latter confirms the entity
    // exists, which turns the endpoint into an existence oracle over another
    // tenant's model.
    assert_eq!(
        err.code(),
        Code::NotFound,
        "an invisible entity must be indistinguishable from a missing one, got {err:?}",
    );
}

/// The same gate applied to a batch, and applied per key rather than to the
/// whole call: one out-of-scope key must not fail the visible ones, and dense
/// mode must keep reporting positionally.
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_read_reports_invisible_keys_as_missing() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("beta-own", "mine")), BETA_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("acme-own", "theirs")), ACME_TOKEN))
        .await
        .unwrap();

    let resp = c
        .batch_get_entities(authed(
            pb::BatchGetEntitiesRequest {
                keys: vec![
                    key("acme-own", "theirs"),
                    key("beta-own", "mine"),
                    key("beta-own", "never-written"),
                ],
                dense: true,
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.found,
        vec![false, true, false],
        "the other owner's key must read as absent, in its original position",
    );
    assert_eq!(resp.entities.len(), 3, "dense mode stays positional");
}

/// An entity's edit log is as sensitive as the entity: it carries every prior
/// revision of it.
#[tokio::test(flavor = "multi_thread")]
async fn entity_history_does_not_cross_the_owner_boundary() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("acme-log", "placed")), ACME_TOKEN))
        .await
        .unwrap();

    let history = pb::GetEntityHistoryRequest {
        r#ref: Some(key("acme-log", "placed")),
        ..Default::default()
    };

    c.get_entity_history(authed(history.clone(), ACME_TOKEN))
        .await
        .expect("the owner reads its own history");

    let err = c
        .get_entity_history(authed(history, BETA_TOKEN))
        .await
        .expect_err("another owner must not read it");
    assert_eq!(err.code(), Code::NotFound, "got {err:?}");
}

/// The change feed is a flat log over every namespace, so an unscoped poll
/// has to be filtered record by record.
#[tokio::test(flavor = "multi_thread")]
async fn the_change_feed_only_carries_the_callers_own_namespaces() {
    let mut c = client().await;

    let before = c
        .list_changes(authed(
            pb::ListChangesRequest {
                page_size: 100,
                ..Default::default()
            },
            UNBOUND_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .next_token;

    c.put_entity(authed(put_req(event("feed-acme", "a")), ACME_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("feed-beta", "b")), BETA_TOKEN))
        .await
        .unwrap();

    let namespaces = |resp: pb::ListChangesResponse| -> Vec<String> {
        let mut seen: Vec<String> = resp
            .events
            .into_iter()
            .filter_map(|e| e.entity.and_then(|r| r.id).map(|id| id.namespace))
            .collect();
        seen.sort();
        seen.dedup();
        seen
    };

    let poll = |token: &'static str, since: String| {
        let mut c = c.clone();
        async move {
            c.list_changes(authed(
                pb::ListChangesRequest {
                    since_token: since,
                    page_size: 100,
                    ..Default::default()
                },
                token,
            ))
            .await
            .unwrap()
            .into_inner()
        }
    };

    assert_eq!(
        namespaces(poll(ACME_TOKEN, before.clone()).await),
        vec!["feed-acme".to_string()],
        "a bound caller must not see another owner's writes in the feed",
    );
    assert_eq!(
        namespaces(poll(UNBOUND_TOKEN, before).await),
        vec!["feed-acme".to_string(), "feed-beta".to_string()],
        "an unbound caller still sees the whole feed",
    );
}

// ---------------------------------------------------------------------------
// Surfaces that filter by owned branch rather than refusing outright
// ---------------------------------------------------------------------------

/// A changeset, a branch diff and a branch listing each describe one unit of
/// work that may span several owners. A classic or baseline name carries no
/// owner at all, so it still can't be disclosed to a bound caller: it stays
/// invisible (an empty page, or a `NotFound` for a direct lookup) rather than
/// refusing the whole RPC the way it used to.
#[tokio::test(flavor = "multi_thread")]
async fn unpartitioned_surfaces_are_filtered_for_bound_callers() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("refused", "x")), ACME_TOKEN))
        .await
        .unwrap();

    let changesets = c
        .list_changesets(authed(pb::ListChangesetsRequest::default(), ACME_TOKEN))
        .await
        .expect("ListChangesets now filters instead of refusing a bound caller")
        .into_inner();
    assert!(
        changesets.changesets.is_empty(),
        "a baseline changeset carries no owner and must stay invisible, got {changesets:?}",
    );

    let err = c
        .get_changeset(authed(
            pb::GetChangesetRequest {
                id: "00000000-0000-0000-0000-000000000000".into(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect_err("an unattributed changeset id must not be disclosed");
    assert_eq!(err.code(), Code::NotFound, "got {err:?}");

    let branches = c
        .list_branches(authed(pb::ListBranchesRequest::default(), ACME_TOKEN))
        .await
        .expect("ListBranches now filters instead of refusing a bound caller")
        .into_inner();
    assert!(
        branches.branches.is_empty(),
        "a classic/global branch carries no owner and must stay invisible, got {branches:?}",
    );

    let err = c
        .diff_branch(authed(
            pb::DiffBranchRequest { name: "wip".into() },
            ACME_TOKEN,
        ))
        .await
        .expect_err("a classic/global branch diff must not be disclosed to a bound caller");
    assert_eq!(err.code(), Code::NotFound, "got {err:?}");
}

/// An owned branch (`@owner:name`) is attributed, so it is visible to the
/// owner that created it and to any agent that owner delegated to, but not to
/// a different owner's agent: delegation is per owner, never per namespace.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_delegated_by_one_owner_cannot_see_another_owners_branch() {
    let mut c = client().await;

    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "@bob:feature".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .expect("an unrestricted caller can create an owned branch on bob's behalf");

    let err = c
        .diff_branch(authed(
            pb::DiffBranchRequest {
                name: "@bob:feature".into(),
            },
            ALICE_AGENT_TOKEN,
        ))
        .await
        .expect_err("alice's agent is not delegated by bob and must not see bob's branch");
    assert_eq!(err.code(), Code::NotFound, "got {err:?}");

    let branches = c
        .list_branches(authed(
            pb::ListBranchesRequest::default(),
            ALICE_AGENT_TOKEN,
        ))
        .await
        .expect("ListBranches still serves a bound caller a filtered page")
        .into_inner();
    assert!(
        branches.branches.iter().all(|b| b.name != "@bob:feature"),
        "bob's branch must not be listed for alice's agent, got {branches:?}",
    );
}

/// ...and the same agent can see the branch its own owner created: delegation
/// is per owner, not per namespace, so it follows the owner's name alone.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_can_see_the_branch_its_own_owner_created() {
    let mut c = client().await;

    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "@alice:feature".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .expect("an unrestricted caller can create an owned branch on alice's behalf");

    c.diff_branch(authed(
        pb::DiffBranchRequest {
            name: "@alice:feature".into(),
        },
        ALICE_AGENT_TOKEN,
    ))
    .await
    .expect("alice's agent is delegated by alice and must see alice's branch");

    let branches = c
        .list_branches(authed(
            pb::ListBranchesRequest::default(),
            ALICE_AGENT_TOKEN,
        ))
        .await
        .expect("ListBranches serves a bound caller a filtered page")
        .into_inner();
    assert!(
        branches.branches.iter().any(|b| b.name == "@alice:feature"),
        "alice's branch must be listed for alice's agent, got {branches:?}",
    );
}

/// ...and the refusal is scoped to bound callers only. An unbound principal
/// is exactly as capable as it was before ownership existed.
#[tokio::test(flavor = "multi_thread")]
async fn unpartitioned_surfaces_still_serve_unbound_callers() {
    let mut c = client().await;

    c.list_changesets(authed(pb::ListChangesetsRequest::default(), UNBOUND_TOKEN))
        .await
        .expect("an unbound caller keeps the changeset log");
    c.list_branches(authed(pb::ListBranchesRequest::default(), UNBOUND_TOKEN))
        .await
        .expect("an unbound caller keeps the branch list");
}

/// `DiffEntities` is two key lookups wearing a diff for a hat, and
/// `GetLatestVersion` answers a namespace-and-slug question directly off the
/// store. Both bypass the snapshot, so both need their own gate.
#[tokio::test(flavor = "multi_thread")]
async fn the_remaining_key_shaped_reads_are_gated_too() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("acme-diff", "one")), ACME_TOKEN))
        .await
        .unwrap();
    c.put_entity(authed(put_req(event("acme-diff", "two")), ACME_TOKEN))
        .await
        .unwrap();

    let err = c
        .diff_entities(authed(
            pb::DiffEntitiesRequest {
                a: Some(key("acme-diff", "one")),
                b: Some(key("acme-diff", "two")),
            },
            BETA_TOKEN,
        ))
        .await
        .expect_err("diffing another owner's entities must not reveal them");
    assert_eq!(err.code(), Code::NotFound, "got {err:?}");

    // An empty namespace means "every namespace", so the filter has to run on
    // the results rather than on the request.
    let latest = c
        .get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: String::new(),
                slug: "one".into(),
            },
            BETA_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        latest.entity.is_none(),
        "a wildcard version lookup must not reach across the boundary",
    );

    let mine = c
        .get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "acme-diff".into(),
                slug: "one".into(),
            },
            ACME_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(mine.entity.is_some(), "the owner still gets its own answer");
}

/// A classic/global branch name carries no owner, and a changeset carries no
/// owner at all, so neither gives a bound caller's delegation anything to
/// attach to: creating, rebasing, merging or deleting a classic branch, or
/// reverting a changeset, stays refused. Contrast the tests below, where the
/// branch is owned (`@owner:name`) and the caller is delegated by that owner.
#[tokio::test(flavor = "multi_thread")]
async fn branch_and_changeset_mutations_are_refused_to_bound_callers() {
    let mut c = client().await;

    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "wip".into(),
            doc: "unbound callers keep branches".into(),
        },
        UNBOUND_TOKEN,
    ))
    .await
    .expect("an unbound caller keeps the branch lifecycle");

    let denied = [
        c.create_branch(authed(
            pb::CreateBranchRequest {
                name: "mine".into(),
                doc: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .err(),
        c.update_branch(authed(
            pb::UpdateBranchRequest { name: "wip".into() },
            ACME_TOKEN,
        ))
        .await
        .err(),
        c.merge_branch(authed(
            pb::MergeBranchRequest {
                operation_id: String::new(),
                name: "wip".into(),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .err(),
        c.delete_branch(authed(
            pb::DeleteBranchRequest { name: "wip".into() },
            ACME_TOKEN,
        ))
        .await
        .err(),
        c.revert_changeset(authed(
            pb::RevertChangesetRequest {
                operation_id: String::new(),
                id: "whatever".into(),
                ..Default::default()
            },
            ACME_TOKEN,
        ))
        .await
        .err(),
    ];

    for err in denied {
        let err = err.expect("a bound caller must be refused, not served");
        assert_eq!(
            err.code(),
            Code::PermissionDenied,
            "expected a plain refusal, got {err:?}",
        );
    }

    // The refusal must not have destroyed anything on the way past.
    let branches = c
        .list_branches(authed(pb::ListBranchesRequest::default(), UNBOUND_TOKEN))
        .await
        .unwrap()
        .into_inner();
    assert!(
        branches.branches.iter().any(|b| b.name == "wip"),
        "a refused delete must leave the branch standing",
    );
}

/// Item B, write RPCs: an owner's agent can do real work on its own owned
/// branch -- create it, write inside a granted namespace, and merge it --
/// without needing an unrestricted token to do any of it on its behalf.
#[tokio::test(flavor = "multi_thread")]
async fn an_owners_agent_creates_and_merges_its_own_owned_branch_within_scope() {
    let mut c = client().await;

    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "@alice:x".into(),
            doc: String::new(),
        },
        ALICE_AGENT_TOKEN,
    ))
    .await
    .expect("alice's agent authorizes alice and must be able to create alice's own branch");

    c.put_entity(authed_on_branch(
        put_req(event("alice-ns", "widget")),
        ALICE_AGENT_TOKEN,
        "@alice:x",
    ))
    .await
    .expect("writing inside a granted namespace on the agent's own branch must succeed");

    c.merge_branch(authed(
        pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "@alice:x".into(),
            ..Default::default()
        },
        ALICE_AGENT_TOKEN,
    ))
    .await
    .expect("merging its own owned branch, within its granted namespaces, must succeed");

    let latest = c
        .get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "alice-ns".into(),
                slug: "widget".into(),
            },
            ALICE_AGENT_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        latest.entity.is_some(),
        "the merge must have landed the entity on baseline",
    );
}

/// Delegation is per owner: alice's agent authorizes alice, never bob, so it
/// cannot create or update a branch owned by a different owner.
#[tokio::test(flavor = "multi_thread")]
async fn an_owners_agent_is_refused_on_another_owners_branch() {
    let mut c = client().await;

    let err = c
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "@bob:x".into(),
                doc: String::new(),
            },
            ALICE_AGENT_TOKEN,
        ))
        .await
        .expect_err("alice's agent must not create a branch owned by bob");
    assert_eq!(err.code(), Code::PermissionDenied, "got {err:?}");

    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "@bob:x".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .expect("an unrestricted caller can create the branch on bob's behalf");

    let err = c
        .update_branch(authed(
            pb::UpdateBranchRequest {
                name: "@bob:x".into(),
            },
            ALICE_AGENT_TOKEN,
        ))
        .await
        .expect_err("alice's agent must not update a branch owned by bob");
    assert_eq!(err.code(), Code::PermissionDenied, "got {err:?}");
}

/// MergeBranch on an owned branch is still gated by the merging principal's
/// own namespace scope: owning the branch only clears the delegation gate, it
/// does not widen what the agent is allowed to write. The refusal names the
/// first out-of-scope namespace, and nothing in the diff lands -- not even
/// the half that was in scope.
#[tokio::test(flavor = "multi_thread")]
async fn an_owners_agent_merge_is_refused_for_an_out_of_scope_namespace() {
    let mut c = client().await;

    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "@alice:mixed".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .expect("an unrestricted caller can create the branch on alice's behalf");

    // Staged by an unrestricted caller, not by alice's agent itself: the
    // agent's own branch-scoped PUT is already scope-checked at write time
    // (see `namespace_scope.rs::scope_applies_on_a_branch_too`), so this is
    // the only way to get an out-of-scope entity onto the branch at all.
    c.put_entity(authed_on_branch(
        put_req(event("alice-ns", "in-scope")),
        ADMIN_TOKEN,
        "@alice:mixed",
    ))
    .await
    .unwrap();
    c.put_entity(authed_on_branch(
        put_req(event("outside-scope", "smuggled")),
        ADMIN_TOKEN,
        "@alice:mixed",
    ))
    .await
    .unwrap();

    let err = c
        .merge_branch(authed(
            pb::MergeBranchRequest {
                operation_id: String::new(),
                name: "@alice:mixed".into(),
                ..Default::default()
            },
            ALICE_AGENT_TOKEN,
        ))
        .await
        .expect_err("a diff touching a namespace outside the agent's scope must be refused");
    assert_eq!(err.code(), Code::PermissionDenied, "got {err:?}");
    assert!(
        err.message().contains("outside-scope"),
        "the refusal must name the first out-of-scope namespace, got {err:?}",
    );

    let landed = c
        .get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "outside-scope".into(),
                slug: "smuggled".into(),
            },
            ADMIN_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        landed.entity.is_none(),
        "a refused merge must not land any part of the diff, got {landed:?}",
    );

    let in_scope = c
        .get_latest_version(authed(
            pb::GetLatestVersionRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "alice-ns".into(),
                slug: "in-scope".into(),
            },
            ADMIN_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(
        in_scope.entity.is_none(),
        "a refused merge must not land any part of the diff, including the in-scope half, got {in_scope:?}",
    );
}

/// A user principal (not an agent) gets no special treatment: a classic,
/// unowned branch name carries no owner for anyone to be delegated against,
/// so item B's new allowance for owned branches does not reach it.
#[tokio::test(flavor = "multi_thread")]
async fn a_user_principal_stays_refused_on_an_unowned_branch() {
    let mut c = client().await;

    let err = c
        .create_branch(authed(
            pb::CreateBranchRequest {
                name: "unowned".into(),
                doc: String::new(),
            },
            ACME_TOKEN,
        ))
        .await
        .expect_err("a classic branch name carries no owner, so a bound user stays refused");
    assert_eq!(err.code(), Code::PermissionDenied, "got {err:?}");
}
