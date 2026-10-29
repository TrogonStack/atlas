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
kind: Component
metadata: {namespace: topology-surfaces, name: checkout}
spec: {title: Checkout}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
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
  subjects: [{subject: {entity: {kind: ENTITY_KIND_COMPONENT, id: {namespace: topology-surfaces, slug: checkout, version: 1}}}}]
  steps:
    - title: {value: Start}
      target: {component: {kind: ENTITY_KIND_COMPONENT, id: {namespace: topology-surfaces, slug: checkout, version: 1}}}
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
    let call = |tool: &'static str, request: serde_json::Value| {
        let client = &client;
        async move {
            let result = client
                .call_tool(
                    rmcp::model::CallToolRequestParams::new(tool)
                        .with_arguments(args_obj(&[("request_json", request)])),
                )
                .await
                .unwrap();
            assert!(!result.is_error.unwrap_or(false), "{tool}: {result:?}");
            call_tool_result_text(&result)
        }
    };

    let detail = call(
        "get_design_graph_detail",
        serde_json::json!({"graph":{"id":id("flow")}}),
    )
    .await;
    let steps = detail["entity"]["designGraph"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2, "{detail}");
    let start = steps[0]["stepId"]["value"].as_str().unwrap();
    let invoke = steps[1]["stepId"]["value"].as_str().unwrap();
    assert_ne!(start, invoke);
    assert_eq!(detail["entity"]["designGraph"]["title"]["value"], "Flow");
    let targets = detail["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2, "{detail}");
    assert_eq!(targets[0]["field"], "steps[0].target");
    assert_eq!(targets[1]["field"], "steps[1].target");
    for target in targets {
        assert_eq!(target["state"], "CATALOG_RESOLUTION_STATE_RESOLVED");
    }
    assert_eq!(detail["declarations"].as_array().unwrap().len(), 1);
    assert_eq!(detail["declarations"][0]["field"], "subjects[0].subject");
    assert!(detail.get("layoutState").is_none());

    let subject =
        serde_json::json!({"entity":{"kind":"ENTITY_KIND_COMPONENT","id":id("checkout")}});
    let listed = call(
        "list_subject_design_graphs",
        serde_json::json!({"subject":subject}),
    )
    .await;
    assert_eq!(
        listed["declarations"].as_array().unwrap().len(),
        1,
        "{listed}"
    );
    assert_eq!(
        listed["declarations"][0]["owner"]["owner"]["id"]["slug"],
        "flow"
    );
    assert_eq!(listed["subjectState"], "CATALOG_RESOLUTION_STATE_RESOLVED");

    let target_only = call(
        "list_subject_design_graphs",
        serde_json::json!({"subject":{"entity":{"kind":"ENTITY_KIND_DESIGN_GRAPH","id":id("callee")}}}),
    )
    .await;
    assert!(target_only.get("declarations").is_none(), "{target_only}");

    let traversal = call(
        "traverse_design_graph_topology",
        serde_json::json!({"root":{"id":id("flow")},"followSubflows":true}),
    )
    .await;
    let nodes = traversal["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3, "{traversal}");
    assert_eq!(nodes[0]["stepId"]["value"], start);
    assert_eq!(nodes[1]["stepId"]["value"], invoke);
    assert_eq!(nodes[2]["depth"], 1);
    assert_eq!(nodes[2]["graph"]["owner"]["id"]["slug"], "callee");
    let edges = traversal["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1, "{traversal}");
    assert_eq!(edges[0]["kind"], "KIND_SUBFLOW");
    assert_eq!(
        traversal["subflows"][0]["state"],
        "TOPOLOGY_SUBFLOW_STATE_EXPANDED"
    );

    let incoming = call(
        "list_incoming_design_graph_subflows",
        serde_json::json!({"graph":{"id":id("callee")}}),
    )
    .await;
    let references = incoming["references"].as_array().unwrap();
    assert_eq!(references.len(), 1, "{incoming}");
    assert_eq!(references[0]["field"], "steps[1].target.subflow");
    assert_eq!(references[0]["stepId"]["value"], invoke);
    assert_eq!(references[0]["owner"]["owner"]["id"]["slug"], "flow");
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
