#![allow(clippy::unwrap_used, clippy::expect_used)]

use trogon_atlas_mcp::server::{ToolRole, TOOL_POLICIES};

#[test]
fn human_interaction_inspection_is_a_reader_tool() {
    let policy = TOOL_POLICIES
        .iter()
        .find(|policy| policy.tool == "inspect_human_interaction")
        .expect("Reader clients need the native inspection tool");
    assert_eq!(policy.min_role, ToolRole::Reader);
    assert_eq!(policy.rpcs, ["InspectHumanInteraction"]);
}

#[path = "../../trogon-atlas-client/tests/support/human_interaction_fixture.rs"]
mod fixture;

use std::{
    collections::HashMap,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use base64::Engine;
use fixture::{ADMIN, BRANCH, HIDDEN, OWNER, READER, WRITER};
use http::{HeaderName, HeaderValue};
use prost::Message;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use trogon_atlas_client::client::{connect, connect_with_options, ConnectOptions};
use trogon_atlas_proto::{self as pb, event_model_service_server::EventModelServiceServer};
use trogon_atlas_server::{
    auth::{BearerAuth, TokenRegistry},
    mcp::{data_plane, mcp_router},
    metrics::ServerMetrics,
    service::EventModelServiceImpl,
    telemetry::{InflightLayer, RpcMetricsLayer},
};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

async fn server() -> (String, String, String, Vec<tokio::task::JoinHandle<()>>) {
    let registry = TokenRegistry::from_file_contents(
        serde_json::from_value(json!({"principals":{
            "admin":{"role":"admin","tokens":[ADMIN]},
            "writer":{"role":"writer","kind":"agent","parent":OWNER,"tokens":[WRITER]},
            "reader":{"role":"reader","kind":"agent","parent":OWNER,"tokens":[READER]},
            "hidden":{"role":"reader","kind":"agent","parent":"outside","tokens":[HIDDEN]}
        }}))
        .unwrap(),
    )
    .unwrap();
    let service = EventModelServiceImpl::try_new(
        Arc::new(trogon_atlas_testsupport::shared().await.store().await),
        ServerMetrics::default(),
    )
    .unwrap();
    let service = EventModelServiceServer::new(service);
    let grpc_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grpc_address = grpc_listener.local_addr().unwrap();
    let grpc = data_plane(service.clone(), BearerAuth::new(registry.clone()));
    let grpc_task = tokio::spawn(async move {
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
    let http_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let endpoint = format!("http://{grpc_address}");
    let mut admin = connect(&endpoint, Some(ADMIN), 30).await.unwrap();
    let namespace = admin
        .register_namespace(pb::RegisterNamespaceRequest {
            name: "human-mcp".into(),
            parent: OWNER.into(),
        })
        .await
        .unwrap()
        .into_inner()
        .namespace
        .unwrap()
        .id;
    admin
        .create_branch(pb::CreateBranchRequest {
            name: BRANCH.into(),
            doc: "Human inspection".into(),
        })
        .await
        .unwrap();
    (
        format!("http://{address}/mcp"),
        endpoint,
        namespace,
        vec![grpc_task, http_task],
    )
}

async fn connect_mcp(uri: &str, token: &str) -> McpClient {
    let mut headers = HashMap::new();
    headers.insert(
        HeaderName::from_static("x-trogon-atlas-branch"),
        HeaderValue::from_static(BRANCH),
    );
    ().serve(StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri.to_owned())
            .auth_header(token)
            .custom_headers(headers),
    ))
    .await
    .unwrap()
}

async fn invoke(client: &McpClient, name: &'static str, arguments: Value) -> Result<Value, String> {
    match client
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
    {
        Ok(result) if result.is_error.unwrap_or(false) => Err(format!("{result:?}")),
        Ok(result) => Ok(body(&result)),
        Err(error) => Err(error.to_string()),
    }
}

fn body(result: &CallToolResult) -> Value {
    let text = &result.content[0].as_text().unwrap().text;
    serde_json::from_str(text).unwrap()
}

async fn succeed(client: &McpClient, name: &'static str, arguments: Value) -> Value {
    invoke(client, name, arguments)
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_mcp_authors_and_inspects_exact_hosting_through_export_reapply() {
    let (uri, endpoint, namespace, tasks) = server().await;
    let writer = connect_mcp(&uri, WRITER).await;
    let reader = connect_mcp(&uri, READER).await;
    let hidden = connect_mcp(&uri, HIDDEN).await;
    let catalog = reader.list_all_tools().await.unwrap();
    let tool = catalog
        .iter()
        .find(|tool| tool.name == "inspect_human_interaction")
        .expect("reader inspection tool");
    assert_eq!(
        tool.annotations.as_ref().unwrap().read_only_hint,
        Some(true)
    );
    assert_eq!(
        tool.annotations.as_ref().unwrap().destructive_hint,
        Some(false)
    );
    let manifests = fixture::manifests(&namespace);
    succeed(
        &writer,
        "apply_manifests",
        json!({"manifests_yaml":manifests,"dry_run":true}),
    )
    .await;
    succeed(
        &writer,
        "apply_manifests",
        json!({"manifests_yaml":manifests}),
    )
    .await;
    let mut grpc = connect_with_options(
        &endpoint,
        Some(WRITER),
        30,
        ConnectOptions {
            branch: Some(BRANCH.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (issuance, display, before) = fixture::selections(&mut grpc, &namespace).await;
    for request in [&issuance, &display] {
        let response = succeed(
            &reader,
            "inspect_human_interaction",
            json!({"request_json":request}),
        )
        .await;
        fixture::assert_checked(&response, request);
    }
    let mut wrong_role = display.clone();
    wrong_role["display"]["persona"] = display["display"]["sourceReadModel"].clone();
    let unresolved = succeed(
        &reader,
        "inspect_human_interaction",
        json!({"request_json":wrong_role}),
    )
    .await;
    assert_eq!(unresolved["state"], "CATALOG_RESOLUTION_STATE_UNRESOLVED");
    assert_eq!(unresolved["request"], wrong_role);
    assert!(unresolved
        .get("edges")
        .is_none_or(|edges| edges.as_array().unwrap().is_empty()));
    let document = fixture::manifest(
        &namespace,
        "Document",
        "hosting-note",
        &json!({
            "title":{"value":"Hosting assertion"},"link":{"value":"https://example.invalid/hosting"},
            "subjects":[{"subject":{"connection":issuance["hosting"]}}]
        }),
    );
    succeed(
        &writer,
        "apply_manifests",
        json!({"manifests_yaml":document}),
    )
    .await;
    let subjects = succeed(
        &reader,
        "list_subject_documents",
        json!({"request_json":{"subject":{"connection":issuance["hosting"]}}}),
    )
    .await;
    assert_eq!(subjects["declarations"].as_array().unwrap().len(), 1);
    assert_eq!(
        subjects["declarations"][0]["subject"]["subject"]["connection"],
        issuance["hosting"]
    );
    let outgoing = succeed(&reader,"get_outgoing_references",json!({"kind":"component","namespace":namespace,"slug":"portal","version":fixture::VERSION})).await;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(outgoing["binpb_base64"].as_str().unwrap())
        .unwrap();
    let outgoing = pb::GetReferencesResponse::decode(bytes.as_slice()).unwrap();
    assert_eq!(outgoing.references.len(), 2);
    for reference in outgoing.references {
        assert_eq!(
            reference.from_uid,
            issuance["hosting"]["ownerUid"].as_str().unwrap()
        );
        assert_eq!(
            reference.to.as_ref().unwrap().id.as_ref().unwrap().version,
            fixture::VERSION
        );
        assert_eq!(
            reference.from.as_ref().unwrap().kind,
            pb::EntityKind::Component as i32
        );
        assert_eq!(
            reference.from.as_ref().unwrap().id.as_ref().unwrap().slug,
            "portal"
        );
        assert_eq!(
            reference.to.as_ref().unwrap().kind,
            pb::EntityKind::Ui as i32
        );
        assert_eq!(
            reference.to.as_ref().unwrap().id.as_ref().unwrap().slug,
            "review"
        );
        let expected = match reference.from_field.as_str() {
            "ui_hostings[0].ui" => &display["hosting"]["id"]["value"],
            "ui_hostings[1].ui" => &issuance["hosting"]["id"]["value"],
            field => panic!("unexpected hosting reference field {field}"),
        };
        assert_eq!(
            reference.connection_id.as_ref().unwrap().value,
            expected.as_str().unwrap()
        );
    }
    let exported = succeed(&reader, "export_namespace", json!({"namespace":namespace})).await;
    assert_eq!(
        exported["skipped"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    let files = exported["files"].as_object().unwrap();
    for text in files.values() {
        let yaml = text.as_str().unwrap();
        let diff = succeed(&reader, "diff_manifests", json!({"manifests_yaml":yaml})).await;
        assert_eq!(diff["changed"], false);
        succeed(&writer, "apply_manifests", json!({"manifests_yaml":yaml})).await;
    }
    assert_eq!(
        fixture::get(&mut grpc, &namespace, pb::EntityKind::Component, "portal").await,
        before
    );
    let change = fixture::manifest(
        &namespace,
        "Component",
        "reader-write",
        &json!({"title":"Reader cannot author hosting","uiHostings":[{"ui":{"id":{"namespace":namespace,"slug":"review","version":fixture::VERSION.to_string()}},"label":{"value":"Forbidden"}}]}),
    );
    let denied = invoke(&reader, "apply_manifests", json!({"manifests_yaml":change}))
        .await
        .unwrap_err();
    assert!(
        denied.contains("unauthorized")
            || denied.contains("PermissionDenied")
            || denied.contains("permission"),
        "{denied}"
    );
    let invisible = invoke(
        &hidden,
        "inspect_human_interaction",
        json!({"request_json":issuance}),
    )
    .await
    .unwrap_err();
    assert!(!invisible.contains(
        before
            .entity
            .as_ref()
            .unwrap()
            .system
            .as_ref()
            .unwrap()
            .uid
            .as_str()
    ));
    for task in tasks {
        task.abort();
    }
}
