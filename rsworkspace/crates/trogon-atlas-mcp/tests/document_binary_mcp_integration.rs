#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use base64::Engine;
use prost::Message;
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tonic::Request;
use trogon_atlas_proto::{
    self as pb,
    event_model_service_server::{EventModelService, EventModelServiceServer},
};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn server() -> (String, tokio::task::JoinHandle<()>, String, String) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals": {
            "admin":{"role":"admin","tokens":["document-binary-mcp-admin"]},
            "writer":{"role":"writer","tokens":["document-binary-mcp-writer"],"parent":"document-binary-mcp-owner"},
            "reader":{"role":"reader","tokens":["document-binary-mcp-reader"],"parent":"document-binary-mcp-owner"},
            "hidden":{"role":"reader","tokens":["document-binary-mcp-hidden"],"parent":"other-owner"}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let namespace = service
        .register_namespace(Request::new(pb::RegisterNamespaceRequest {
            name: "document-binary-mcp".into(),
            parent: "document-binary-mcp-owner".into(),
        }))
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    let service = EventModelServiceServer::new(service)
        .max_decoding_message_size(36 * 1024 * 1024)
        .max_encoding_message_size(36 * 1024 * 1024);
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_address = grpc_listener.local_addr().unwrap();
    let grpc = data_plane(service.clone(), BearerAuth::new(registry.clone()));
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(
                grpc_listener,
            ))
            .await
            .unwrap();
    });
    let (drain, _) = tokio::sync::watch::channel(0i64);
    let app = mcp_router(
        data_plane(service, BearerAuth::new(registry)),
        RpcMetricsLayer::new(ServerMetrics::default()),
        InflightLayer::new(Arc::new(AtomicI64::new(0)), Arc::new(drain)),
        Some(Duration::from_secs(30)),
        36 * 1024 * 1024,
        "127.0.0.1".parse().unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        format!("http://{address}/mcp"),
        task,
        namespace,
        format!("http://{grpc_address}"),
    )
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
    request: Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name)
                .with_arguments(json!({"request_json":request}).as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}
fn body(result: rmcp::model::CallToolResult) -> Value {
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let content = result.content.into_iter().next().unwrap();
    serde_json::from_str(&content.as_text().unwrap().text).unwrap()
}

