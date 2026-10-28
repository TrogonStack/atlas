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

const MANIFEST: &str = r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {namespace: topology-surfaces, name: callee}
spec:
  title: {value: Callee}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  steps: [{title: {value: Work}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: DesignGraph
metadata: {namespace: topology-surfaces, name: flow}
spec:
  title: {value: Flow}
  profile: DESIGN_GRAPH_PROFILE_FLOW
  steps:
    - title: {value: Start}
    - title: {value: Call}
      target: {subflow: {id: {namespace: topology-surfaces, slug: callee, version: 1}}}
";

#[tokio::test(flavor = "multi_thread")]
async fn design_topology_mcp_apply_inspect_traverse_and_incoming() {
    let (client, mcp_service, _endpoint) = start_mcp_client().await;
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests")
                .with_arguments(args_obj(&[("manifests_yaml", serde_json::json!(MANIFEST))])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let id = |slug| serde_json::json!({"namespace":"topology-surfaces","slug":slug,"version":"1"});
    let subject = serde_json::json!({"entity":{"kind":"ENTITY_KIND_DESIGN_GRAPH","id":id("flow")}});
    let mut traversal = None;
    for (tool, request) in [
        (
            "get_design_graph_detail",
            serde_json::json!({"graph":{"id":id("flow")}}),
        ),
        (
            "list_subject_design_graphs",
            serde_json::json!({"subject":subject}),
        ),
        (
            "traverse_design_graph_topology",
            serde_json::json!({"root":{"id":id("flow")},"followSubflows":true}),
        ),
        (
            "list_incoming_design_graph_subflows",
            serde_json::json!({"graph":{"id":id("callee")}}),
        ),
    ] {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new(tool)
                    .with_arguments(args_obj(&[("request_json", request)])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{tool}: {result:?}");
        let body = call_tool_result_text(&result);
        assert!(
            body["scope"].is_object() || body["entity"].is_object(),
            "{tool}: {body}"
        );
        if tool == "traverse_design_graph_topology" {
            traversal = Some(body.clone());
        }
        if tool == "list_incoming_design_graph_subflows" {
            assert_eq!(body["references"].as_array().unwrap().len(), 1, "{body}");
        }
    }
    let traversal = traversal.unwrap();
    assert!(traversal["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["depth"] == 1));
    let malformed = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_design_graph_detail").with_arguments(
                args_obj(&[("request_json", serde_json::json!({"graph":{},"unknown":1}))]),
            ),
        )
        .await;
    assert!(malformed.is_err());
    client.cancel().await.unwrap();
    mcp_service.cancel().await.unwrap();
}
