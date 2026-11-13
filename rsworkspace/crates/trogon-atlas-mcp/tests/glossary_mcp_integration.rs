#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use rmcp::{
    model::CallToolRequestParams,
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

const REVIEWER: &str = "glossary-mcp-reviewer-fixture";
const READER: &str = "glossary-mcp-reader-fixture";
const SOURCE: &str = "A parcel is a shipment handled by fulfillment. Consignment is an old internal label for a parcel.";
type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn serve() -> String {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "reviewer": {"role": "writer", "tokens": [REVIEWER]},
            "reader": {"role": "reader", "tokens": [READER]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
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
        None,
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}/mcp")
}

async fn connect(uri: &str, token: &str) -> McpClient {
    ().serve(StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned()).auth_header(token),
    ))
    .await
    .unwrap()
}

async fn call(client: &McpClient, name: &'static str, arguments: Value) -> Value {
    let result = client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    assert!(!result.is_error.unwrap_or(false), "{name}: {result:?}");
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

async fn query(client: &McpClient, name: &'static str, request: Value) -> Value {
    call(client, name, json!({"request_json": request})).await
}

fn reference(namespace: &str, slug: &str) -> Value {
    json!({"id": {"namespace": namespace, "slug": slug, "version": "1"}})
}

fn selection(namespace: &str, unreviewed: bool) -> Value {
    json!({"namespaces": [namespace], "includeUnreviewed": unreviewed})
}

async fn source(
    client: &McpClient,
    namespace: &str,
    content: &str,
    if_match: Option<&str>,
) -> Value {
    let digest = trogon_atlas_core::specification::content_digest(content).sha256;
    let mut entity = if if_match.is_some() {
        call(
            client,
            "get_entity_json",
            json!({
                "kind": "document", "namespace": namespace, "slug": "source", "version": 1
            }),
        )
        .await["json"]
            .clone()
    } else {
        call(client, "put_entity_json", json!({"create_only": true, "entity_json": {"event": {
            "id": reference(namespace, "context")["id"], "title": "Controlled vocabulary context"
        }}})).await;
        json!({"document": {
            "id": reference(namespace, "source")["id"],
            "title": {"value": "Controlled glossary source"},
            "subjects": [{"subject": {"entity": {
                "kind": "ENTITY_KIND_EVENT", "id": reference(namespace, "context")["id"]
            }}}]
        }})
    };
    entity["document"]["retained"] = json!({
        "mediaType": {"value": "text/plain"}, "content": content,
        "digest": {"sha256": digest}
    });
    let mut args = json!({"entity_json": entity});
    if let Some(etag) = if_match {
        args["if_match"] = json!(etag);
    } else {
        args["create_only"] = json!(true);
    }
    call(client, "put_entity_json", args).await;
    query(
        client,
        "inspect_glossary_sources",
        json!({"documents": [reference(namespace, "source")]}),
    )
    .await["sources"][0]
        .clone()
}

async fn candidate(
    client: &McpClient,
    namespace: &str,
    slug: &str,
    title: &str,
    definition: &str,
    material: &Value,
    policy: Value,
) -> Value {
    call(
        client,
        "put_entity_json",
        json!({"create_only": true, "entity_json": {"term": {
            "id": reference(namespace, slug)["id"], "title": title, "doc": definition,
            "glossary": {
                "evidence": [{
                    "document": material["document"], "ownerUid": material["ownerUid"],
                    "contentDigest": material["contentDigest"], "locator": "paragraph:1",
                    "quote": material["text"]
                }],
                "generationRevision": "controlled-extraction-v1", "audience": "internal",
                "nextReviewOn": {"value": "9999-12-31"}, "policy": policy
            }
        }}}),
    )
    .await;
    query(
        client,
        "get_glossary_term",
        json!({"term": reference(namespace, slug)}),
    )
    .await
}

async fn accept(client: &McpClient, entry: &Value) -> Value {
    query(
        client,
        "review_glossary_term",
        json!({
            "term": entry["entry"]["term"], "ifMatch": entry["etag"],
            "disposition": "GLOSSARY_REVIEW_DISPOSITION_ACCEPTED",
            "reason": "Fixture reviewer checked the retained passage."
        }),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_mcp_glossary_authors_reviews_compares_checks_and_detects_source_change() {
    let uri = serve().await;
    let reviewer = connect(&uri, REVIEWER).await;
    let reader = connect(&uri, READER).await;
    let material = source(&reviewer, "fulfillment-glossary", SOURCE, None).await;
    assert_eq!(material["text"], SOURCE);
    assert_eq!(
        material["contentDigest"]["sha256"],
        trogon_atlas_core::specification::content_digest(SOURCE).sha256
    );
    let draft = candidate(
        &reviewer,
        "fulfillment-glossary",
        "parcel",
        "Parcel",
        "A parcel handled by a customer.",
        &material,
        json!({"usage": "GLOSSARY_USAGE_ALLOWED"}),
    )
    .await;
    assert_eq!(draft["entry"]["state"], "GLOSSARY_ENTRY_STATE_DRAFT");
    let unpublished = query(
        &reader,
        "list_glossary_entries",
        json!({
            "selection": selection("fulfillment-glossary", false)
        }),
    )
    .await;
    assert!(unpublished["entries"].as_array().is_none_or(Vec::is_empty));
    let changes = query(
        &reviewer,
        "review_glossary_term",
        json!({
            "term": draft["entry"]["term"], "ifMatch": draft["etag"],
            "disposition": "GLOSSARY_REVIEW_DISPOSITION_NEEDS_CHANGES",
            "reason": "The retained passage describes fulfillment, not the customer."
        }),
    )
    .await;
    assert_eq!(
        changes["entry"]["state"],
        "GLOSSARY_ENTRY_STATE_NEEDS_CHANGES"
    );
    let mut corrected = changes["entity"].clone();
    corrected["term"]["doc"] = json!("A shipment handled by fulfillment.");
    call(
        &reviewer,
        "put_entity_json",
        json!({"entity_json": corrected, "if_match": changes["etag"]}),
    )
    .await;
    let corrected = query(
        &reviewer,
        "get_glossary_term",
        json!({"term": reference("fulfillment-glossary", "parcel")}),
    )
    .await;
    assert_eq!(
        corrected["entry"]["state"],
        "GLOSSARY_ENTRY_STATE_STALE_REVIEW"
    );
    let accepted = accept(&reviewer, &corrected).await;
    assert_eq!(accepted["entry"]["published"], true);
    assert_eq!(
        accepted["entity"]["term"]["glossary"]["review"]["actor"],
        "reviewer"
    );
    let listed = query(
        &reader,
        "list_glossary_entries",
        json!({
            "selection": selection("fulfillment-glossary", false), "pageSize": 1
        }),
    )
    .await;
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert!(listed["entries"][0].get("definition").is_none());
    assert!(!listed["snapshotRevision"].as_str().unwrap().is_empty());

    let legacy = candidate(
        &reviewer,
        "fulfillment-glossary",
        "consignment",
        "Consignment",
        "An old internal label for a parcel.",
        &material,
        json!({"usage": "GLOSSARY_USAGE_PROHIBITED",
            "preferredTerm": reference("fulfillment-glossary", "parcel"),
            "rationale": "Use parcel in current fulfillment copy."}),
    )
    .await;
    accept(&reviewer, &legacy).await;
    let checked = query(
        &reader,
        "check_glossary_usage",
        json!({
            "selection": selection("fulfillment-glossary", false),
            "text": "Ship the consignment. \"consignment\" is a historical quote.",
            "textRevision": "copy-revision-1", "on": {"value": "2026-11-12"},
            "skipQuoted": true
        }),
    )
    .await;
    assert_eq!(checked["textRevision"], "copy-revision-1");
    assert_eq!(checked["findings"].as_array().unwrap().len(), 1);
    assert_eq!(checked["findings"][0]["matched"], "consignment");
    assert_eq!(
        checked["findings"][0]["preferredTerm"],
        reference("fulfillment-glossary", "parcel")
    );

    let customer = source(
        &reviewer,
        "customer-glossary",
        "A parcel is a package visible to a customer.",
        None,
    )
    .await;
    let other = candidate(
        &reviewer,
        "customer-glossary",
        "parcel",
        "Parcel",
        "A package visible to a customer.",
        &customer,
        json!({"usage": "GLOSSARY_USAGE_ALLOWED"}),
    )
    .await;
    accept(&reviewer, &other).await;
    let compared = query(
        &reader,
        "compare_glossaries",
        json!({
            "left": selection("fulfillment-glossary", false),
            "right": selection("customer-glossary", false)
        }),
    )
    .await;
    assert!(compared["differences"]
        .as_array()
        .unwrap()
        .iter()
        .any(|difference| difference["kind"] == "GLOSSARY_DIFFERENCE_KIND_DIFFERENT_MEANING"));

    source(
        &reviewer,
        "fulfillment-glossary",
        &format!("{SOURCE} The process has changed."),
        material["etag"].as_str(),
    )
    .await;
    let refresh = query(
        &reader,
        "inspect_glossary_refresh",
        json!({
            "selection": selection("fulfillment-glossary", false),
            "documents": [reference("fulfillment-glossary", "source")],
            "generationRevision": "controlled-extraction-v2"
        }),
    )
    .await;
    let observations = refresh["observations"].as_array().unwrap();
    assert!(!observations.is_empty());
    assert!(observations
        .iter()
        .all(|observation| observation["generationChanged"] == true));
    assert!(observations
        .iter()
        .any(|observation| observation["entry"]["state"] == "GLOSSARY_ENTRY_STATE_SOURCE_CHANGED"));
    let tools = reader.list_all_tools().await.unwrap();
    assert!(!tools.iter().any(|tool| tool.name == "review_glossary_term"));
    assert!(tools
        .iter()
        .any(|tool| tool.name == "inspect_glossary_refresh"));
    reviewer.cancel().await.unwrap();
    reader.cancel().await.unwrap();
}
