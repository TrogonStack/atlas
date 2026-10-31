//! `export_attested_model` and `verify_attested_bundle` through a real MCP
//! session against an in-process gRPC server: a complete export verifies, an
//! edited bundle does not, and a partial export is refused unless the caller
//! asks for one marked incomplete.
//!
//! The store is shared across tests, so every test writes into its own
//! namespace.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use rmcp::ServiceExt as _;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpService =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn start_server(snapshot_cap: Option<usize>) -> String {
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let mut svc =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    if let Some(cap) = snapshot_cap {
        svc = svc.with_snapshot_cap(cap);
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(EventModelServiceServer::new(svc))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn start_mcp(grpc_endpoint: &str) -> (McpClient, McpService) {
    let server = support::mcp_server(grpc_endpoint, None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    let (service, client) = tokio::join!(
        server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client.unwrap(), service.unwrap())
}

fn id(ns: &str, slug: &str) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version: 1,
    }
}

async fn put(grpc_endpoint: &str, entity: pb::Entity) {
    EventModelServiceClient::connect(grpc_endpoint.to_string())
        .await
        .unwrap()
        .put_entity(pb::PutEntityRequest {
            operation_id: String::new(),
            entity: Some(entity),
            create_only: false,
            if_match: String::new(),
            validate_only: false,
            force: false,
        })
        .await
        .unwrap();
}

async fn seed_model(grpc_endpoint: &str, ns: &str) {
    for slug in ["placed", "shipped"] {
        put(
            grpc_endpoint,
            pb::Entity {
                system: None,
                kind: Some(pb::entity::Kind::Event(pb::Event {
                    id: Some(id(ns, slug)),
                    title: format!("Order {slug}"),
                    ..Default::default()
                })),
            },
        )
        .await;
    }
    put(
        grpc_endpoint,
        pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::EventModel(pb::EventModel {
                id: Some(id(ns, "checkout")),
                title: "Checkout".into(),
                members: ["placed", "shipped"]
                    .iter()
                    .map(|slug| pb::EntityRef {
                        kind: pb::EntityKind::Event as i32,
                        id: Some(id(ns, slug)),
                    })
                    .collect(),
                ..Default::default()
            })),
        },
    )
    .await;
}

