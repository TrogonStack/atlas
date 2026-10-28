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

use serde_json::json;
fn fixture() -> serde_json::Value {
    let target = json!({"kind":"ENTITY_KIND_COMPONENT","id":{"namespace":"catalog","slug":"target","version":"18446744073709551615"}});
    let raw = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"agent","version":"9007199254740993"},"spec":{"title":"Agent","metadata":[
        {"@type":"type.googleapis.com/trogonatlas.eventmodel.v1alpha1.AgentAnnotation","model":{"version":{"value":"opaque preview"}},"tools":[{"name":{"value":"lookup"},"type":{"value":"manual"},"description":{"value":"Inert description"},"icon":{"value":"unknown:token"},"url":{"value":"https://example.test/tool"}},{"name":{"value":"lookup"},"type":{"value":"other"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.LifecycleAnnotation","status":"draft","draft":{"message":{"value":"Review required"}}},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.BadgesAnnotation","badges":[{"content":{"value":"Preview"},"backgroundColor":{"value":"#aAbBcC"},"textColor":{"value":"#FFFFFF"}}]},
        {"@type":"type.googleapis.com/trogonatlas.annotation.v1alpha1.ResourceGroupsAnnotation","groups":[{"groupId":{"value":"10000000-0000-4000-8000-000000000001"},"displayLimit":{"value":1},"sidebar":false,"items":[{"connectionId":{"value":"10000000-0000-4000-8000-000000000002"},"target":target},{"connectionId":{"value":"10000000-0000-4000-8000-000000000003"},"target":target}]}]}
    ]}});
    raw
}

#[tokio::test(flavor = "multi_thread")]
async fn native_metadata_mcp_apply_get_and_reapply() {
    let (client, service, _) = start_mcp_client().await;
    let target = json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Component","metadata":{"namespace":"catalog","name":"target","version":"18446744073709551615"},"spec":{"title":"Target"}});
    for manifest in [target, fixture(), fixture()] {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("apply_manifests")
                    .with_arguments(args_obj(&[("manifests_yaml", json!(manifest.to_string()))])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
    }
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", json!("component")),
                ("namespace", json!("catalog")),
                ("slug", json!("agent")),
                ("version", json!(9_007_199_254_740_993_u64)),
            ])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let body = call_tool_result_text(&result);
    assert_eq!(
        body["json"]["component"]["metadata"][3]["groups"][0]["sidebar"],
        false
    );
    assert_eq!(
        body["json"]["component"]["metadata"][3]["groups"][0]["items"][1]["target"]["id"]
            ["version"],
        "18446744073709551615"
    );
    let mut invalid = fixture();
    invalid["spec"]["metadata"][0]["model"] = json!({});
    let rejected = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests")
                .with_arguments(args_obj(&[("manifests_yaml", json!(invalid.to_string()))])),
        )
        .await
        .unwrap_err();
    assert!(
        rejected.to_string().contains("invalid metadata"),
        "{rejected:?}"
    );
    let unchanged = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", json!("component")),
                ("namespace", json!("catalog")),
                ("slug", json!("agent")),
                ("version", json!(9_007_199_254_740_993_u64)),
            ])),
        )
        .await
        .unwrap();
    assert_eq!(call_tool_result_text(&unchanged), body);
    let diff = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("diff_manifests").with_arguments(args_obj(&[
                ("manifests_yaml", json!(fixture().to_string())),
            ])),
        )
        .await
        .unwrap();
    assert_eq!(call_tool_result_text(&diff)["changed"], false);
    use base64::Engine as _;
    use prost::Message as _;
    let mut missing_item_identity = Vec::new();
    for (tool, slug, version) in [
        (
            "get_outgoing_references",
            "agent",
            9_007_199_254_740_993_u64,
        ),
        ("get_incoming_references", "target", u64::MAX),
    ] {
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new(tool).with_arguments(args_obj(&[
                    ("kind", json!("component")),
                    ("namespace", json!("catalog")),
                    ("slug", json!(slug)),
                    ("version", json!(version)),
                ])),
            )
            .await
            .unwrap();
        assert!(!result.is_error.unwrap_or(false), "{result:?}");
        let payload = call_tool_result_text(&result);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload["binpb_base64"].as_str().unwrap())
            .unwrap();
        let refs = trogon_atlas_proto::GetReferencesResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(refs.references.len(), 2);
        assert_ne!(refs.references[0].from_field, refs.references[1].from_field);
        for reference in refs.references {
            assert!(
                !reference.from_uid.is_empty(),
                "missing authoritative owner UID: {reference:?}"
            );
            if reference.connection_id.is_none() {
                missing_item_identity.push(reference);
            }
        }
    }
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_impact").with_arguments(args_obj(&[
                ("kind", json!("component")),
                ("namespace", json!("catalog")),
                ("slug", json!("target")),
                ("version", json!(u64::MAX)),
                ("max_depth", json!(1)),
            ])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let payload = call_tool_result_text(&result);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload["binpb_base64"].as_str().unwrap())
        .unwrap();
    let impact = trogon_atlas_proto::GetImpactResponse::decode(bytes.as_slice()).unwrap();
    assert!(impact.nodes.iter().any(|node| node.depth == 1
        && node.entity.as_ref().unwrap().id.as_ref().unwrap().slug == "agent"));
    assert_eq!(
        missing_item_identity,
        [],
        "backend must return item provenance"
    );
    client.cancel().await.unwrap();
    service.cancel().await.unwrap();
}
