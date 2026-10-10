//! `McpServer::list_tools` must only offer tools the caller's own role can
//! actually invoke, so an agent never attempts a tool the server would
//! answer with `PERMISSION_DENIED`. These tests spin up the same
//! authentication + authorization stack as
//! `trogon-atlas-server/tests/auth_stack.rs` (`BearerAuth` -> `AuthzLayer`
//! -> the real service impl), connect `McpServer` with each role's token,
//! and assert the tool menu each role sees.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use tokio::net::TcpListener;
use tonic::service::interceptor::InterceptedService;
use trogon_atlas_mcp::server::{ToolRole, TOOL_POLICIES};
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{AuthzLayer, BearerAuth, TokenRegistry},
    service::EventModelServiceImpl,
};

const READER_TOKEN: &str = "role-filter-reader-aaaa";
const WRITER_TOKEN: &str = "role-filter-writer-bbbb";
const ADMIN_TOKEN: &str = "role-filter-admin-cccc";

fn make_registry() -> TokenRegistry {
    let toml = format!(
        r#"
[principals.reader]
role = "reader"
tokens = ["{READER_TOKEN}"]

[principals.writer]
role = "writer"
tokens = ["{WRITER_TOKEN}"]

[principals.admin]
role = "admin"
tokens = ["{ADMIN_TOKEN}"]
"#
    );
    let file: trogon_atlas_server::auth::RegistryFile =
        toml::from_str(&toml).expect("valid registry TOML");
    TokenRegistry::from_file_contents(file).expect("valid registry")
}

/// Same composition as `main.rs` and `trogon-atlas-server/tests/auth_stack.rs`:
/// `BearerAuth` authenticates before `AuthzLayer` authorizes.
async fn start_auth_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc_impl = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
    let grpc_service = EventModelServiceServer::new(svc_impl);

    let registry = Arc::new(make_registry());
    let auth = BearerAuth::new(registry);

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

/// A gRPC server with no auth stack mounted at all, same as
/// `mcp_integration.rs`'s `start_server`, but kept local here so this file
/// has no cross-file dependency on that test's helper.
async fn start_unauthenticated_server() -> String {
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

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServiceRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn mcp_client_for(
    grpc_endpoint: &str,
    auth_token: Option<&str>,
) -> (McpClient, McpServiceRunning) {
    let mcp_server =
        trogon_atlas_mcp::server::McpServer::connect(grpc_endpoint, auth_token, None, 30, None)
            .await
            .unwrap();
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (mcp_service_result, client_result) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client_result.unwrap(), mcp_service_result.unwrap())
}

fn tool_names(tools: &[rmcp::model::Tool]) -> std::collections::HashSet<String> {
    tools.iter().map(|t| t.name.to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_sees_only_reader_min_role_tools() {
    let endpoint = start_auth_server().await;
    let (client, service) = mcp_client_for(&endpoint, Some(READER_TOKEN)).await;

    let tools = client.list_all_tools().await.unwrap();
    let names = tool_names(&tools);

    for policy in TOOL_POLICIES {
        assert_eq!(
            names.contains(policy.tool),
            policy.min_role == ToolRole::Reader,
            "reader's tool list disagrees with the policy table for {}: \
             present = {}, min_role = {:?}",
            policy.tool,
            names.contains(policy.tool),
            policy.min_role
        );
    }

    assert!(names.contains("who_am_i"));
    assert!(names.contains("get_server_info"));
    assert!(names.contains("list_entities"));
    assert!(!names.contains("put_entity"));
    assert!(!names.contains("merge_branch"));

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn writer_sees_no_admin_only_tools() {
    let endpoint = start_auth_server().await;
    let (client, service) = mcp_client_for(&endpoint, Some(WRITER_TOKEN)).await;

    let tools = client.list_all_tools().await.unwrap();
    let names = tool_names(&tools);

    let admin_only: Vec<&str> = TOOL_POLICIES
        .iter()
        .filter(|p| p.min_role == ToolRole::Admin)
        .map(|p| p.tool)
        .collect();
    assert!(
        !admin_only.is_empty(),
        "expected at least one admin-only tool"
    );
    for tool in &admin_only {
        assert!(
            !names.contains(*tool),
            "writer must not see admin-only tool {tool}"
        );
    }

    for policy in TOOL_POLICIES {
        assert_eq!(
            names.contains(policy.tool),
            policy.min_role <= ToolRole::Writer,
            "writer's tool list disagrees with the policy table for {}",
            policy.tool
        );
    }

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_sees_every_tool() {
    let endpoint = start_auth_server().await;
    let (client, service) = mcp_client_for(&endpoint, Some(ADMIN_TOKEN)).await;

    let tools = client.list_all_tools().await.unwrap();
    let names = tool_names(&tools);

    assert_eq!(
        names.len(),
        TOOL_POLICIES.len(),
        "admin must see exactly every tool the policy table knows about"
    );
    for policy in TOOL_POLICIES {
        assert!(
            names.contains(policy.tool),
            "admin is missing {}",
            policy.tool
        );
    }

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn readers_can_compile_and_resolve_types() {
    let endpoint = start_auth_server().await;
    let (client, service) = mcp_client_for(&endpoint, Some(READER_TOKEN)).await;

    let names = tool_names(&client.list_all_tools().await.unwrap());
    assert!(names.contains("compile_type_library"));
    assert!(names.contains("resolve_type"));

    let library = serde_json::json!({
        "id": {"namespace": "role-filter-types", "slug": "acme.orders", "version": 1},
        "files": [{
            "path": "acme/orders/v1/orders.proto",
            "content": "syntax = \"proto3\";\npackage acme.orders.v1;\nmessage OrderPlaced { strin id = 1; }\n",
        }],
    });
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("compile_type_library").with_arguments(
                serde_json::json!({ "library_json": library.to_string() })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "a reader may compile without writing: {result:?}"
    );

    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("resolve_type").with_arguments(
                serde_json::json!({
                    "namespace": "role-filter-types",
                    "type_url": "type.googleapis.com/google.protobuf.Timestamp",
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "a reader may resolve a type: {result:?}"
    );

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_who_am_i_reports_its_own_identity() {
    let endpoint = start_auth_server().await;
    let (client, service) = mcp_client_for(&endpoint, Some(READER_TOKEN)).await;

    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("who_am_i"))
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "who_am_i must succeed: {result:?}"
    );

    let body: serde_json::Value =
        serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
    assert_eq!(body["role"], "reader");
    assert_eq!(body["kind"], "user");
    assert_eq!(body["anonymous"], false);
    assert_eq!(body["name"], "reader");

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unauthenticated_server_reports_anonymous_and_lists_every_tool() {
    let endpoint = start_unauthenticated_server().await;
    let (client, service) = mcp_client_for(&endpoint, None).await;

    let tools = client.list_all_tools().await.unwrap();
    let names = tool_names(&tools);
    assert_eq!(
        names.len(),
        TOOL_POLICIES.len(),
        "a server with no auth stack must still list every tool \
         (role can never be learned, so the adapter degrades to unfiltered)"
    );

    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("who_am_i"))
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "who_am_i must succeed: {result:?}"
    );
    let body: serde_json::Value =
        serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
    assert_eq!(body["anonymous"], true);

    service.cancel().await.unwrap();
}
