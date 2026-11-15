#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;

use base64::Engine as _;
use prost::Message;
use rmcp::ServiceExt;
use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_server::{metrics::ServerMetrics, service::EventModelServiceImpl};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

async fn connected() -> (McpClient, McpRunning) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let service =
        EventModelServiceImpl::try_new(Arc::new(store), ServerMetrics::default()).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(pb::event_model_service_server::EventModelServiceServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    let server = support::mcp_server(&format!("http://{addr}"), None);
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    let (running, client) = tokio::join!(
        server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client.unwrap(), running.unwrap())
}

async fn call(
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
        .expect("the advertised retrieval parameters must be accepted");
    assert!(!result.is_error.unwrap_or(false), "{name}: {result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

fn decode<M: Message + Default>(value: &serde_json::Value) -> M {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value["binpb_base64"].as_str().unwrap())
        .unwrap();
    M::decode(bytes.as_slice()).unwrap()
}

async fn term(client: &McpClient, namespace: &str, version: u64) {
    call(
        client,
        "put_entity_json",
        serde_json::json!({
            "entity_json": {"term": {
                "id": {"namespace": namespace, "slug": "shipment", "version": version.to_string()},
                "title": "Shipment vocabulary marker",
                "doc": "The contextual meaning of shipment."
            }},
            "create_only": true
        }),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_search_restricts_terms_to_the_selected_contexts() {
    let (client, running) = connected().await;
    term(&client, "fulfillment", 1).await;
    term(&client, "customer", 1).await;
    let selected: pb::SearchEntitiesResponse = decode(
        &call(
            &client,
            "search_entities",
            serde_json::json!({
                "q": "vocabulary marker", "kinds": ["term"], "namespaces": ["fulfillment"]
            }),
        )
        .await,
    );
    assert_eq!(selected.results.len(), 1);
    let entry = selected.results[0].entity.as_ref().unwrap();
    assert_eq!(
        trogon_atlas_core::entity_id(entry).unwrap().namespace,
        "fulfillment"
    );
    let all: pb::SearchEntitiesResponse = decode(
        &call(
            &client,
            "search_entities",
            serde_json::json!({"q": "vocabulary marker", "kinds": ["term"]}),
        )
        .await,
    );
    assert_eq!(all.results.len(), 2);
    client.cancel().await.unwrap();
    running.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_listing_selects_latest_versions_only_when_requested() {
    let (client, running) = connected().await;
    term(&client, "fulfillment", 1).await;
    term(&client, "fulfillment", 2).await;
    let all: pb::ListEntitiesResponse = decode(
        &call(
            &client,
            "list_entities",
            serde_json::json!({"kind": "term", "namespace": "fulfillment"}),
        )
        .await,
    );
    assert_eq!(all.entities.len(), 2);
    let latest: pb::ListEntitiesResponse = decode(
        &call(
            &client,
            "list_entities",
            serde_json::json!({
                "kind": "term", "namespace": "fulfillment", "latest_versions_only": true
            }),
        )
        .await,
    );
    assert_eq!(latest.entities.len(), 1);
    assert_eq!(
        trogon_atlas_core::entity_id(&latest.entities[0])
            .unwrap()
            .version,
        2
    );
    client.cancel().await.unwrap();
    running.cancel().await.unwrap();
}
