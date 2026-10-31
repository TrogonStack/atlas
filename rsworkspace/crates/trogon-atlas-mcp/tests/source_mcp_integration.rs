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
            "admin":{"role":"admin","tokens":["source-mcp-admin"]},
            "writer":{"role":"writer","tokens":["source-mcp-writer"],"parent":"source-mcp-owner"},
            "reader":{"role":"reader","tokens":["source-mcp-reader"],"parent":"source-mcp-owner"},
            "hidden":{"role":"reader","tokens":["source-mcp-hidden"],"parent":"other-owner"}
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
            name: "source-mcp".into(),
            parent: "source-mcp-owner".into(),
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
        None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_source_mcp_http_retains_scope_bytes_overlay_and_fidelity_reports() {
    let (uri, task, namespace, _) = server().await;
    let admin = connect(&uri, "source-mcp-admin").await;
    let writer = connect(&uri, "source-mcp-writer").await;
    let reader = connect(&uri, "source-mcp-reader").await;
    let hidden = connect(&uri, "source-mcp-hidden").await;
    let tools = reader.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "get_source_run"));
    assert!(tools
        .iter()
        .all(|tool| tool.name != "apply_source_run" && tool.name != "create_source_registration"));
    let registration = body(call(&admin, "create_source_registration", json!({"registration":{"id":"mcp-source","administrationNamespace":namespace,"destinationNamespaces":[namespace],"profileRevision":1,"ownershipPolicyRevision":"policy","deletionPolicy":"SOURCE_DELETION_POLICY_RETAIN_NATIVE"}})).await);
    let revision = registration["registration"]["revision"].as_str().unwrap();
    let inventory = serde_json::to_vec(&json!({"profile":"supplied-native-v1","profile_revision":1,"registration":"mcp-source","source_version":"revision","partition":{"categories":["model"],"logical_roots":["native"],"selected_keys":[]},"coverage":"PARTIAL","resources":[{"key":{"category":"model","key":"a"},"file":"native/model.yaml","pointer":{"kind":"opaque","raw":"https://never-fetch.invalid/private"}}],"relations":[{"owner":{"category":"model","key":"a"},"occurrence":"b","role":"observes","position":9,"target":{"kind":"opaque","raw":"same"},"file":"native/model.yaml"},{"owner":{"category":"model","key":"a"},"occurrence":"a","role":"observes","position":2,"target":{"kind":"opaque","raw":"same"},"file":"native/model.yaml"}]})).unwrap();
    let request = pb::AcceptSourceSnapshotRequest {
        registration_id: "mcp-source".into(),
        registration_revision: revision.into(),
        inventory_bytes: inventory,
        files: vec![pb::SourceFile {
            logical_name: "native/model.yaml".into(),
            media_type: "application/octet-stream".into(),
            content: b"\0exact\xff\n".to_vec(),
            ..Default::default()
        }],
    };
    let json = trogon_atlas_core::transcode::TranscodePool::builtin()
        .unwrap()
        .message_json_value(
            "trogonatlas.api.eventmodel.v1alpha1.AcceptSourceSnapshotRequest",
            &request,
        )
        .unwrap();
    let snapshot = body(call(&writer, "accept_source_snapshot", json).await);
    let snapshot_id = snapshot["snapshot"]["id"].as_str().unwrap();
    let bytes = body(
        call(
            &reader,
            "get_source_snapshot",
            json!({"registrationId":"mcp-source","snapshotId":snapshot_id,"includeBytes":true}),
        )
        .await,
    );
    let bytes: pb::GetSourceSnapshotResponse =
        trogon_atlas_client::source_integration::parse_request("GetSourceSnapshotResponse", &bytes)
            .unwrap();
    assert_eq!(bytes.snapshot.unwrap().files[0].content, b"\0exact\xff\n");
    let overlay = body(
        call(
            &reader,
            "get_source_overlay",
            json!({"registrationId":"mcp-source","snapshotId":snapshot_id,"pageSize":100}),
        )
        .await,
    );
    let occurrences = overlay["overlay"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["occurrenceKey"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(occurrences, vec!["b", "a"]);
    let preview = body(call(&writer, "preview_source_run", json!({"registrationId":"mcp-source","registrationRevision":revision,"snapshotId":snapshot_id,"runId":"mcp-run","overlayOnly":true})).await);
    assert_eq!(preview["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    let apply = json!({"registrationId":"mcp-source","runId":"mcp-run","ifMatch":preview["run"]["revision"],"previewDigest":preview["run"]["previewDigest"]});
    let denied = reader
        .call_tool(
            CallToolRequestParams::new("apply_source_run").with_arguments(
                json!({"request_json":apply.clone()})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(denied.is_err() || denied.unwrap().is_error.unwrap_or(false));
    let applied = body(call(&writer, "apply_source_run", apply).await);
    assert_eq!(applied["run"]["overlayOnly"], true);
    let report = body(
        call(
            &reader,
            "list_source_run_results",
            json!({"registrationId":"mcp-source","runId":"mcp-run","pageSize":1}),
        )
        .await,
    );
    assert!(report["runRevision"].as_str().is_some());
    assert!(report["previewDigest"].as_str().is_some());
    let denied = hidden
        .call_tool(
            CallToolRequestParams::new("get_source_registration").with_arguments(
                json!({"request_json":{"registrationId":"mcp-source"}})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(denied.is_err() || denied.unwrap().is_error.unwrap_or(false));
    admin.cancel().await.unwrap();
    writer.cancel().await.unwrap();
    reader.cancel().await.unwrap();
    hidden.cancel().await.unwrap();
    task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_mcp_rejects_wrong_scalar_types_before_rpc_serialization() {
    let (uri, task, _, _) = server().await;
    let admin = connect(&uri, "source-mcp-admin").await;
    for (name, input) in [
        (
            "list_source_runs",
            json!({"registrationId":"r","pageSize":"100"}),
        ),
        (
            "create_source_registration",
            json!({"registration":{"deletionPolicy":991}}),
        ),
        (
            "get_source_snapshot",
            json!({"registrationId":"r","includeBytes":null}),
        ),
    ] {
        let result = admin
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(json!({"request_json":input}).as_object().unwrap().clone()),
            )
            .await;
        let error = result.expect_err("malformed source JSON must fail locally");
        assert!(
            error.to_string().contains("invalid source request"),
            "{name}: {error}"
        );
    }
    admin.cancel().await.unwrap();
    task.abort();
}

#[path = "../../trogon-atlas-client/tests/support/source_tenant.rs"]
mod source_tenant;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connected_source_mcp_tenant_any_request_and_reports() {
    let (uri, task, namespace, endpoint) = server().await;
    let request = Box::pin(source_tenant::prepare(
        &endpoint,
        "source-mcp-admin",
        namespace,
    ))
    .await;
    let admin = connect(&uri, "source-mcp-admin").await;
    for schema in [
        json!({"@type":"type.googleapis.com/acme.money.v1.Money","unknown":"PRIVATE_SENTINEL"}),
        json!({"@type":"type.googleapis.com/acme.money.v1.Money","cents":true}),
    ] {
        let mut invalid = request.clone();
        invalid["nativePayloads"][0]["entity"]["event"]["schema"] = schema;
        let error = admin
            .call_tool(
                CallToolRequestParams::new("preview_source_run")
                    .with_arguments(json!({"request_json":invalid}).as_object().unwrap().clone()),
            )
            .await
            .unwrap_err();
        let text = format!("{error:?}");
        assert!(text.contains("invalid complete native owner"), "{text}");
        assert!(!text.contains("PRIVATE_SENTINEL"));
    }
    let preview = body(call(&admin, "preview_source_run", request).await);
    assert_eq!(preview["run"]["status"], "SOURCE_RUN_STATUS_PREVIEWED");
    source_tenant::assert_operations(&body(call(&admin, "list_source_run_operations", json!({"registrationId":"surface-tenant-source","runId":"surface-tenant-run","pageSize":100})).await));
    let applied = body(call(&admin, "apply_source_run", json!({"registrationId":"surface-tenant-source","runId":"surface-tenant-run","ifMatch":preview["run"]["revision"],"previewDigest":preview["run"]["previewDigest"]})).await);
    assert_eq!(applied["run"]["status"], "SOURCE_RUN_STATUS_APPLIED");
    source_tenant::assert_mapping(&body(
        call(
            &admin,
            "get_source_mapping",
            json!({"registrationId":"surface-tenant-source","mappingId":"surface-priced-schema"}),
        )
        .await,
    ));
    admin.cancel().await.unwrap();
    task.abort();
}
