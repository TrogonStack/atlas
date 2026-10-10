//! Integration tests for the composed authentication + authorization stack.
//!
//! These tests exercise the EXACT same service composition as main.rs:
//!
//!   `InterceptedService<BearerAuth>`
//!     wraps `AuthzService<EventModelServiceServer<Impl>>`
//!
//! A real in-process gRPC server is spawned on a random port so requests
//! travel through the full tonic codec, interceptor, and tower middleware
//! chain. This catches ordering bugs (authn before authz, not vice versa)
//! that unit tests on the interceptor and layer in isolation cannot detect.
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

/// Attach a bearer token to a request.
fn with_token<T>(body: T, token: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(body);
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("valid header value"),
    );
    req
}

const READER_TOKEN: &str = "reader-token-aaaa";
const WRITER_TOKEN: &str = "writer-token-bbbb";
const ADMIN_TOKEN: &str = "admin-token-cccc";
const SCOPED_AGENT_TOKEN: &str = "scoped-agent-token-dddd";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.gateway]
role = "reader"
tokens = ["{READER_TOKEN}"]

[principals.mcp]
role = "writer"
tokens = ["{WRITER_TOKEN}"]

[principals.cli]
role = "admin"
tokens = ["{ADMIN_TOKEN}"]

[principals.delegated-agent]
role = "writer"
kind = "agent"
tokens = ["{SCOPED_AGENT_TOKEN}"]
namespaces = ["orders", "shop-*"]
parent = "acme-org"
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// Spin up a real gRPC server using the same composition as main.rs and return
/// its address. The composition is:
///
///   `InterceptedService<AuthzService<EventModelServiceServer<Impl>>, BearerAuth>`
///
/// This is intentionally written out explicitly here so a future change that
/// inverts the nesting in main.rs will also need to update this test.
async fn start_auth_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc_impl = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc_service = EventModelServiceServer::new(svc_impl);

    let registry = Arc::new(make_registry());
    let auth = BearerAuth::new(registry.clone());

    // Correct nesting: InterceptedService (BearerAuth authn) is OUTERMOST so
    // it runs first and populates extensions before AuthzLayer reads them.
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

async fn make_client(endpoint: &str) -> EventModelServiceClient<Channel> {
    EventModelServiceClient::connect(endpoint.to_owned())
        .await
        .unwrap()
}

fn make_event(ns: &str, slug: &str, version: u64) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: ns.into(),
                slug: slug.into(),
                version,
            }),
            title: format!("{ns}/{slug}@{version}"),
            ..Default::default()
        })),
    }
}

// ---------------------------------------------------------------------------
// Composed-stack regression tests
// ---------------------------------------------------------------------------

/// Reader token on a read RPC (`GetServerInfo`) must succeed.
/// This verifies authn runs before authz so the Principal is in extensions
/// by the time `AuthzLayer` checks it.
#[tokio::test(flavor = "multi_thread")]
async fn reader_token_on_read_rpc_succeeds() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let result = client
        .get_server_info(with_token(pb::GetServerInfoRequest {}, READER_TOKEN))
        .await;
    assert!(
        result.is_ok(),
        "reader token on read RPC should succeed: {result:?}"
    );
}

/// Reader token on `DeleteByQuery` (Admin-only) must return `PERMISSION_DENIED`.
#[tokio::test(flavor = "multi_thread")]
async fn reader_token_on_admin_rpc_returns_permission_denied() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let err = client
        .delete_by_query(with_token(
            pb::DeleteByQueryRequest {
                project: String::new(),
                namespace: "test".into(),
                kind: 0,
                slug: String::new(),
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                max_deletes: 1,
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::PermissionDenied,
        "reader token must be denied on Admin RPC; got {err:?}"
    );
}

/// Missing bearer token must return `UNAUTHENTICATED` (not `PERMISSION_DENIED`).
///
/// This is the key regression guard: if `AuthzLayer` ran before `BearerAuth`,
/// the request would have no `Principal` in extensions. The deny-by-default
/// arm in `AuthzService::call` would fire and return `PERMISSION_DENIED`.
/// The correct behavior is `UNAUTHENTICATED` from `BearerAuth`.
#[tokio::test(flavor = "multi_thread")]
async fn missing_token_returns_unauthenticated_not_permission_denied() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let err = client
        .get_server_info(tonic::Request::new(pb::GetServerInfoRequest {}))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::Unauthenticated,
        "missing token must return UNAUTHENTICATED, not PERMISSION_DENIED; got {err:?}"
    );
}

/// Unknown token returns `UNAUTHENTICATED`, not `PERMISSION_DENIED`.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_token_returns_unauthenticated() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let err = client
        .get_server_info(with_token(
            pb::GetServerInfoRequest {},
            "completely-wrong-token",
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::Unauthenticated,
        "unknown token must return UNAUTHENTICATED; got {err:?}"
    );
}

/// Writer token on `PutEntity` must succeed (Writer >= Writer).
#[tokio::test(flavor = "multi_thread")]
async fn writer_token_on_put_entity_succeeds() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let result = client
        .put_entity(with_token(
            pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("authz-test", "writer-put", 1)),
                create_only: false,
                if_match: String::new(),
                validate_only: false,
                force: false,
            },
            WRITER_TOKEN,
        ))
        .await;
    assert!(
        result.is_ok(),
        "writer token on PutEntity should succeed: {result:?}"
    );
}

