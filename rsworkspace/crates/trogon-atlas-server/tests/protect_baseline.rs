//! In-process gRPC integration tests for baseline write protection
//! (`--protect-baseline` / `TROGON_ATLAS_PROTECT_BASELINE`, see
//! docs/explanation/branching.md, "Baseline protection"). Same pattern as
//! `tests/branching.rs` and `tests/auth_stack.rs`: a real tonic server over
//! an ephemeral JetStream store, exercised through a real gRPC client.
//!
//! Covers:
//! - Protection ON, no auth stack: baseline `PutEntity` / `DeleteEntity` /
//!   `BatchMutate` / `RetargetReferences` / `DeleteByQuery` rejected with
//!   `FAILED_PRECONDITION` and the exact guidance message; the same ops with
//!   a branch header succeed; a full branch -> apply -> merge lifecycle still
//!   lands on baseline; branch lifecycle RPCs are never blocked.
//! - Protection ON + auth stack: writer token rejected on baseline writes,
//!   admin token allowed; reader token still denied (RBAC runs first).
//! - Protection ON + `--insecure-allow-anonymous`: anonymous writers are
//!   still rejected on baseline writes (no admin bypass for anonymous mode).
//! - Protection OFF (default): behavior byte-identical to today.

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

const BASELINE_PROTECTED_MESSAGE: &str =
    "baseline is protected: make changes on a branch (x-trogon-atlas-branch metadata / \
     trogon-atlas --branch) and merge them";

async fn start_server(protect_baseline: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_protect_baseline(protect_baseline);
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

fn put_req(entity: pb::Entity, force: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force,
    }
}

/// Attach the `x-trogon-atlas-branch` header to a request.
fn with_branch<T>(body: T, branch: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "x-trogon-atlas-branch",
        MetadataValue::try_from(branch).expect("valid header value"),
    );
    req
}

// ---------------------------------------------------------------------------
// Protection ON, no auth stack
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_put_entity_rejected_without_branch() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    let err = c
        .put_entity(put_req(event("pb-put", "a", 1, "A"), false))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_delete_entity_rejected_without_branch() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    let err = c
        .delete_entity(pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("pb-del", "a", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_batch_mutate_rejected_without_branch() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    let put_op = |e: pb::Entity| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_req(e, false))),
    };
    let err = c
        .batch_mutate(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![put_op(event("pb-bm", "a", 1, "A"))],
            validate_only: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_retarget_references_rejected() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    // Without a branch header retarget_references rewrites baseline
    // directly, so under protection it is rejected unless the caller is
    // Admin. With a header it is branch-scoped, which the next test covers.
    let err = c
        .retarget_references(pb::RetargetReferencesRequest {
            from: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("pb-rt", "old", 1)),
            }),
            to: Some(pb::EntityRef {
                kind: pb::EntityKind::Event as i32,
                id: Some(id("pb-rt", "new", 1)),
            }),
            dry_run: true,
            filter_kinds: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_retarget_references_succeeds_with_branch_header() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-rt-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    // Under protection a branch is the only place these can be authored,
    // which is exactly the situation a staged version migration is in.
    c.put_entity(with_branch(
        put_req(event("pb-rt-br", "thing", 1, "V1"), false),
        "pb-rt-branch",
    ))
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("pb-rt-br", "thing", 2, "V2"), false),
        "pb-rt-branch",
    ))
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(
            pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
                    id: Some(id("pb-rt-br", "slice", 1)),
                    title: "slice".into(),
                    emitted_events: vec![pb::EventEdge {
                        event: Some(pb::EventRef {
                            id: Some(id("pb-rt-br", "thing", 1)),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
            },
            false,
        ),
        "pb-rt-branch",
    ))
    .await
    .unwrap();

    // A branch header does not bypass protection, it satisfies it: the
    // rewrite lands as a branch delta and reaches baseline only on merge.
    let resp = c
        .retarget_references(with_branch(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("pb-rt-br", "thing", 1)),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("pb-rt-br", "thing", 2)),
                }),
                dry_run: false,
                filter_kinds: vec![],
            },
            "pb-rt-branch",
        ))
        .await
        .expect("retarget_references with a branch header must succeed under protection")
        .into_inner();
    assert_eq!(resp.rewritten, 1, "results: {:?}", resp.results);
    assert_eq!(resp.failed, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_delete_by_query_rejected_without_branch() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    let err = c
        .delete_by_query(pb::DeleteByQueryRequest {
            project: String::new(),
            namespace: "pb-dbq".into(),
            kind: 0,
            slug: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
            max_deletes: 10,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_delete_by_query_succeeds_with_branch_header() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-dbq-branch".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.put_entity(with_branch(
        put_req(event("pb-dbq-br", "a", 1, "A"), false),
        "pb-dbq-branch",
    ))
    .await
    .unwrap();

    let resp = c
        .delete_by_query(with_branch(
            pb::DeleteByQueryRequest {
                project: String::new(),
                namespace: "pb-dbq-br".into(),
                kind: 0,
                slug: String::new(),
                mode: pb::delete_entity_request::Mode::Force as i32,
                max_deletes: 10,
            },
            "pb-dbq-branch",
        ))
        .await
        .expect("delete_by_query with a branch header must succeed under protection")
        .into_inner();
    assert_eq!(resp.deleted_count, 1);
}

