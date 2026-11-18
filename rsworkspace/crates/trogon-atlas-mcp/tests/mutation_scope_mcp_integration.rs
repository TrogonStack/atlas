#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

// Every mutating tool names the scope it wrote to, so a caller can tell a
// branch write from a baseline write without exporting and diffing both.

use std::sync::Arc;

use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
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
    branch: Option<&str>,
) -> (McpClient, McpServiceRunning) {
    let mcp_server = support::mcp_server(grpc_endpoint, branch.map(str::to_string));
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (mcp_service_result, client_result) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client_result.unwrap(), mcp_service_result.unwrap())
}

fn args_obj(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

fn body_of(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

fn event_json(ns: &str, slug: &str) -> String {
    serde_json::json!({
        "event": {
            "id": {"namespace": ns, "slug": slug, "version": 1},
            "title": slug,
        }
    })
    .to_string()
}

async fn put_then_delete(client: &McpClient, ns: &str) -> (serde_json::Value, serde_json::Value) {
    let put = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(args_obj(&[
                ("entity_json", serde_json::json!(event_json(ns, "scoped"))),
            ])),
        )
        .await
        .unwrap();
    let delete = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_entity").with_arguments(args_obj(&[
                ("kind", serde_json::json!("Event")),
                ("namespace", serde_json::json!(ns)),
                ("slug", serde_json::json!("scoped")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap();
    (body_of(&put), body_of(&delete))
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_scoped_mutations_name_their_branch() {
    let grpc_endpoint = start_server().await;
    let (admin, admin_service) = mcp_client_for(&grpc_endpoint, None).await;
    admin
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(args_obj(&[
                ("name", serde_json::json!("scope-echo")),
                ("doc", serde_json::json!("")),
            ])),
        )
        .await
        .unwrap();

    let (client, service) = mcp_client_for(&grpc_endpoint, Some("scope-echo")).await;
    let (put, delete) = put_then_delete(&client, "scope-echo-ns").await;
    for body in [&put, &delete] {
        assert_eq!(body["scope"], "branch", "{body}");
        assert_eq!(body["branch"], "scope-echo", "{body}");
    }
    service.cancel().await.unwrap();
    admin_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn baseline_mutations_say_baseline() {
    let grpc_endpoint = start_server().await;
    let (client, service) = mcp_client_for(&grpc_endpoint, None).await;
    let (put, delete) = put_then_delete(&client, "scope-baseline-ns").await;
    for body in [&put, &delete] {
        assert_eq!(body["scope"], "baseline", "{body}");
        assert!(body["branch"].is_null(), "{body}");
    }
    service.cancel().await.unwrap();
}