/// Reader token on `PutEntity` must return `PERMISSION_DENIED` (Reader < Writer).
#[tokio::test(flavor = "multi_thread")]
async fn reader_token_on_write_rpc_returns_permission_denied() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let err = client
        .put_entity(with_token(
            pb::PutEntityRequest {
                operation_id: String::new(),
                entity: Some(make_event("authz-test", "reader-put", 1)),
                create_only: false,
                if_match: String::new(),
                validate_only: false,
                force: false,
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        err.code(),
        Code::PermissionDenied,
        "reader token must be denied on Writer RPC; got {err:?}"
    );
}

/// Admin token on `DeleteByQuery` succeeds (Admin >= Admin).
/// Uses `DRY_RUN` mode with `max_deletes=1` (server requires a positive limit).
#[tokio::test(flavor = "multi_thread")]
async fn admin_token_on_delete_by_query_succeeds() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let result = client
        .delete_by_query(with_token(
            pb::DeleteByQueryRequest {
                project: String::new(),
                namespace: "authz-test".into(),
                kind: 0,
                slug: String::new(),
                mode: pb::delete_entity_request::Mode::DryRun as i32,
                // Server requires max_deletes > 0.
                max_deletes: 1,
            },
            ADMIN_TOKEN,
        ))
        .await;
    assert!(
        result.is_ok(),
        "admin token on DeleteByQuery should succeed: {result:?}"
    );
}

/// Studio's gateway credential is a `reader`. Lifted out of the gateway and
/// sent straight to the server, it must still be refused every namespace
/// mutation, or the gateway's own refusal is only advisory.
#[tokio::test(flavor = "multi_thread")]
async fn reader_token_cannot_register_or_move_a_namespace() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let err = client
        .register_namespace(with_token(
            pb::RegisterNamespaceRequest {
                name: "observer-claim".into(),
                parent: String::new(),
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied, "{err:?}");

    let err = client
        .move_namespace(with_token(
            pb::MoveNamespaceRequest {
                id: "observer-claim".into(),
                parent: "acme".into(),
            },
            READER_TOKEN,
        ))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied, "{err:?}");
}

/// Observing branches is a read. A `reader` can already diff a branch it
/// names, so refusing it the list hid nothing and left Studio unable to show
/// which branches exist.
#[tokio::test(flavor = "multi_thread")]
async fn reader_token_can_list_branches() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let result = client
        .list_branches(with_token(pb::ListBranchesRequest {}, READER_TOKEN))
        .await;
    assert!(
        result.is_ok(),
        "reader token on ListBranches should succeed: {result:?}"
    );
}

/// `WhoAmI` echoes exactly the identity `BearerAuth` resolved: name, kind,
/// role, the namespace write allow-list, and the owner the principal is
/// bound to. This is the shape an MCP client relies on to decide which
/// tools it can call without probing the server one `PERMISSION_DENIED` at
/// a time.
#[tokio::test(flavor = "multi_thread")]
async fn who_am_i_reports_a_scoped_principals_identity() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    let resp = client
        .who_am_i(with_token(pb::WhoAmIRequest {}, SCOPED_AGENT_TOKEN))
        .await
        .expect("who_am_i should succeed for a known token")
        .into_inner();

    assert_eq!(resp.name, "delegated-agent");
    assert_eq!(resp.kind, pb::PrincipalKind::Agent as i32);
    assert_eq!(resp.role, pb::Role::Writer as i32);
    assert_eq!(
        resp.namespaces,
        vec!["orders".to_string(), "shop-*".to_string()]
    );
    assert_eq!(resp.owner, "acme-org");
    assert!(!resp.anonymous);
}

/// Every token in the registry gets a distinct, correctly-shaped identity
/// back from `WhoAmI`; this guards the reader/writer/admin round trip the
/// MCP role-filtering tests build on.
#[tokio::test(flavor = "multi_thread")]
async fn who_am_i_reports_role_for_every_registered_token() {
    let addr = start_auth_server().await;
    let mut client = make_client(&addr).await;

    for (token, expected_role, expected_name) in [
        (READER_TOKEN, pb::Role::Reader, "gateway"),
        (WRITER_TOKEN, pb::Role::Writer, "mcp"),
        (ADMIN_TOKEN, pb::Role::Admin, "cli"),
    ] {
        let resp = client
            .who_am_i(with_token(pb::WhoAmIRequest {}, token))
            .await
            .unwrap_or_else(|e| panic!("who_am_i for {expected_name} failed: {e:?}"))
            .into_inner();
        assert_eq!(resp.name, expected_name);
        assert_eq!(resp.role, expected_role as i32, "role for {expected_name}");
        assert_eq!(
            resp.kind,
            pb::PrincipalKind::User as i32,
            "kind for {expected_name}"
        );
        assert!(resp.namespaces.is_empty(), "namespaces for {expected_name}");
        assert!(resp.owner.is_empty(), "owner for {expected_name}");
        assert!(!resp.anonymous, "anonymous for {expected_name}");
    }
}