// ---------------------------------------------------------------------------
// Protection ON, WITH a branch header: same ops succeed
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_put_entity_succeeds_with_branch_header() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-branch-put".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(event("pb-put-branch", "a", 1, "A"), false),
        "pb-branch-put",
    ))
    .await
    .expect("put_entity with a branch header must succeed under protection");
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_delete_entity_succeeds_with_branch_header() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-branch-del".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(event("pb-del-branch", "a", 1, "A"), false),
        "pb-branch-del",
    ))
    .await
    .unwrap();

    c.delete_entity(with_branch(
        pb::DeleteEntityRequest {
            operation_id: String::new(),
            kind: pb::EntityKind::Event as i32,
            id: Some(id("pb-del-branch", "a", 1)),
            if_match: String::new(),
            mode: pb::delete_entity_request::Mode::Force as i32,
        },
        "pb-branch-del",
    ))
    .await
    .expect("delete_entity with a branch header must succeed under protection");
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_batch_mutate_succeeds_with_branch_header() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-branch-bm".into(),
        doc: String::new(),
    })
    .await
    .unwrap();

    let put_op = |e: pb::Entity| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_req(e, false))),
    };
    let resp = c
        .batch_mutate(with_branch(
            pb::BatchMutateRequest {
                operation_id: String::new(),
                ops: vec![put_op(event("pb-bm-branch", "a", 1, "A"))],
                validate_only: false,
            },
            "pb-branch-bm",
        ))
        .await
        .expect("batch_mutate with a branch header must succeed under protection")
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
}

// ---------------------------------------------------------------------------
// Protection ON: full branch -> apply -> merge lifecycle still lands
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_branch_merge_lifecycle_lands_on_baseline() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-merge-branch".into(),
        doc: "lifecycle test".into(),
    })
    .await
    .unwrap();

    c.put_entity(with_branch(
        put_req(event("pb-merge", "a", 1, "A on branch"), false),
        "pb-merge-branch",
    ))
    .await
    .unwrap();

    let merge_resp = c
        .merge_branch(pb::MergeBranchRequest {
            operation_id: String::new(),
            name: "pb-merge-branch".into(),
            dry_run: false,
            keep_branch: false,
            auto_merge: false,
        })
        .await
        .expect("MergeBranch is how baseline changes under protection")
        .into_inner();
    assert_eq!(
        merge_resp.status,
        pb::merge_branch_response::Status::Applied as i32
    );

    // The merge itself is not a mutating RPC subject to protection, and it
    // must have landed the change onto baseline (readable without a branch
    // header).
    let got = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("pb-merge", "a", 1)),
        })
        .await
        .expect("merged entity must be visible on baseline")
        .into_inner();
    assert_eq!(
        got.entity
            .unwrap()
            .kind
            .and_then(|k| match k {
                pb::entity::Kind::Event(e) => Some(e.title),
                _ => None,
            })
            .unwrap(),
        "A on branch"
    );
}