async fn call(
    client: &McpClient,
    tool: &'static str,
    args: Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ServiceError> {
    client
        .call_tool(
            rmcp::model::CallToolRequestParams::new(tool)
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
}

fn body(result: &rmcp::model::CallToolResult) -> Value {
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

fn export_args(ns: &str, extra: &Value) -> Value {
    let mut args = json!({"namespace": ns, "slug": "checkout", "version": 1});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    args
}

async fn verify(client: &McpClient, bundle_json: &str) -> rmcp::model::CallToolResult {
    call(
        client,
        "verify_attested_bundle",
        json!({"bundle_json": bundle_json}),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_export_round_trips_through_verify() {
    let grpc = start_server(None).await;
    let ns = "mcp-attested-complete";
    seed_model(&grpc, ns).await;
    let (client, service) = start_mcp(&grpc).await;

    let tools = client.list_all_tools().await.unwrap();
    for name in ["export_attested_model", "verify_attested_bundle"] {
        assert!(tools.iter().any(|t| t.name == name), "{name} is not listed");
    }

    let summary = call(
        &client,
        "export_attested_model",
        export_args(ns, &json!({})),
    )
    .await
    .unwrap();
    let summary = body(&summary);
    assert_eq!(summary["complete"], true, "{summary}");
    assert_eq!(summary["bundle_omitted"], "not_requested", "{summary}");
    assert!(summary.get("bundle_json").is_none(), "{summary}");
    assert_eq!(summary["state"]["mode"], "MODE_CURRENT", "{summary}");

    let exported = call(
        &client,
        "export_attested_model",
        export_args(ns, &json!({"include_bundle": true})),
    )
    .await
    .unwrap();
    assert!(!exported.is_error.unwrap_or(false), "{exported:?}");
    let exported = body(&exported);
    assert_eq!(exported["complete"], true, "{exported}");
    assert_eq!(exported["incomplete_reasons"], json!([]), "{exported}");
    assert_eq!(exported["unresolved"], json!([]), "{exported}");
    assert!(exported.get("bundle_omitted").is_none(), "{exported}");
    let bundle_json = exported["bundle_json"].as_str().unwrap();
    assert_eq!(
        exported["bundle_bytes"].as_u64().unwrap(),
        bundle_json.len() as u64
    );

    let verified = verify(&client, bundle_json).await;
    assert!(!verified.is_error.unwrap_or(false), "{verified:?}");
    let verified = body(&verified);
    assert_eq!(verified["verified"], true, "{verified}");
    assert_eq!(verified["complete"], true, "{verified}");
    assert_eq!(verified["bundle_digest"], exported["bundle_digest"]);

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_bundle_fails_verify() {
    let grpc = start_server(None).await;
    let ns = "mcp-attested-tamper";
    seed_model(&grpc, ns).await;
    let (client, service) = start_mcp(&grpc).await;

    let exported = call(
        &client,
        "export_attested_model",
        export_args(ns, &json!({"include_bundle": true})),
    )
    .await
    .unwrap();
    let bundle_json = body(&exported)["bundle_json"].as_str().unwrap().to_string();
    assert!(bundle_json.contains("Order placed"));

    let tampered = verify(&client, &bundle_json.replace("Order placed", "Order lost")).await;
    assert_eq!(tampered.is_error, Some(true), "{tampered:?}");
    let tampered = body(&tampered);
    assert_eq!(tampered["verified"], false, "{tampered}");
    assert!(
        tampered["error"]
            .as_str()
            .unwrap()
            .contains("does not match"),
        "{tampered}"
    );

    let garbage = verify(&client, "{\"format\": 7}").await;
    assert_eq!(garbage.is_error, Some(true), "{garbage:?}");
    assert_eq!(body(&garbage)["verified"], false);

    service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_export_is_refused_unless_allowed_and_then_marked() {
    let grpc = start_server(Some(1)).await;
    let ns = "mcp-attested-partial";
    seed_model(&grpc, ns).await;
    let (client, service) = start_mcp(&grpc).await;

    let refused = call(
        &client,
        "export_attested_model",
        export_args(ns, &json!({})),
    )
    .await
    .unwrap_err();
    let rmcp::ServiceError::McpError(refused) = refused else {
        panic!("expected McpError, got {refused:?}");
    };
    let data = refused
        .data
        .as_ref()
        .expect("classified error carries data");
    assert_eq!(data["code"], "EXPORT_INCOMPLETE", "{refused:?}");
    assert!(
        data["next_action_detail"]["incomplete_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("REASON_SNAPSHOT_TRUNCATED")),
        "{refused:?}"
    );

    let marked = call(
        &client,
        "export_attested_model",
        export_args(
            ns,
            &json!({"allow_incomplete": true, "include_bundle": true}),
        ),
    )
    .await
    .unwrap();
    assert!(!marked.is_error.unwrap_or(false), "{marked:?}");
    let marked = body(&marked);
    assert_eq!(marked["complete"], false, "{marked}");
    let reasons = marked["incomplete_reasons"].as_array().unwrap();
    assert!(
        reasons.contains(&json!("REASON_SNAPSHOT_TRUNCATED")),
        "{marked}"
    );
    assert!(
        reasons.contains(&json!("REASON_UNRESOLVED_ENTITIES")),
        "{marked}"
    );
    let unresolved = marked["unresolved"].as_array().unwrap();
    assert!(
        unresolved.iter().any(|u| u["cause"] == "CAUSE_MISSING"
            && u["key"].as_str().is_some_and(|k| !k.is_empty())
            && u["entity"]
                .as_str()
                .is_some_and(|e| e.contains(&format!("{ns}/")))),
        "{marked}"
    );

    let verified = verify(&client, marked["bundle_json"].as_str().unwrap()).await;
    assert!(!verified.is_error.unwrap_or(false), "{verified:?}");
    let verified = body(&verified);
    assert_eq!(verified["verified"], true, "{verified}");
    assert_eq!(verified["complete"], false, "{verified}");
    assert_eq!(verified["incomplete_reasons"], marked["incomplete_reasons"]);
    assert_eq!(verified["unresolved"], marked["unresolved"]);

    service.cancel().await.unwrap();
}
