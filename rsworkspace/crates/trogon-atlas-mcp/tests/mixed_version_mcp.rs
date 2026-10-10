#![allow(clippy::unwrap_used, clippy::expect_used)]

// The MCP adapter must refuse a mutation tool before the mutation reaches a
// server that cannot honor it, while reads keep working so an agent can
// still inspect the server and see why.

use trogon_atlas_proto as pb;
use trogon_atlas_testsupport::discovery::{pre_revision_server_info, DiscoveryOnlyServer};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServiceRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn start_mcp(endpoint: &str, branch: Option<String>) -> (McpClient, McpServiceRunning) {
    let mcp_server = trogon_atlas_mcp::server::McpServer::connect(endpoint, None, None, 5, branch)
        .await
        .unwrap();
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (service, client) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client.unwrap(), service.unwrap())
}

fn tool(name: &'static str, args: serde_json::Value) -> rmcp::model::CallToolRequestParams {
    let serde_json::Value::Object(map) = args else {
        panic!("tool arguments must be an object");
    };
    rmcp::model::CallToolRequestParams::new(name).with_arguments(map)
}

fn current_server_info() -> pb::GetServerInfoResponse {
    pb::GetServerInfoResponse {
        schema_version: pb::SCHEMA_VERSION.into(),
        server_version: "test".into(),
        features: Some(pb::get_server_info_response::Features {
            mutations: true,
            validate_only: true,
            branch_scoped_requests: true,
            state_preconditions: true,
            ..Default::default()
        }),
        limits: None,
        contract_revision: pb::CONTRACT_REVISION,
        min_client_contract_revision: pb::MIN_CLIENT_CONTRACT_REVISION,
        writer_status: None,
    }
}

fn refusal_message(err: rmcp::ServiceError) -> String {
    match err {
        rmcp::ServiceError::McpError(e) => e.message.to_string(),
        other => panic!("expected McpError, got {other:?}"),
    }
}

const EVENT_JSON: &str = r#"{"event": {"id": {"namespace": "mcp-compat", "slug": "thing", "version": 1}, "title": "Thing"}}"#;

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_json_refuses_server_that_predates_contract_revisions() {
    let server = DiscoveryOnlyServer::start(pre_revision_server_info())
        .await
        .unwrap();
    let (client, service) = start_mcp(&server.endpoint, None).await;

    let info = client
        .call_tool(tool("get_server_info", serde_json::json!({})))
        .await
        .unwrap();
    assert!(
        !info.is_error.unwrap_or(false),
        "reads stay available: {info:?}"
    );

    let err = client
        .call_tool(tool(
            "put_entity_json",
            serde_json::json!({ "entity_json": EVENT_JSON }),
        ))
        .await
        .unwrap_err();
    let message = refusal_message(err);
    assert!(
        message.contains("upgrade the server"),
        "refusal must say which side to upgrade: {message}"
    );
    assert!(
        server.calls_other_than_discovery().is_empty(),
        "the mutation must never be sent: {:?}",
        server.calls()
    );

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn branch_scoped_adapter_refuses_server_that_ignores_branch_header() {
    let mut info = current_server_info();
    info.features.as_mut().unwrap().branch_scoped_requests = false;
    let server = DiscoveryOnlyServer::start(info).await.unwrap();
    let (client, service) = start_mcp(&server.endpoint, Some("feature-x".into())).await;

    let err = client
        .call_tool(tool(
            "put_entity_json",
            serde_json::json!({ "entity_json": EVENT_JSON }),
        ))
        .await
        .unwrap_err();
    let message = refusal_message(err);
    assert!(
        message.contains("branch_scoped_requests"),
        "refusal must name the missing capability: {message}"
    );
    assert_eq!(server.calls_other_than_discovery(), Vec::<String>::new());

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_only_put_refuses_server_without_validate_only() {
    let mut info = current_server_info();
    info.features.as_mut().unwrap().validate_only = false;
    let server = DiscoveryOnlyServer::start(info).await.unwrap();
    let (client, service) = start_mcp(&server.endpoint, None).await;

    let err = client
        .call_tool(tool(
            "put_entity_json",
            serde_json::json!({ "entity_json": EVENT_JSON, "validate_only": true }),
        ))
        .await
        .unwrap_err();
    let message = refusal_message(err);
    assert!(
        message.contains("validate_only"),
        "refusal must name the missing capability: {message}"
    );
    assert_eq!(server.calls_other_than_discovery(), Vec::<String>::new());

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn supported_server_lets_the_mutation_through() {
    let server = DiscoveryOnlyServer::start(current_server_info())
        .await
        .unwrap();
    let (client, service) = start_mcp(&server.endpoint, None).await;

    let _ = client
        .call_tool(tool(
            "put_entity_json",
            serde_json::json!({ "entity_json": EVENT_JSON }),
        ))
        .await;
    assert_eq!(
        server.calls_other_than_discovery(),
        vec!["/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity".to_string()]
    );

    service.cancel().await.unwrap();
}

const EVENT_MANIFEST: &str = "apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: mcp-compat, name: thing }
spec: { title: Thing }
";

#[tokio::test(flavor = "multi_thread")]
async fn diff_manifests_still_reads_a_server_that_predates_state_preconditions() {
    let server = DiscoveryOnlyServer::start_serving_entities(
        pre_revision_server_info(),
        pb::BatchGetEntitiesResponse {
            entities: vec![pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(pb::Id {
                        namespace: "mcp-compat".into(),
                        slug: "thing".into(),
                        version: 1,
                    }),
                    title: "Earlier title".into(),
                    ..Default::default()
                })),
            }],
            found: vec![true],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (client, service) = start_mcp(&server.endpoint, None).await;

    let result = client
        .call_tool(tool(
            "diff_manifests",
            serde_json::json!({ "manifests_yaml": EVENT_MANIFEST }),
        ))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let body: serde_json::Value =
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(body["changed"], serde_json::json!(true), "{body}");
    let diff = body["diff"].as_str().unwrap();
    assert!(diff.contains("Earlier title"), "{diff}");
    assert_eq!(
        server.calls_other_than_discovery(),
        vec!["/trogonatlas.api.eventmodel.v1alpha1.EventModelService/BatchGetEntities".to_string()]
    );

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn json_tools_skip_tenant_types_on_a_server_that_has_none() {
    let server = DiscoveryOnlyServer::start(current_server_info())
        .await
        .unwrap();
    let (client, service) = start_mcp(&server.endpoint, None).await;

    let builtin_schema = r#"{"event": {"id": {"namespace": "mcp-compat", "slug": "thing", "version": 1}, "schema": {"@type": "type.googleapis.com/trogonatlas.eventmodel.v1alpha1.Schema", "fields": [{"name": "order_id"}]}}}"#;
    let _ = client
        .call_tool(tool(
            "put_entity_json",
            serde_json::json!({ "entity_json": builtin_schema }),
        ))
        .await;
    assert_eq!(
        server.calls_other_than_discovery(),
        vec!["/trogonatlas.api.eventmodel.v1alpha1.EventModelService/PutEntity".to_string()],
        "built-in types still transcode, and the server is never asked for tenant types"
    );

    service.cancel().await.unwrap();
}