// ---------------------------------------------------------------------------
// Protection ON: branch lifecycle RPCs are never blocked
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_branch_lifecycle_rpcs_all_work() {
    let endpoint = start_server(true).await;
    let mut c = client(&endpoint).await;

    c.create_branch(pb::CreateBranchRequest {
        name: "pb-lifecycle".into(),
        doc: String::new(),
    })
    .await
    .expect("CreateBranch must never be blocked by baseline protection");

    c.list_branches(pb::ListBranchesRequest {})
        .await
        .expect("ListBranches must never be blocked by baseline protection");

    c.put_entity(with_branch(
        put_req(event("pb-lifecycle-ns", "a", 1, "A"), false),
        "pb-lifecycle",
    ))
    .await
    .unwrap();

    c.diff_branch(pb::DiffBranchRequest {
        name: "pb-lifecycle".into(),
    })
    .await
    .expect("DiffBranch must never be blocked by baseline protection");

    c.update_branch(pb::UpdateBranchRequest {
        name: "pb-lifecycle".into(),
    })
    .await
    .expect("UpdateBranch must never be blocked by baseline protection");

    c.merge_branch(pb::MergeBranchRequest {
        operation_id: String::new(),
        name: "pb-lifecycle".into(),
        dry_run: false,
        keep_branch: false,
        auto_merge: false,
    })
    .await
    .expect("MergeBranch must never be blocked by baseline protection");

    // Branch is gone after a non-keep_branch merge; exercise DeleteBranch on
    // a fresh one instead.
    c.create_branch(pb::CreateBranchRequest {
        name: "pb-lifecycle-2".into(),
        doc: String::new(),
    })
    .await
    .unwrap();
    c.delete_branch(pb::DeleteBranchRequest {
        name: "pb-lifecycle-2".into(),
    })
    .await
    .expect("DeleteBranch must never be blocked by baseline protection");
}

// ---------------------------------------------------------------------------
// Protection ON + auth stack
// ---------------------------------------------------------------------------

fn with_token<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("valid header value"),
    );
    req
}

const READER_TOKEN: &str = "pb-reader-token";
const WRITER_TOKEN: &str = "pb-writer-token";
const ADMIN_TOKEN: &str = "pb-admin-token";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.reader-svc]
role = "reader"
tokens = ["{READER_TOKEN}"]

[principals.writer-svc]
role = "writer"
tokens = ["{WRITER_TOKEN}"]

