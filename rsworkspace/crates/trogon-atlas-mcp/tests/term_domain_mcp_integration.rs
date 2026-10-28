#![allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
mod support;
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

async fn start_mcp_client() -> (McpClient, McpServiceRunning, String) {
    let grpc_endpoint = start_server().await;
    let mcp_server = support::mcp_server(&grpc_endpoint, None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (mcp_service_result, client_result) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (
        client_result.unwrap(),
        mcp_service_result.unwrap(),
        grpc_endpoint,
    )
}

fn args_obj(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

fn call_tool_result_text(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

const MANIFEST: &str = r#"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Domain
metadata: {namespace: vocabulary, name: sales}
spec: {title: Sales}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Term
metadata: {namespace: vocabulary, name: order}
spec:
  title: Order
  owningDomain: {id: {namespace: vocabulary, slug: sales, version: '1'}}
"#;

#[tokio::test(flavor = "multi_thread")]
async fn term_domain_mcp_typed_query_roundtrip() {
    let (client, mcp_service, _) = start_mcp_client().await;
    for _ in 0..2 {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("apply_manifests")
                    .with_arguments(args_obj(&[("manifests_yaml", serde_json::json!(MANIFEST))])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
    }
    let result = client.call_tool(rmcp::model::CallToolRequestParams::new("list_term_domain_declarations")
        .with_arguments(args_obj(&[("request_json", serde_json::json!({"domain":{"id":{"namespace":"vocabulary","slug":"sales","version":"1"}},"pageSize":1}))]))).await.unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let body = call_tool_result_text(&result);
    assert_eq!(body["declarations"][0]["domain"]["id"]["slug"], "sales");
    client.cancel().await.unwrap();
    mcp_service.cancel().await.unwrap();
}
