//! In-process gRPC integration tests for the namespace axis of
//! authorization (G16): a principal's write allow-list, and baseline
//! protection scoped to named namespaces instead of all of them. Same
//! pattern as `tests/protect_baseline.rs`: a real tonic server with the real
//! `BearerAuth` + `AuthzLayer` stack over an ephemeral JetStream store.
//!
//! Covers:
//! - A scoped writer may write inside its namespaces and is denied outside
//!   them, on baseline and on a branch alike.
//! - Scope applies to every write shape: `PutEntity`, `DeleteEntity`,
//!   `BatchMutate` (all-or-nothing on the union of its ops), `MergeBranch`,
//!   and the two RPCs that select their own victims.
//! - An unscoped principal is unaffected, which is what makes the field
//!   opt-in.
//! - Per-namespace baseline protection: a protected namespace behaves as it
//!   did under the global flag, an unprotected one keeps accepting direct
//!   baseline writes.

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
    scope::BaselineProtection,
    service::EventModelServiceImpl,
};

/// Writes `orders` and anything under `shop-`, nothing else.
const SCOPED_TOKEN: &str = "ns-scoped-token";
/// Same role, no allow-list: the pre-G16 shape.
const UNSCOPED_TOKEN: &str = "ns-unscoped-token";
/// Admin, no allow-list.
const ADMIN_TOKEN: &str = "ns-admin-token";
/// Admin that is nevertheless scoped, to prove scope is not a role bypass.
const SCOPED_ADMIN_TOKEN: &str = "ns-scoped-admin-token";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.scoped-writer]
role = "writer"
tokens = ["{SCOPED_TOKEN}"]
namespaces = ["orders", "shop-*"]

[principals.unscoped-writer]
role = "writer"
tokens = ["{UNSCOPED_TOKEN}"]

[principals.admin-svc]
role = "admin"
tokens = ["{ADMIN_TOKEN}"]

[principals.scoped-admin]
role = "admin"
tokens = ["{SCOPED_ADMIN_TOKEN}"]
namespaces = ["orders"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

async fn start_server(protection: BaselineProtection) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_baseline_protection(protection);
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

    format!("http://{addr}")
}

async fn client(protection: BaselineProtection) -> EventModelServiceClient<Channel> {
    let endpoint = start_server(protection).await;
    EventModelServiceClient::connect(endpoint).await.unwrap()
}

async fn open_client() -> EventModelServiceClient<Channel> {
    client(BaselineProtection::Off).await
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

fn authed_on_branch<T>(body: T, token: &str, branch: &str) -> tonic::Request<T> {
    let mut req = authed(body, token);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).unwrap(),
    );
    req
}

