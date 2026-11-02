//! A dry run has no persistent effect, and in particular does not claim the
//! namespace it validates against.
//!
//! Clearing authorization on a write claims an unregistered namespace for the
//! caller's owner (claim-on-first-write). That claim is a registry row, so a
//! dry run that went through the same gate listed an empty namespace and
//! handed its bare name to whoever validated against it first.
//!
//! Each test drives one dry-run shape into a namespace that was never
//! created and then asserts, from the outside, that the namespace listing,
//! the change feed and the changeset log are unchanged, and that ownership
//! was not taken: a different principal's real write still claims the name
//! as if no dry run had happened. Authorization is asserted to still run on
//! dry runs, so a dry run is refused whenever the real write would be.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::{metadata::MetadataValue, transport::Channel, Code};
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
};

/// Writer bound to owner `acme`.
const ACME_TOKEN: &str = "dry-acme-token";
/// Writer bound to owner `beta`.
const BETA_TOKEN: &str = "dry-beta-token";
/// Writer with no `parent` (the pre-ownership shape), and so the one that
/// sees every owner's namespaces.
const UNBOUND_TOKEN: &str = "dry-unbound-token";
/// Admin with no `parent`. `DeleteByQuery` is Admin-only.
const ADMIN_TOKEN: &str = "dry-admin-token";
/// Writer with no `parent` and an allow-list of `fenced-*`.
const FENCED_TOKEN: &str = "dry-fenced-token";

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

[principals.fenced-writer]
role = "writer"
tokens = ["{FENCED_TOKEN}"]
namespaces = ["fenced-*"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// One server per test, over its own isolated store.
async fn client() -> EventModelServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    let grpc = EventModelServiceServer::new(svc);

    let auth = BearerAuth::new(Arc::new(make_registry()));
    let data_plane = tonic::service::interceptor::InterceptedService::new(
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

fn put_req(entity: pb::Entity, validate_only: bool) -> pb::PutEntityRequest {
    pb::PutEntityRequest {
        operation_id: String::new(),
        entity: Some(entity),
        create_only: false,
        if_match: String::new(),
        validate_only,
        force: false,
    }
}

fn batch_put(ns: &str, slug: &str, validate_only: bool) -> pb::BatchMutateRequest {
    pb::BatchMutateRequest {
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(put_req(
                event(ns, slug),
                false,
            ))),
        }],
        validate_only,
        operation_id: String::new(),
    }
}

