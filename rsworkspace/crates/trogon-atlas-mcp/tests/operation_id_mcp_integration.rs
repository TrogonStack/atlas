#![allow(clippy::unwrap_used, clippy::expect_used)]

// operation_id surface: the idempotency-key param on the mutating tools,
// the get_operation tool, and operation_id showing up in a failed
// mutation's error data. Same duplex-pipe harness as
// tool_lifecycle_integration.rs / tool_surface_gap_coverage.rs: a fresh
// server/client pair against a fresh in-process gRPC backend, driven by
// rmcp::serve_client over tokio::io::duplex.

use std::sync::Arc;

use base64::Engine as _;
use prost::Message as _;
use tokio::net::TcpListener;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::event_model_service_server::EventModelServiceServer;
use trogon_atlas_server::service::EventModelServiceImpl;

async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = trogon_atlas_testsupport::shared().await.store().await;
    let svc = EventModelServiceImpl::try_new(Arc::new(store)).unwrap();
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
    let mcp_server =
        trogon_atlas_mcp::server::McpServer::connect(&grpc_endpoint, None, None, 30, None)
            .await
            .unwrap();
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

async fn mcp_client_for(
    grpc_endpoint: &str,
    branch: Option<&str>,
) -> (McpClient, McpServiceRunning) {
    let mcp_server = trogon_atlas_mcp::server::McpServer::connect(
        grpc_endpoint,
        None,
        None,
        30,
        branch.map(str::to_string),
    )
    .await
    .unwrap();
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);
    use rmcp::ServiceExt;
    let (mcp_service_result, client_result) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    (client_result.unwrap(), mcp_service_result.unwrap())
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