// ---------------------------------------------------------------------------
// The allow-list itself
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_scoped_writer_may_write_inside_its_namespaces() {
    let mut c = open_client().await;

    c.put_entity(authed(put_req(event("orders", "placed")), SCOPED_TOKEN))
        .await
        .expect("an exact match must be admitted");
    c.put_entity(authed(
        put_req(event("shop-checkout", "started")),
        SCOPED_TOKEN,
    ))
    .await
    .expect("a wildcard match must be admitted");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scoped_writer_is_denied_outside_its_namespaces() {
    let mut c = open_client().await;

    let err = c
        .put_entity(authed(put_req(event("billing", "invoiced")), SCOPED_TOKEN))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert!(
        err.message().contains("billing"),
        "the message must name the namespace that was refused: {err:?}"
    );
}

/// The prefix is `shop-`, so the bare namespace `shop` is outside it. Getting
/// this wrong would silently widen every wildcard grant by one namespace.
#[tokio::test(flavor = "multi_thread")]
async fn a_wildcard_does_not_admit_the_bare_prefix() {
    let mut c = open_client().await;

    let err = c
        .put_entity(authed(put_req(event("shop", "anything")), SCOPED_TOKEN))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

/// The opt-in property: a principal with no `namespaces` key is exactly what
/// it was before the field existed.
#[tokio::test(flavor = "multi_thread")]
async fn an_unscoped_writer_writes_anywhere() {
    let mut c = open_client().await;

    c.put_entity(authed(
        put_req(event("billing", "invoiced")),
        UNSCOPED_TOKEN,
    ))
    .await
    .expect("an empty allow-list must admit everything");
}

/// Scope is a second axis, not a weaker role. Admin buys the RPC, not the
/// namespace.
#[tokio::test(flavor = "multi_thread")]
async fn admin_does_not_override_its_own_scope() {
    let mut c = open_client().await;

    let err = c
        .put_entity(authed(
            put_req(event("billing", "invoiced")),
            SCOPED_ADMIN_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

/// A branch is a staging area for baseline, so allowing an out-of-scope
/// branch write would only defer the violation to merge time.
#[tokio::test(flavor = "multi_thread")]
async fn scope_applies_on_a_branch_too() {
    let mut c = open_client().await;
    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "feat".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .unwrap();

    c.put_entity(authed_on_branch(
        put_req(event("orders", "placed")),
        SCOPED_TOKEN,
        "feat",
    ))
    .await
    .expect("in-scope branch writes are ordinary writes");

    let err = c
        .put_entity(authed_on_branch(
            put_req(event("billing", "invoiced")),
            SCOPED_TOKEN,
            "feat",
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_is_scoped_like_a_write() {
    let mut c = open_client().await;
    c.put_entity(authed(put_req(event("billing", "invoiced")), ADMIN_TOKEN))
        .await
        .unwrap();

    let err = c
        .delete_entity(authed(
            pb::DeleteEntityRequest {
                operation_id: String::new(),
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: "billing".into(),
                    slug: "invoiced".into(),
                    version: 1,
                }),
                mode: pb::delete_entity_request::Mode::Force as i32,
                if_match: String::new(),
            },
            SCOPED_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

/// A batch is one atomic act, so it is authorized as one: the whole request
/// is refused when any op falls outside the scope. Authorizing per-op would
/// mean partially applying a batch that the caller was not allowed to make.
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_is_refused_whole_when_one_op_is_out_of_scope() {
    let mut c = open_client().await;

    let ops = vec![
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(event("orders", "batched")),
                ..Default::default()
            })),
        },
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(event("billing", "batched")),
                ..Default::default()
            })),
        },
    ];
    let err = c
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                operation_id: String::new(),
                ops,
                validate_only: false,
            },
            SCOPED_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    // The in-scope op must not have landed: the refusal is not partial.
    let found = c
        .get_entity(authed(
            pb::GetEntityRequest {
                kind: pb::EntityKind::Event as i32,
                id: Some(pb::Id {
                    namespace: "orders".into(),
                    slug: "batched".into(),
                    version: 1,
                }),
            },
            ADMIN_TOKEN,
        ))
        .await;
    assert_eq!(
        found.unwrap_err().code(),
        Code::NotFound,
        "a refused batch must write nothing at all"
    );
}

/// Merging is how a branch write reaches baseline. If scope were not checked
/// here, a scoped principal could write anywhere by branching first, and the
/// branch-time check would be theatre.
#[tokio::test(flavor = "multi_thread")]
async fn merge_cannot_launder_an_out_of_scope_write() {
    let mut c = open_client().await;
    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "smuggle".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .unwrap();
    // The unscoped writer stages a namespace the scoped one may not touch.
    c.put_entity(authed_on_branch(
        put_req(event("billing", "smuggled")),
        UNSCOPED_TOKEN,
        "smuggle",
    ))
    .await
    .unwrap();

    let err = c
        .merge_branch(authed(
            pb::MergeBranchRequest {
                operation_id: String::new(),
                name: "smuggle".into(),
                dry_run: false,
                keep_branch: true,
                auto_merge: false,
            },
            SCOPED_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    // The same merge by an unscoped principal still works, so the branch
    // itself was never the problem.
    c.merge_branch(authed(
        pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "smuggle".into(),
            dry_run: false,
            keep_branch: true,
            auto_merge: false,
        },
        UNSCOPED_TOKEN,
    ))
    .await
    .expect("an unscoped principal may land the same merge");
}

/// `RetargetReferences` rewrites whatever referenced an entity, anywhere, and
/// the set is only known after a scan. There is no allow-list that can bound
/// it, so a scoped principal is refused outright rather than being allowed a
/// write whose reach nobody checked.
#[tokio::test(flavor = "multi_thread")]
async fn a_scoped_principal_cannot_retarget_references() {
    let mut c = open_client().await;

    let err = c
        .retarget_references(authed(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(pb::Id {
                        namespace: "orders".into(),
                        slug: "old".into(),
                        version: 1,
                    }),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(pb::Id {
                        namespace: "orders".into(),
                        slug: "new".into(),
                        version: 1,
                    }),
                }),
                dry_run: true,
                filter_kinds: vec![],
            },
            SCOPED_ADMIN_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert!(
        err.message().contains("no allow-list can bound"),
        "the refusal must explain why, not just deny: {err:?}"
    );
}