fn delete_req(
    ns: &str,
    slug: &str,
    mode: pb::delete_entity_request::Mode,
) -> pb::DeleteEntityRequest {
    pb::DeleteEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(pb::Id {
            namespace: ns.into(),
            slug: slug.into(),
            version: 1,
        }),
        mode: mode as i32,
        if_match: String::new(),
        operation_id: String::new(),
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

/// Everything a dry run must leave alone, as seen by the principal that
/// sees every owner.
#[derive(Debug, PartialEq, Eq)]
struct Observed {
    namespaces: Vec<(String, String, String, u32)>,
    change_events: Vec<(String, String)>,
    changesets: Vec<String>,
}

async fn observe(c: &mut EventModelServiceClient<Channel>) -> Observed {
    let mut namespaces: Vec<_> = c
        .list_namespaces(authed(pb::ListNamespacesRequest {}, UNBOUND_TOKEN))
        .await
        .unwrap()
        .into_inner()
        .namespaces
        .into_iter()
        .map(|n| (n.name, n.id, n.parent, n.entity_count))
        .collect();
    namespaces.sort();

    let change_events = c
        .list_changes(authed(
            pb::ListChangesRequest {
                page_size: 500,
                ..Default::default()
            },
            UNBOUND_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .events
        .into_iter()
        .map(|e| {
            let id = e.entity.and_then(|r| r.id).unwrap_or_default();
            (id.namespace, id.slug)
        })
        .collect();

    let changesets = c
        .list_changesets(authed(
            pb::ListChangesetsRequest {
                page_size: 500,
                ..Default::default()
            },
            UNBOUND_TOKEN,
        ))
        .await
        .unwrap()
        .into_inner()
        .changesets
        .into_iter()
        .map(|cs| cs.id)
        .collect();

    Observed {
        namespaces,
        change_events,
        changesets,
    }
}

/// After a dry run into `ns`, the name must still be free: the next real
/// write by a different principal claims it for that principal, and the
/// principal that dry-ran is the one refused afterwards, exactly as if the
/// dry run had never happened.
async fn assert_ownership_untouched(c: &mut EventModelServiceClient<Channel>, ns: &str) {
    c.put_entity(authed(put_req(event(ns, "real"), false), BETA_TOKEN))
        .await
        .expect("a later real write by another principal must claim the name");

    assert!(
        observe(c)
            .await
            .namespaces
            .iter()
            .any(|(name, _, parent, _)| name == ns && parent == "beta"),
        "the namespace must belong to the principal that really wrote it",
    );

    let err = c
        .put_entity(authed(put_req(event(ns, "late"), false), ACME_TOKEN))
        .await
        .expect_err("the principal that only dry-ran owns nothing and must be fenced out");
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_put_does_not_claim_the_namespace() {
    let mut c = client().await;
    let before = observe(&mut c).await;

    let resp = c
        .put_entity(authed(put_req(event("dry-put", "probe"), true), ACME_TOKEN))
        .await
        .expect("a dry run into a fresh namespace is allowed");
    assert!(
        resp.into_inner().etag.is_empty(),
        "a dry run persists nothing"
    );

    assert_eq!(observe(&mut c).await, before);
    assert_ownership_untouched(&mut c, "dry-put").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_batch_does_not_claim_the_namespace() {
    let mut c = client().await;
    let before = observe(&mut c).await;

    // This is the RPC `trogon-atlas apply --dry-run` sends.
    c.batch_mutate(authed(batch_put("dry-batch", "probe", true), ACME_TOKEN))
        .await
        .expect("a dry run into a fresh namespace is allowed");

    assert_eq!(observe(&mut c).await, before);
    assert_ownership_untouched(&mut c, "dry-batch").await;
}

/// One dry-run op is enough to make the whole batch a dry run.
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_with_a_dry_run_op_does_not_claim_the_namespace() {
    let mut c = client().await;
    let before = observe(&mut c).await;

    let request = pb::BatchMutateRequest {
        ops: vec![pb::BatchMutateOp {
            op: Some(pb::batch_mutate_op::Op::Put(put_req(
                event("dry-op", "probe"),
                true,
            ))),
        }],
        validate_only: false,
        operation_id: String::new(),
    };
    c.batch_mutate(authed(request, ACME_TOKEN)).await.unwrap();

    assert_eq!(observe(&mut c).await, before);
    assert_ownership_untouched(&mut c, "dry-op").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_delete_does_not_claim_the_namespace() {
    let mut c = client().await;
    let before = observe(&mut c).await;

    // The entity does not exist, so the call may report that; what matters
    // is that it left nothing behind.
    let _ = c
        .delete_entity(authed(
            delete_req(
                "dry-delete",
                "probe",
                pb::delete_entity_request::Mode::DryRun,
            ),
            ACME_TOKEN,
        ))
        .await;

    assert_eq!(observe(&mut c).await, before);
    assert_ownership_untouched(&mut c, "dry-delete").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_delete_by_query_does_not_claim_the_namespace() {
    let mut c = client().await;
    let before = observe(&mut c).await;

    let _ = c
        .delete_by_query(authed(
            pb::DeleteByQueryRequest {
                namespace: "dry-query".into(),
                kind: pb::EntityKind::Event as i32,
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                max_deletes: 10,
                ..Default::default()
            },
            ADMIN_TOKEN,
        ))
        .await;

    assert_eq!(observe(&mut c).await, before);
    assert_ownership_untouched(&mut c, "dry-query").await;
}

/// An unbound principal is never refused, but it still claims on a real
/// write; a dry run must not.
#[tokio::test(flavor = "multi_thread")]
async fn an_unbound_principals_dry_run_does_not_claim_either() {
    let mut c = client().await;
    let before = observe(&mut c).await;

    c.batch_mutate(authed(
        batch_put("dry-unbound", "probe", true),
        UNBOUND_TOKEN,
    ))
    .await
    .unwrap();

    assert_eq!(observe(&mut c).await, before);
    assert_ownership_untouched(&mut c, "dry-unbound").await;
}

/// Skipping the claim must not weaken the check: a dry run is refused
/// whenever the real write would be.
#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_is_refused_where_the_real_write_is_refused() {
    let mut c = client().await;

    c.put_entity(authed(put_req(event("taken", "mine"), false), ACME_TOKEN))
        .await
        .unwrap();
    let before = observe(&mut c).await;

    let err = c
        .put_entity(authed(put_req(event("taken", "yours"), true), BETA_TOKEN))
        .await
        .expect_err("a dry run into another owner's namespace is denied");
    assert_eq!(err.code(), Code::PermissionDenied);

    let err = c
        .batch_mutate(authed(batch_put("taken", "yours", true), BETA_TOKEN))
        .await
        .expect_err("a dry-run batch into another owner's namespace is denied");
    assert_eq!(err.code(), Code::PermissionDenied);

    let err = c
        .batch_mutate(authed(batch_put("elsewhere", "probe", true), FENCED_TOKEN))
        .await
        .expect_err("a dry run outside the principal's allow-list is denied");
    assert_eq!(err.code(), Code::PermissionDenied);

    let err = c
        .delete_entity(authed(
            delete_req("taken", "mine", pb::delete_entity_request::Mode::DryRun),
            BETA_TOKEN,
        ))
        .await
        .expect_err("a dry-run delete in another owner's namespace is denied");
    assert_eq!(err.code(), Code::PermissionDenied);

    assert_eq!(observe(&mut c).await, before);
}

/// The control: a real write still claims, so the dry-run fix did not turn
/// claim-on-first-write off.
#[tokio::test(flavor = "multi_thread")]
async fn a_real_write_still_claims_the_namespace() {
    let mut c = client().await;

    c.batch_mutate(authed(batch_put("real-batch", "probe", false), ACME_TOKEN))
        .await
        .unwrap();

    assert!(observe(&mut c)
        .await
        .namespaces
        .iter()
        .any(|(name, _, parent, _)| name == "real-batch" && parent == "acme"));
}
