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
";

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

fn complete_model_manifest() -> String {
    let version = u64::MAX.to_string();
    let native = |kind: &str, name: &str, version: &str, spec: serde_json::Value| serde_json::json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":kind,"metadata":{"namespace":"term-export","name":name,"version":version},"spec":spec});
    let reference = |kind: &str, slug: &str| serde_json::json!({"kind":kind,"id":{"namespace":"term-export","slug":slug,"version":version}});
    [native("Domain", "domain", &version, serde_json::json!({"title":"Domain"})),
     native("Term", "declared", &version, serde_json::json!({"title":"Word","doc":"Exact definition","owningDomain":{"id":{"namespace":"term-export","slug":"domain","version":version}}})),
     native("Term", "undeclared", &version, serde_json::json!({"title":"Word","doc":"Exact definition"})),
     native("EventModel", "model", "1", serde_json::json!({"title":"Vocabulary model","members":[reference("ENTITY_KIND_TERM","declared"),reference("ENTITY_KIND_TERM","undeclared")]}))]
        .iter().map(ToString::to_string).collect::<Vec<_>>().join("\n---\n")
}

async fn native_tool(
    client: &McpClient,
    name: &'static str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new(name)
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{name}: {result:?}");
    call_tool_result_text(&result)
}
#[tokio::test(flavor = "multi_thread")]
async fn mcp_model_export_diff_and_reapply_preserve_full_native_identity_at_u64_max() {
    use trogon_atlas_proto as pb;
    let (client, running, endpoint) = start_mcp_client().await;
    let mut authoritative =
        pb::event_model_service_client::EventModelServiceClient::connect(endpoint)
            .await
            .unwrap();
    native_tool(
        &client,
        "apply_manifests",
        serde_json::json!({"manifests_yaml":complete_model_manifest()}),
    )
    .await;
    let keys = [
        (pb::EntityKind::Domain, "domain", u64::MAX),
        (pb::EntityKind::Term, "declared", u64::MAX),
        (pb::EntityKind::Term, "undeclared", u64::MAX),
        (pb::EntityKind::EventModel, "model", 1),
    ];
    let mut before = Vec::new();
    for (kind, slug, version) in keys {
        before.push(
            authoritative
                .get_entity(pb::GetEntityRequest {
                    kind: kind as i32,
                    id: Some(pb::Id {
                        namespace: "term-export".into(),
                        slug: slug.into(),
                        version,
                    }),
                })
                .await
                .unwrap()
                .into_inner(),
        );
    }
    let exported = native_tool(
        &client,
        "export_namespace",
        serde_json::json!({"namespace":"term-export"}),
    )
    .await;
    assert_eq!(exported["skipped"].as_array().unwrap().len(), 0);
    let yaml = exported["files"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n---\n");
    let parsed = trogon_atlas_client::manifest::parse_str(&yaml, "export.yaml").unwrap();
    assert_eq!(parsed.len(), 4);
    for entry in &parsed {
        assert!(before
            .iter()
            .any(|native| trogon_atlas_core::semantically_equal(
                native.entity.as_ref().unwrap(),
                &entry.entity
            )));
    }
    let diff = native_tool(
        &client,
        "diff_manifests",
        serde_json::json!({"manifests_yaml":yaml}),
    )
    .await;
    assert_eq!(diff["changed"], false);
    native_tool(
        &client,
        "apply_manifests",
        serde_json::json!({"manifests_yaml":yaml}),
    )
    .await;
    let model = authoritative
        .export_event_model_manifests(pb::ExportEventModelManifestsRequest {
            id: Some(pb::Id {
                namespace: "term-export".into(),
                slug: "model".into(),
                version: 1,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!model.truncated);
    assert_eq!(model.skipped.len(), 0);
    assert_eq!(
        native_tool(
            &client,
            "diff_manifests",
            serde_json::json!({"manifests_yaml":model.manifests})
        )
        .await["changed"],
        false
    );
    native_tool(
        &client,
        "apply_manifests",
        serde_json::json!({"manifests_yaml":model.manifests}),
    )
    .await;
    let changed = serde_json::json!({"apiVersion":trogon_atlas_client::manifest::API_VERSION,"kind":"Term","metadata":{"namespace":"term-export","name":"declared","version":u64::MAX.to_string()},"spec":{"title":"Word","doc":"Exact definition"}});
    let changed_diff = native_tool(
        &client,
        "diff_manifests",
        serde_json::json!({"manifests_yaml":changed.to_string()}),
    )
    .await;
    assert_eq!(changed_diff["changed"], true);
    assert!(changed_diff["diff"]
        .as_str()
        .unwrap()
        .contains("owningDomain"));
    let selection = |slug: &str| serde_json::json!({"kind":"term","namespace":"term-export","slug":slug,"version":u64::MAX});
    let diff = native_tool(
        &client,
        "diff_entities",
        serde_json::json!({"a":selection("declared"),"b":selection("undeclared")}),
    )
    .await;
    use base64::Engine as _;
    use prost::Message as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(diff["binpb_base64"].as_str().unwrap())
        .unwrap();
    let diff = pb::DiffEntitiesResponse::decode(bytes.as_slice()).unwrap();
    assert!(
        diff.ops.iter().any(|op| op.path == "owning_domain"),
        "{diff:?}"
    );
    let change = diff
        .ops
        .iter()
        .find(|op| op.path == "owning_domain")
        .unwrap();
    assert_eq!(change.before, format!("term-export/domain@{}", u64::MAX));
    assert_eq!(change.after, "");
    for ((kind, slug, version), expected) in keys.into_iter().zip(before) {
        let actual = authoritative
            .get_entity(pb::GetEntityRequest {
                kind: kind as i32,
                id: Some(pb::Id {
                    namespace: "term-export".into(),
                    slug: slug.into(),
                    version,
                }),
            })
            .await
            .unwrap()
            .into_inner();
        assert_ne!(actual.etag, "");
        assert_ne!(
            actual.entity.as_ref().unwrap().system.as_ref().unwrap().uid,
            ""
        );
        assert_eq!(actual, expected);
    }
    client.cancel().await.unwrap();
    running.cancel().await.unwrap();
}