/// `DeleteByQuery` is the one self-selecting RPC that can be bounded up
/// front: with a namespace filter, nothing outside it can match.
#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_is_bounded_by_its_namespace_filter() {
    let mut c = open_client().await;
    c.put_entity(authed(put_req(event("orders", "doomed")), ADMIN_TOKEN))
        .await
        .unwrap();

    let deleted = c
        .delete_by_query(authed(
            pb::DeleteByQueryRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "orders".into(),
                max_deletes: 10,
                ..Default::default()
            },
            SCOPED_ADMIN_TOKEN,
        ))
        .await
        .expect("a namespace filter inside the scope is bounded")
        .into_inner();
    assert_eq!(deleted.deleted.len(), 1);

    let err = c
        .delete_by_query(authed(
            pb::DeleteByQueryRequest {
                kind: pb::EntityKind::Event as i32,
                namespace: "billing".into(),
                max_deletes: 10,
                ..Default::default()
            },
            SCOPED_ADMIN_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    let err = c
        .delete_by_query(authed(
            pb::DeleteByQueryRequest {
                kind: pb::EntityKind::Event as i32,
                max_deletes: 10,
                ..Default::default()
            },
            SCOPED_ADMIN_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::PermissionDenied,
        "without a namespace filter the query selects its own victims"
    );
}

// ---------------------------------------------------------------------------
// Per-namespace baseline protection
// ---------------------------------------------------------------------------

fn protect(namespaces: &[&str]) -> BaselineProtection {
    BaselineProtection::configure(false, namespaces).expect("valid patterns")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_protected_namespace_refuses_a_direct_baseline_write() {
    let mut c = client(protect(&["orders"])).await;

    let err = c
        .put_entity(authed(put_req(event("orders", "placed")), UNSCOPED_TOKEN))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unprotected_namespace_keeps_accepting_direct_baseline_writes() {
    let mut c = client(protect(&["orders"])).await;

    c.put_entity(authed(put_req(event("sandbox", "scratch")), UNSCOPED_TOKEN))
        .await
        .expect("protection must not leak into namespaces it does not name");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_write_into_a_protected_namespace_is_still_allowed() {
    let mut c = client(protect(&["orders"])).await;
    c.create_branch(authed(
        pb::CreateBranchRequest {
            name: "review".into(),
            doc: String::new(),
        },
        ADMIN_TOKEN,
    ))
    .await
    .unwrap();

    c.put_entity(authed_on_branch(
        put_req(event("orders", "placed")),
        UNSCOPED_TOKEN,
        "review",
    ))
    .await
    .expect("branching is exactly what protection asks for");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_touching_one_protected_namespace_is_protected() {
    let mut c = client(protect(&["orders"])).await;

    let ops = vec![
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(event("sandbox", "scratch")),
                ..Default::default()
            })),
        },
        pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(event("orders", "placed")),
                ..Default::default()
            })),
        },
    ];
    let err = c
        .batch_mutate(authed(
            pb::BatchMutateRequest {
                operation_id: String::new(),
                ops,
                validate_only: false,
            },
            UNSCOPED_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::FailedPrecondition,
        "a batch is one act: touching a protected namespace protects all of it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admin_still_bypasses_per_namespace_protection() {
    let mut c = client(protect(&["orders"])).await;

    c.put_entity(authed(put_req(event("orders", "placed")), ADMIN_TOKEN))
        .await
        .expect("the admin escape hatch is unchanged by scoping protection");
}