[principals.admin-svc]
role = "admin"
tokens = ["{ADMIN_TOKEN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

async fn start_auth_server(protect_baseline: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc_impl = EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_protect_baseline(protect_baseline);
    let grpc_service = EventModelServiceServer::new(svc_impl);

    let registry = Arc::new(make_registry());
    let auth = BearerAuth::new(registry.clone());

    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(grpc_service),
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

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_writer_token_rejected_on_baseline_write() {
    let addr = start_auth_server(true).await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    let err = c
        .put_entity(with_token(
            put_req(event("pb-auth-writer", "a", 1, "A"), false),
            WRITER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_admin_token_allowed_on_baseline_write() {
    let addr = start_auth_server(true).await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    c.put_entity(with_token(
        put_req(event("pb-auth-admin", "a", 1, "A"), false),
        ADMIN_TOKEN,
    ))
    .await
    .expect("admin token must bypass baseline protection");
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_reader_token_denied_on_baseline_write() {
    let addr = start_auth_server(true).await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    // Reader is denied by ordinary RBAC (Reader < Writer) before baseline
    // protection is even consulted, so this is PERMISSION_DENIED, not
    // FAILED_PRECONDITION.
    let err = c
        .put_entity(with_token(
            put_req(event("pb-auth-reader", "a", 1, "A"), false),
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_admin_token_allowed_on_retarget_references() {
    let addr = start_auth_server(true).await;
    let mut c = EventModelServiceClient::connect(addr).await.unwrap();

    // retarget_references has no branch-scoped mode; it always requires
    // Admin under protection. dry_run so no existing "to" entity is needed.
    let result = c
        .retarget_references(with_token(
            pb::RetargetReferencesRequest {
                from: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("pb-auth-rt", "old", 1)),
                }),
                to: Some(pb::EntityRef {
                    kind: pb::EntityKind::Event as i32,
                    id: Some(id("pb-auth-rt", "new", 1)),
                }),
                dry_run: true,
                filter_kinds: vec![],
            },
            ADMIN_TOKEN,
        ))
        .await;
    // The admin bypass must be granted; any remaining error must come from
    // downstream validation (e.g. `to` not existing), never from the
    // baseline-protection guard.
    if let Err(err) = &result {
        assert_ne!(
            err.code(),
            Code::FailedPrecondition,
            "admin token must not be rejected by baseline protection: {err:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Protection ON + `--insecure-allow-anonymous`: no admin bypass for anonymous
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn protected_baseline_anonymous_writer_still_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc_impl = EventModelServiceImpl::try_new(Arc::new(store))
        .unwrap()
        .with_protect_baseline(true);
    let grpc_service = EventModelServiceServer::new(svc_impl);

    let registry = Arc::new(TokenRegistry::anonymous());
    let auth = BearerAuth::new(registry.clone());
    let data_plane = InterceptedService::new(
        tower::ServiceBuilder::new()
            .layer(AuthzLayer)
            .service(grpc_service),
        auth,
    );
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(data_plane)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    let mut c = EventModelServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap();

    // Anonymous mode grants Role::Admin for ordinary RBAC (so the request
    // passes AuthzLayer), but the synthetic anonymous principal must NOT
    // satisfy the baseline-protection escape hatch: a dev stack that
    // combines `--insecure-allow-anonymous` with `--protect-baseline`
    // genuinely enforces branching, with no bypass for anonymous callers.
    let err = c
        .put_entity(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(event("pb-anon", "a", 1, "A")),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(err.message(), BASELINE_PROTECTED_MESSAGE);

    // The same anonymous caller with a branch header still succeeds.
    c.create_branch(pb::CreateBranchRequest {
        name: "pb-anon-branch".into(),
        doc: String::new(),
    })
    .await
    .expect("anonymous caller may still create a branch");

    c.put_entity(with_branch(
        pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(event("pb-anon-branch-ns", "a", 1, "A")),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        },
        "pb-anon-branch",
    ))
    .await
    .expect("anonymous caller with a branch header must succeed under protection");
}

// ---------------------------------------------------------------------------
// Protection OFF (default): byte-identical to today
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn unprotected_baseline_put_entity_succeeds_without_branch() {
    let endpoint = start_server(false).await;
    let mut c = client(&endpoint).await;

    c.put_entity(put_req(event("pb-off", "a", 1, "A"), false))
        .await
        .expect("protection off must behave exactly as before this feature");
}

#[tokio::test(flavor = "multi_thread")]
async fn unprotected_baseline_batch_mutate_succeeds_without_branch() {
    let endpoint = start_server(false).await;
    let mut c = client(&endpoint).await;

    let put_op = |e: pb::Entity| pb::BatchMutateOp {
        op: Some(pb::batch_mutate_op::Op::Put(put_req(e, false))),
    };
    let resp = c
        .batch_mutate(pb::BatchMutateRequest {
            operation_id: String::new(),
            ops: vec![put_op(event("pb-off-bm", "a", 1, "A"))],
            validate_only: false,
        })
        .await
        .expect("protection off must behave exactly as before this feature")
        .into_inner();
    assert_eq!(
        resp.status,
        pb::batch_mutate_response::Status::Applied as i32
    );
}