fn base64_decode(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

fn event_json(ns: &str, slug: &str, version: u64, title: &str) -> String {
    serde_json::json!({
        "event": {
            "id": {"namespace": ns, "slug": slug, "version": version},
            "title": title,
        }
    })
    .to_string()
}

async fn get_operation(client: &McpClient, operation_id: &str) -> pb::GetOperationResponse {
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_operation").with_arguments(args_obj(&[(
                "operation_id",
                serde_json::json!(operation_id),
            )])),
        )
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false), "{result:?}");
    let body = call_tool_result_text(&result);
    let raw = base64_decode(body["binpb_base64"].as_str().unwrap());
    pb::GetOperationResponse::decode(raw.as_slice()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_put_entity_json_with_operation_id_is_applied_and_retrievable_via_get_operation() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let operation_id = "mcp-op-put-applied-1";
    let put_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(args_obj(&[
                (
                    "entity_json",
                    serde_json::json!(event_json("mcp-op-id", "created", 1, "Created")),
                ),
                ("create_only", serde_json::json!(true)),
                ("operation_id", serde_json::json!(operation_id)),
            ])),
        )
        .await
        .unwrap();
    assert!(!put_result.is_error.unwrap_or(false), "{put_result:?}");

    let status = get_operation(&client, operation_id).await;
    assert_eq!(
        status.status,
        pb::OperationStatus::Applied as i32,
        "{status:?}"
    );
    assert!(!status.changeset_id.is_empty(), "{status:?}");

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_get_operation_reports_unspecified_for_an_id_never_claimed() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    // No record at all (never claimed) surfaces as UNSPECIFIED, distinct
    // from UNKNOWN (which is reserved for a claimed-but-branch-scoped
    // receipt, see mcp_resolve_branch_entry_... below). The CLI/MCP text
    // layer (trogon_atlas_client::operation::status_label) collapses both to
    // "unknown" for callers who only care that nothing is recoverable, but
    // the raw proto status this tool returns keeps them apart.
    let status = get_operation(&client, "mcp-op-never-claimed").await;
    assert_eq!(
        status.status,
        pb::OperationStatus::Unspecified as i32,
        "{status:?}"
    );
    assert!(status.changeset_id.is_empty(), "{status:?}");

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_put_entity_json_rejects_a_malformed_operation_id() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let err = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(args_obj(&[
                (
                    "entity_json",
                    serde_json::json!(event_json("mcp-op-id-bad", "created", 1, "Created")),
                ),
                ("create_only", serde_json::json!(true)),
                ("operation_id", serde_json::json!("bad id with spaces")),
            ])),
        )
        .await
        .unwrap_err();
    match err {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{e:?}");
            assert!(e.message.contains("operation_id"), "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    // The malformed id must be rejected before any write: the entity was
    // never created.
    let get_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("get_entity_json").with_arguments(args_obj(&[
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("mcp-op-id-bad")),
                ("slug", serde_json::json!("created")),
                ("version", serde_json::json!(1)),
            ])),
        )
        .await
        .unwrap_err();
    match get_result {
        rmcp::ServiceError::McpError(e) => {
            let data = e.data.as_ref().expect("classified error carries data");
            assert_eq!(data["category"], "not_found", "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_put_entity_json_validate_only_does_not_claim_the_operation_id() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let operation_id = "mcp-op-dry-run-1";
    let put_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(args_obj(&[
                (
                    "entity_json",
                    serde_json::json!(event_json("mcp-op-id-dry", "created", 1, "Created")),
                ),
                ("create_only", serde_json::json!(true)),
                ("validate_only", serde_json::json!(true)),
                ("operation_id", serde_json::json!(operation_id)),
            ])),
        )
        .await
        .unwrap();
    assert!(!put_result.is_error.unwrap_or(false), "{put_result:?}");

    let status = get_operation(&client, operation_id).await;
    assert_eq!(
        status.status,
        pb::OperationStatus::Unspecified as i32,
        "dry run must never claim an operation_id: {status:?}"
    );

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_batch_mutate_failure_carries_operation_id_in_error_data() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let operation_id = "mcp-op-batch-failed-1";
    let err = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate").with_arguments(args_obj(&[
                (
                    "ops",
                    serde_json::json!([
                        {"op": "delete", "kind": "event", "namespace": "mcp-op-batch", "slug": "missing", "version": 1},
                    ]),
                ),
                ("operation_id", serde_json::json!(operation_id)),
            ])),
        )
        .await
        .unwrap_err();
    match err {
        rmcp::ServiceError::McpError(e) => {
            let data = e.data.as_ref().expect("classified error carries data");
            assert_eq!(data["operation_id"], operation_id, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_batch_mutate_rpc_failure_carries_operation_id_in_error_data() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut grpc = trogon_atlas_client::client::connect(&grpc_endpoint, None, 30)
        .await
        .unwrap();

    // Seed an entity so create_only against it is a genuine conflict that
    // the gRPC call itself rejects (as opposed to a STATUS_FAILED batch
    // result), exercising the other operation_id-attachment call site.
    grpc.put_entity(pb::PutEntityRequest {
        entity: Some(pb::Entity {
            system: None,
            kind: Some(pb::entity::Kind::Event(pb::Event {
                id: Some(pb::Id {
                    namespace: "mcp-op-batch-rpc".into(),
                    slug: "exists".into(),
                    version: 1,
                }),
                title: "Exists".into(),
                ..Default::default()
            })),
        }),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "mcp-op-batch-rpc".into(),
                slug: "exists".into(),
                version: 1,
            }),
            title: "Exists, again".into(),
            ..Default::default()
        })),
    };
    let entity_b64 = base64::engine::general_purpose::STANDARD.encode(entity.encode_to_vec());

    let operation_id = "mcp-op-batch-rpc-conflict-1";
    let err = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate").with_arguments(args_obj(&[
                (
                    "ops",
                    serde_json::json!([
                        {"op": "put", "entity_b64": entity_b64, "create_only": true},
                    ]),
                ),
                ("operation_id", serde_json::json!(operation_id)),
            ])),
        )
        .await
        .unwrap_err();
    match err {
        rmcp::ServiceError::McpError(e) => {
            let data = e.data.as_ref().expect("classified error carries data");
            assert_eq!(data["operation_id"], operation_id, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_batch_mutate_rejects_an_operation_id_set_on_a_nested_op() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let entity = pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(pb::Id {
                namespace: "mcp-op-nested".into(),
                slug: "created".into(),
                version: 1,
            }),
            title: "Created".into(),
            ..Default::default()
        })),
    };
    let entity_b64 = base64::engine::general_purpose::STANDARD.encode(entity.encode_to_vec());

    // A per-op operation_id is not a field BatchMutateOpJson declares at
    // all (deny_unknown_fields), so the whole batch_mutate call is rejected
    // at parameter deserialization, before the tool handler runs. The
    // tool_router surfaces that as a successful JSON-RPC response carrying
    // isError: true, not a transport-level error.
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("batch_mutate").with_arguments(args_obj(&[(
                "ops",
                serde_json::json!([
                    {"op": "put", "entity_b64": entity_b64, "create_only": true, "operation_id": "nested-not-allowed"},
                ]),
            )])),
        )
        .await
        .unwrap();
    assert!(result.is_error.unwrap_or(false), "{result:?}");
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("operation_id"), "{text}");

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_put_entity_json_on_a_branch_scoped_client_is_always_unknown_via_get_operation() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;

    client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(args_obj(&[
                ("name", serde_json::json!("mcp-op-branch-scoped")),
                ("doc", serde_json::json!("")),
            ])),
        )
        .await
        .unwrap();

    // A branch-scoped McpServer (connect-time branch, see mcp_client_for's
    // doc comment on the unscoped tests) sends every mutating RPC with a
    // branch header. put_entity under that header claims an operation_id
    // the same way the unscoped connection does, but the claim's receipt
    // carries the branch name and therefore keeps no durable receipt.
    let operation_id = "mcp-op-branch-scoped-1";
    let (branch_client, branch_service) =
        mcp_client_for(&grpc_endpoint, Some("mcp-op-branch-scoped")).await;
    let put_result = branch_client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(args_obj(&[
                (
                    "entity_json",
                    serde_json::json!(event_json("mcp-op-branch", "on-branch", 1, "On branch")),
                ),
                ("operation_id", serde_json::json!(operation_id)),
            ])),
        )
        .await
        .unwrap();
    assert!(!put_result.is_error.unwrap_or(false), "{put_result:?}");

    let status = get_operation(&branch_client, operation_id).await;
    assert_eq!(
        status.status,
        pb::OperationStatus::Unknown as i32,
        "branch-scoped mutations keep no durable receipt: {status:?}"
    );
    branch_service.cancel().await.unwrap();

    mcp_service.cancel().await.unwrap();
}
