//! Entity protection tools over MCP-over-HTTP against a real server behind
//! the auth stack: readers see only the read tools, a protected identity
//! refuses a write with `ENTITY_PROTECTED`, and a listing the server could
//! not finish says so.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};
use trogon_atlas_store::NatsStore;

const ADMIN: &str = "protection-mcp-admin";
const WRITER: &str = "protection-mcp-writer";
const READER: &str = "protection-mcp-reader";

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn serve(store: Arc<NatsStore>) -> String {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "protection-admin":{"role":"admin","tokens":[ADMIN]},
            "protection-writer":{"role":"writer","tokens":[WRITER]},
            "protection-reader":{"role":"reader","tokens":[READER]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(store, ServerMetrics::default()).unwrap();
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let app = mcp_router(
        data_plane(
            EventModelServiceServer::new(service),
            BearerAuth::new(registry),
        ),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        36 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}/mcp")
}

async fn connect(uri: &str, token: &str) -> McpClient {
    ().serve(StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned()).auth_header(token),
    ))
    .await
    .unwrap()
}

async fn call(
    client: &McpClient,
    name: &'static str,
    args: Value,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
        )
        .await
    {
        Ok(result) => Ok(result),
        Err(rmcp::ServiceError::McpError(e)) => Err(e),
        Err(other) => panic!("{name}: {other}"),
    }
}

async fn body(client: &McpClient, name: &'static str, args: Value) -> Value {
    let result = call(client, name, args)
        .await
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

async fn refusal(client: &McpClient, name: &'static str, args: Value) -> Value {
    match call(client, name, args).await {
        Err(e) => e.data.unwrap_or(Value::Null),
        Ok(result) => panic!("{name} was not refused: {result:?}"),
    }
}

fn put_event(namespace: &str, title: &str) -> Value {
    json!({"entity_json": json!({"event": {
        "id": {"namespace": namespace, "slug": "placed", "version": 1},
        "title": title,
    }}).to_string()})
}

fn identity_args(namespace: &str) -> Value {
    json!({"kind": "event", "namespace": namespace, "slug": "placed"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protection_tools_follow_server_roles_and_surface_refused_writes() {
    let uri = serve(Arc::new(
        trogon_atlas_testsupport::shared().await.store().await,
    ))
    .await;
    let admin = connect(&uri, ADMIN).await;
    let writer = connect(&uri, WRITER).await;
    let reader = connect(&uri, READER).await;

    let names = |tools: Vec<rmcp::model::Tool>| -> Vec<String> {
        tools
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    };
    let reader_tools = names(reader.list_all_tools().await.unwrap());
    for read in ["get_entity_protection", "list_entity_protections"] {
        assert!(reader_tools.iter().any(|t| t == read), "{read}");
    }
    let writer_tools = names(writer.list_all_tools().await.unwrap());
    for write in ["protect_entity", "unprotect_entity"] {
        assert!(!reader_tools.iter().any(|t| t == write), "{write}");
        assert!(!writer_tools.iter().any(|t| t == write), "{write}");
    }
    let admin_tools = names(admin.list_all_tools().await.unwrap());
    assert!(admin_tools.iter().any(|t| t == "protect_entity"));
    assert!(admin_tools.iter().any(|t| t == "unprotect_entity"));

    body(
        &writer,
        "put_entity_json",
        put_event("protection-mcp", "Placed"),
    )
    .await;
    let mut protect = identity_args("protection-mcp");
    protect["reason"] = json!("published");

    let denied = refusal(&writer, "protect_entity", protect.clone()).await;
    assert_eq!(denied["category"], "unauthorized", "{denied}");

    let protected = body(&admin, "protect_entity", protect.clone()).await;
    assert_eq!(protected["identity"], "event/protection-mcp/placed");
    assert_eq!(protected["changed"], true);
    let revision = protected["protection"]["revision"].as_u64().unwrap();

    let write = refusal(
        &writer,
        "put_entity_json",
        put_event("protection-mcp", "Edited"),
    )
    .await;
    assert_eq!(write["code"], "ENTITY_PROTECTED", "{write}");
    assert_eq!(write["next_action"], "request_unprotect", "{write}");

    let got = body(
        &reader,
        "get_entity_protection",
        identity_args("protection-mcp"),
    )
    .await;
    assert_eq!(got["state"], "protected");
    assert_eq!(got["protection"]["revision"], revision);
    let listed = body(
        &reader,
        "list_entity_protections",
        json!({"namespace": "protection-mcp"}),
    )
    .await;
    assert_eq!(listed["verdict"], "complete");
    assert_eq!(
        listed["protections"][0]["identity"],
        "event/protection-mcp/placed"
    );

    let mut stale = identity_args("protection-mcp");
    stale["if_match"] = json!(revision + 1000);
    let stale = refusal(&admin, "unprotect_entity", stale).await;
    assert_eq!(stale["code"], "STALE_PRECONDITION", "{stale}");

    let mut lift = identity_args("protection-mcp");
    lift["if_match"] = json!(revision);
    let lifted = body(&admin, "unprotect_entity", lift).await;
    assert_eq!(lifted["changed"], true);
    body(
        &writer,
        "put_entity_json",
        put_event("protection-mcp", "Edited"),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unfinished_protection_listing_is_reported_partial() {
    let nats = trogon_atlas_testsupport::shared().await;
    let (store, registry) = Box::pin(nats.store_with_protection_registry()).await;
    let uri = serve(Arc::new(store)).await;
    let admin = connect(&uri, ADMIN).await;
    let reader = connect(&uri, READER).await;

    body(
        &admin,
        "put_entity_json",
        put_event("protection-mcp-partial", "Placed"),
    )
    .await;
    let mut protect = identity_args("protection-mcp-partial");
    protect["reason"] = json!("published");
    body(&admin, "protect_entity", protect).await;
    let revision = registry.corrupt_every_row().await[0];
    registry.add_unparseable_key().await;

    let listed = body(&reader, "list_entity_protections", json!({})).await;
    assert_eq!(listed["verdict"], "partial");
    assert_eq!(listed["complete"], false);
    assert_eq!(listed["protections"], json!([]));
    assert_eq!(
        listed["unknown"][0]["identity"],
        "event/protection-mcp-partial/placed"
    );
    assert_eq!(listed["unknown"][0]["revision"], revision);
    assert_eq!(listed["unknown"][0]["liftable"], true);

    let got = body(
        &reader,
        "get_entity_protection",
        identity_args("protection-mcp-partial"),
    )
    .await;
    assert_eq!(got["state"], "unknown");

    let write = refusal(
        &admin,
        "put_entity_json",
        put_event("protection-mcp-partial", "Edited"),
    )
    .await;
    assert_eq!(write["code"], "PROTECTION_STATE_UNKNOWN", "{write}");
}
