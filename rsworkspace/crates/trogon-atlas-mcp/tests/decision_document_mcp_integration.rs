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
kind: Event
metadata: {namespace: catalog-surfaces, name: receipt}
spec: {title: Receipt}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: catalog-surfaces, name: note}
spec:
  title: {value: Retained note}
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  retained:
    mediaType: {value: text/markdown}
    content: "Retained notes\n"
    digest: {sha256: 125a1e44e8f47bf4e244de8f09f31442749f57260dec8319d252e2992a3b16f1}
  anchors: [{fragment: {value: Delivery}, label: {value: Delivery}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Person
metadata: {namespace: catalog-surfaces, name: maker}
spec:
  title: {value: Historical maker}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Team
metadata: {namespace: catalog-surfaces, name: makers}
spec:
  title: {value: Historical makers}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Decision
metadata: {namespace: catalog-surfaces, name: original}
spec:
  title: {value: Original choice}
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  status: DECISION_STATUS_PROPOSED
  context: {value: Original question}
  rationale: {value: Original reasoning}
  options: [{title: {value: Retain}, description: {value: Retain note}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Decision
metadata: {namespace: catalog-surfaces, name: choice}
spec:
  title: {value: Delivery choice}
  authoredOn: {value: 2024-02-29}
  decidedOn: {value: 2024-03-01}
  decisionMakers: [{person: maker}, {team: makers}]
  amends: [{decision: original, doc: {value: Clarifies original}}]
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  status: DECISION_STATUS_PROPOSED
  context: {value: Delivery question}
  rationale: {value: Pending adoption}
  options: [{title: {value: Retain}, description: {value: Retain note}}]
  documents: [{document: {document: note}}]
"#;

#[tokio::test(flavor = "multi_thread")]
async fn native_catalog_mcp_create_inspect_query_and_reapply() {
    let (client, mcp_service, _endpoint) = start_mcp_client().await;
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
    let get = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", serde_json::json!("document")),
                ("namespace", serde_json::json!("catalog-surfaces")),
                ("slug", serde_json::json!("note")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap();
    let body = call_tool_result_text(&get);
    assert_eq!(
        body["json"]["document"]["retained"]["content"],
        "Retained notes\n"
    );
    assert!(body["json"]["document"]["anchors"][0]["anchorId"]["value"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    let id = |slug| serde_json::json!({"namespace":"catalog-surfaces","slug":slug,"version":"1"});
    let subject = serde_json::json!({"entity":{"kind":"ENTITY_KIND_EVENT","id":id("receipt")}});
    for (tool, request) in [
        (
            "list_subject_decisions",
            serde_json::json!({"subject":subject}),
        ),
        (
            "list_subject_documents",
            serde_json::json!({"subject":subject}),
        ),
        (
            "list_decision_amendments",
            serde_json::json!({"decision":{"id":id("original")},"direction":"DECISION_AMENDMENT_DIRECTION_INCOMING","pageSize":1}),
        ),
        (
            "get_decision_detail",
            serde_json::json!({"decision":{"id":id("choice")},"followReplacement":false}),
        ),
        (
            "get_document_detail",
            serde_json::json!({"document":{"document":{"id":id("note")}}}),
        ),
        (
            "list_incoming_decision_declarations",
            serde_json::json!({"decision":{"id":id("choice")}}),
        ),
        (
            "list_incoming_document_declarations",
            serde_json::json!({"document":{"document":{"id":id("note")}}}),
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
        let json = call_tool_result_text(&result);
        assert!(json["scope"].is_object());
        if tool == "get_decision_detail" {
            assert_eq!(
                json["entity"]["decision"]["authoredOn"]["value"],
                "2024-02-29"
            );
            assert_eq!(
                json["entity"]["decision"]["decisionMakers"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(json["entity"]["decision"]["amends"][0]["connectionId"]["value"].is_string());
        }
        if tool == "list_decision_amendments" {
            assert!(json["declarations"][0]["amendment"]["connectionId"]["value"].is_string());
        }
    }
    let malformed = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_document_detail").with_arguments(
                args_obj(&[(
                    "request_json",
                    serde_json::json!({"document":{"document":{},"anchor":{}}}),
                )]),
            ),
        )
        .await;
    assert!(malformed.is_err());
    client.cancel().await.unwrap();
    mcp_service.cancel().await.unwrap();
}