const MANIFEST: &str = r"apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: {namespace: NAMESPACE, name: receipt}
spec: {title: Receipt}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: NAMESPACE, name: text}
spec:
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  title: {value: Text anchor target}
  retained:
    mediaType: {value: text/plain}
    content: inert
    digest: {sha256: 3321675705932e37c55c0ba4b0870788177e3f5670b9d7ea7a3206e287dd5ead}
  anchors: [{fragment: {value: example}, label: {value: Example}}]
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: NAMESPACE, name: general}
spec:
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  title: {value: Explicit general}
  purpose: DOCUMENT_PURPOSE_GENERAL
  link: {value: https://example.invalid/inert}
---
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Document
metadata: {namespace: NAMESPACE, name: binary, version: 9007199254740993}
spec:
  subjects: [{subject: {entity: {kind: event, id: receipt}}}]
  title: {value: Inert attachment}
  filename: {value: sample.bin}
  metadata:
    - '@type': type.googleapis.com/trogonatlas.eventmodel.v1alpha1.DocumentCollectionAnnotation
      category: {value: Downloads}
  relatedContent:
    - document: {document: text}
      role: RELATED_CONTENT_ROLE_EXAMPLE
    - document: {document: text}
      role: RELATED_CONTENT_ROLE_DIAGRAM
  retainedContent:
    mediaType: {value: application/octet-stream}
    content: AGV4YWN0/wo=
    digest: {sha256: ea545ad36a9c4c5bfc95d99937328130e5834502247a772fd585d943c97e6831}
";

async fn tool(client: &McpClient, name: &'static str, args: Value) -> Value {
    body(
        client
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap(),
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_document_binary_mcp_roundtrip() {
    let (uri, task, namespace, _) = server().await;
    let writer = connect(&uri, "document-binary-mcp-writer").await;
    let manifest = MANIFEST.replace("NAMESPACE", &namespace);
    tool(
        &writer,
        "apply_manifests",
        json!({"manifests_yaml":manifest,"dry_run":true}),
    )
    .await;
    tool(
        &writer,
        "apply_manifests",
        json!({"manifests_yaml":manifest}),
    )
    .await;
    let get = json!({"kind":"document","namespace":namespace,"slug":"binary","version":9_007_199_254_740_993_u64});
    let text = tool(
        &writer,
        "get_entity_json",
        json!({"kind":"document","namespace":namespace,"slug":"text","version":1}),
    )
    .await;
    let mut owner = tool(&writer, "get_entity_json", get.clone()).await;
    let mut edited = owner["json"].clone();
    let anchor = json!({"anchor":{"document":{"id":{"namespace":namespace,"slug":"text","version":"1"}},"ownerUid":text["json"]["system"]["uid"],"anchorId":text["json"]["document"]["anchors"][0]["anchorId"],"expectedDigest":text["json"]["document"]["retained"]["digest"]}});
    edited["document"]["relatedContent"]
        .as_array_mut()
        .unwrap()
        .push(json!({"document":anchor,"role":"RELATED_CONTENT_ROLE_EXAMPLE"}));
    tool(
        &writer,
        "put_entity_json",
        json!({"entity_json":edited,"if_match":owner["etag"],"validate_only":true}),
    )
    .await;
    assert_eq!(owner, tool(&writer, "get_entity_json", get.clone()).await);
    tool(
        &writer,
        "put_entity_json",
        json!({"entity_json":edited,"if_match":owner["etag"]}),
    )
    .await;
    owner = tool(&writer, "get_entity_json", get.clone()).await;
    assert_eq!(
        owner["json"]["document"]["relatedContent"][2]["document"],
        anchor
    );
    let detail = body(call(&writer,"get_document_detail",json!({"document":{"document":{"id":{"namespace":namespace,"slug":"binary","version":"9007199254740993"}}}})).await);
    assert!(detail.to_string().contains("AGV4YWN0/wo="));
    let before = tool(&writer, "get_entity_json", get.clone()).await;
    let doc = &before["json"]["document"];
    assert_eq!(doc["id"]["version"], "9007199254740993");
    assert_eq!(doc["retainedContent"]["content"], "AGV4YWN0/wo=");
    assert_eq!(
        doc["retainedContent"]["mediaType"]["value"],
        "application/octet-stream"
    );
    assert_eq!(doc["filename"]["value"], "sample.bin");
    assert!(doc.get("purpose").is_none());
    let general = tool(
        &writer,
        "get_entity_json",
        json!({"kind":"document","namespace":namespace,"slug":"general","version":1}),
    )
    .await;
    assert_eq!(
        general["json"]["document"]["purpose"],
        "DOCUMENT_PURPOSE_GENERAL"
    );
    assert_eq!(doc["relatedContent"].as_array().unwrap().len(), 3);
    assert_ne!(
        doc["relatedContent"][0]["connectionId"],
        doc["relatedContent"][1]["connectionId"]
    );
    for (field, value) in [
        ("digest", json!({"sha256":"0".repeat(64)})),
        ("mediaType", json!({"value":"Application/Octet-Stream"})),
    ] {
        let mut invalid = before["json"].clone();
        invalid["document"]["retainedContent"][field] = value;
        let result = writer
            .call_tool(
                CallToolRequestParams::new("put_entity_json").with_arguments(
                    json!({"entity_json":invalid,"if_match":before["etag"]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await;
        assert!(result.is_err(), "backend accepted {field}: {result:?}");
        assert_eq!(before, tool(&writer, "get_entity_json", get.clone()).await);
    }
    let mut source_selection = before["json"].clone();
    source_selection["document"]["relatedContent"][0]["document"] = json!({
        "source": { "registrationId": "native-document-source", "category": "model", "key": "retained-example" }
    });
    let refused_source = writer
        .call_tool(
            CallToolRequestParams::new("put_entity_json").with_arguments(
                json!({"entity_json":source_selection,"if_match":before["etag"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(
        refused_source.is_err(),
        "native Document accepted a Source selection: {refused_source:?}"
    );
    assert_eq!(before, tool(&writer, "get_entity_json", get.clone()).await);
    let outgoing = tool(&writer, "get_outgoing_references", get.clone()).await;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(outgoing["binpb_base64"].as_str().unwrap())
        .unwrap();
    let references = pb::GetReferencesResponse::decode(bytes.as_slice()).unwrap();
    let related = references
        .references
        .iter()
        .filter(|r| r.from_field.starts_with("related_content"))
        .collect::<Vec<_>>();
    assert_eq!(related.len(), 3);
    assert!(related.iter().all(|r| r.connection_id.is_some()));
    let impact = tool(
        &writer,
        "get_impact",
        json!({"kind":"document","namespace":namespace,"slug":"text","version":1}),
    )
    .await;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(impact["binpb_base64"].as_str().unwrap())
        .unwrap();
    let impact = pb::GetImpactResponse::decode(bytes.as_slice()).unwrap();
    let mut reached = impact
        .nodes
        .iter()
        .map(|node| {
            let entity = node.entity.as_ref().unwrap();
            let id = entity.id.as_ref().unwrap();
            (
                entity.kind,
                id.namespace.clone(),
                id.slug.clone(),
                id.version,
                node.depth,
            )
        })
        .collect::<Vec<_>>();
    reached.sort();
    let mut expected = [
        (
            pb::EntityKind::Event as i32,
            namespace.clone(),
            "receipt".into(),
            1,
            1,
        ),
        (
            pb::EntityKind::Document as i32,
            namespace.clone(),
            "text".into(),
            1,
            0,
        ),
        (
            pb::EntityKind::Document as i32,
            namespace.clone(),
            "general".into(),
            1,
            2,
        ),
        (
            pb::EntityKind::Document as i32,
            namespace.clone(),
            "binary".into(),
            9_007_199_254_740_993,
            1,
        ),
    ];
    expected.sort();
    assert_eq!(reached, expected);
    let affected = impact
        .nodes
        .iter()
        .find(|node| {
            node.entity
                .as_ref()
                .and_then(|entity| entity.id.as_ref())
                .is_some_and(|id| {
                    id.namespace == namespace
                        && id.slug == "binary"
                        && id.version == 9_007_199_254_740_993
                })
        })
        .unwrap();
    assert_eq!(affected.depth, 1);
    assert_eq!(affected.path.len(), 1);
    let path = &affected.path[0];
    let selected = related
        .iter()
        .find(|reference| reference.connection_id == path.connection_id)
        .unwrap();
    assert_eq!(path.from_field, selected.from_field);
    assert_eq!(path.from_uid, selected.from_uid);
    assert_eq!(path.from, selected.from);
    assert_eq!(path.to, selected.to);
    assert_eq!(impact.next_page_token, "");
    let exported = tool(&writer, "export_namespace", json!({"namespace":namespace})).await;
    let files = exported["files"].as_object().unwrap();
    let yaml = files
        .values()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n---\n");
    assert_eq!(
        tool(&writer, "diff_manifests", json!({"manifests_yaml":yaml})).await["changed"],
        false
    );
    tool(&writer, "apply_manifests", json!({"manifests_yaml":yaml})).await;
    assert_eq!(before, tool(&writer, "get_entity_json", get).await);
    writer.cancel().await.unwrap();
    task.abort();
}
