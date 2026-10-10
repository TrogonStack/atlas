#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use rmcp::model::ErrorCode;
use tokio::net::TcpListener;
use tonic::Code;
/// Integration tests for trogon-atlas-mcp.
///
/// Covers, in increasing order of scope:
///
/// 1. Error classification -- `classify_status` maps tonic status codes
///    to the right `rmcp::ErrorCode` values without leaking internal detail.
///
/// 2. JSON size guard constant -- the 4 MiB sentinel is verified to have
///    the expected value so a refactor cannot silently change the limit.
///
/// 3. gRPC end-to-end path -- a real in-process gRPC server backed by
///    an ephemeral `JetStream` store (trogon-atlas-testsupport, same pattern
///    as trogon-atlas-server/tests/*.rs) exercises `put_entity_json`,
///    `batch_mutate`, `delete_entity`, `get_entity` (read), and the `NOT_FOUND`
///    error path that the MCP handler classifies via `classify_status`.
///
/// 4. Full MCP transport dispatch -- `McpServer::connect` plus an in-process
///    `tokio::io::duplex` pipe driven by `rmcp::serve_client` exercises the
///    `#[tool_router]`-generated dispatch itself (tool name resolution,
///    parameter deserialization, `Ok`/`Err` -> JSON-RPC result/error mapping),
///    not just the internals a tool method delegates to.
use trogon_atlas_mcp::error::classify_status;
use trogon_atlas_proto as pb;
use trogon_atlas_proto::{
    event_model_service_client::EventModelServiceClient,
    event_model_service_server::EventModelServiceServer,
};
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

async fn grpc_client(endpoint: &str) -> EventModelServiceClient<tonic::transport::Channel> {
    EventModelServiceClient::connect(endpoint.to_string())
        .await
        .unwrap()
}

fn id(ns: &str, slug: &str, version: u64) -> pb::Id {
    pb::Id {
        namespace: ns.into(),
        slug: slug.into(),
        version,
    }
}

fn make_event(ns: &str, slug: &str, version: u64) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::Event(pb::Event {
            id: Some(id(ns, slug, version)),
            title: format!("{ns}/{slug}@{version}"),
            ..Default::default()
        })),
    }
}

// ===== classify_status unit tests =====

#[test]
fn not_found_classifies_as_invalid_params() {
    let s = tonic::Status::not_found("entity not found");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("entity not found"), "{}", err.message);
}

#[test]
fn invalid_argument_classifies_as_invalid_params() {
    let s = tonic::Status::invalid_argument("bad slug");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[test]
fn already_exists_classifies_as_invalid_params() {
    let s = tonic::Status::already_exists("entity already exists");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[test]
fn failed_precondition_classifies_as_invalid_params() {
    let s = tonic::Status::failed_precondition("etag mismatch");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[test]
fn unauthenticated_returns_generic_auth_message() {
    let s = tonic::Status::unauthenticated("invalid token abc123");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        !err.message.contains("abc123"),
        "auth detail must not leak: {}",
        err.message
    );
}

#[test]
fn permission_denied_returns_generic_auth_message() {
    let s = tonic::Status::permission_denied("user X cannot write");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    assert!(!err.message.contains("user X"), "{}", err.message);
}

#[test]
fn internal_classifies_as_internal_error_with_scrubbed_message() {
    let s = tonic::Status::internal("panic: secret stack trace xyz");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    assert!(
        !err.message.contains("secret"),
        "internal detail must not leak: {}",
        err.message
    );
    assert!(
        err.message.to_lowercase().contains("internal"),
        "message should indicate internal error: {}",
        err.message
    );
}

#[test]
fn unavailable_classifies_as_internal_error() {
    let s = tonic::Status::unavailable("circuit open");
    let err = classify_status(&s);
    assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
    assert!(!err.message.contains("circuit"), "{}", err.message);
}

// ===== JSON size guard constant =====

#[test]
fn max_json_bytes_is_four_mib() {
    assert_eq!(trogon_atlas_mcp::server::MAX_JSON_BYTES, 4 * 1024 * 1024);
}

// ===== gRPC end-to-end tests (same wire path as the MCP handlers) =====

#[tokio::test(flavor = "multi_thread")]
async fn put_entity_round_trip() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let resp = c
        .put_entity(pb::PutEntityRequest {
            entity: Some(make_event("shop", "order-placed", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
            operation_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_ne!(resp.etag, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn get_entity_on_missing_produces_not_found_classified_as_invalid_params() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let err = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("shop", "ghost", 1)),
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), Code::NotFound);

    let classified = classify_status(&err);
    assert_eq!(
        classified.code,
        ErrorCode::INVALID_PARAMS,
        "NOT_FOUND must classify as invalid_params, not internal_error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_mutate_round_trip() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let resp = c
        .batch_mutate(pb::BatchMutateRequest {
            ops: vec![pb::BatchMutateOp {
                op: Some(pb::batch_mutate_op::Op::Put(pb::PutEntityRequest {
                    entity: Some(make_event("shop", "cart-cleared", 1)),
                    create_only: true,
                    if_match: String::new(),
                    validate_only: false,
                    force: false,
                    operation_id: String::new(),
                })),
            }],
            validate_only: false,
            operation_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.results.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_entity_round_trip() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    c.put_entity(pb::PutEntityRequest {
        entity: Some(make_event("shop", "item-removed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    c.delete_entity(pb::DeleteEntityRequest {
        kind: pb::EntityKind::Event as i32,
        id: Some(id("shop", "item-removed", 1)),
        mode: pb::delete_entity_request::Mode::FailIfReferenced as i32,
        if_match: String::new(),
        operation_id: String::new(),
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn already_exists_classified_as_invalid_params() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    c.put_entity(pb::PutEntityRequest {
        entity: Some(make_event("shop", "dup", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    let err = c
        .put_entity(pb::PutEntityRequest {
            entity: Some(make_event("shop", "dup", 1)),
            create_only: true,
            if_match: String::new(),
            validate_only: false,
            force: false,
            operation_id: String::new(),
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), Code::AlreadyExists);
    let classified = classify_status(&err);
    assert_eq!(classified.code, ErrorCode::INVALID_PARAMS);
}

// Item 1: Code::Aborted (etag mismatch) must surface as invalid_params, not
// internal_error, so MCP clients doing conditional writes see the real message.
#[test]
fn aborted_classifies_as_invalid_params() {
    let s = tonic::Status::aborted("etag mismatch");
    let err = classify_status(&s);
    assert_eq!(
        err.code,
        ErrorCode::INVALID_PARAMS,
        "ABORTED must classify as invalid_params so etag-mismatch messages reach MCP clients"
    );
    assert!(
        err.message.contains("etag mismatch"),
        "etag mismatch message must not be scrubbed: {}",
        err.message
    );
}

fn make_command_slice(ns: &str, slice_slug: &str, event_slug: &str) -> pb::Entity {
    pb::Entity {
        system: None,
        kind: Some(pb::entity::Kind::CommandSlice(pb::CommandSlice {
            id: Some(id(ns, slice_slug, 1)),
            title: slice_slug.into(),
            doc: String::new(),
            persona: None,
            ui: None,
            command: None,
            emitted_events: vec![pb::EventEdge {
                event: Some(pb::EventRef {
                    id: Some(id(ns, event_slug, 1)),
                }),
                doc: String::new(),
                metadata: Vec::new(),
            }],
            scenarios: Vec::new(),
            metadata: Vec::new(),
            supersedes: None,
        })),
    }
}

// Item 3: infer_data_flow happy path via gRPC adapter pattern.
#[tokio::test(flavor = "multi_thread")]
async fn infer_data_flow_returns_ok_for_command_slice_scope() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    c.put_entity(pb::PutEntityRequest {
        entity: Some(make_command_slice(
            "flow",
            "checkout-slice",
            "checkout-done",
        )),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    let resp = c
        .infer_data_flow(pb::InferDataFlowRequest {
            scope: Some(pb::AnalysisScope {
                scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(id("flow", "checkout-slice", 1)),
                })),
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "without an LLM analyzer the provenance must be Deterministic"
    );
}

// Item 3: check_information_completeness happy path via gRPC adapter pattern.
#[tokio::test(flavor = "multi_thread")]
async fn check_information_completeness_returns_ok_for_slice_scope() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    c.put_entity(pb::PutEntityRequest {
        entity: Some(make_command_slice("compl", "pay-slice", "payment-captured")),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    let resp = c
        .check_information_completeness(pb::CheckInformationCompletenessRequest {
            scope: Some(pb::AnalysisScope {
                scope: Some(pb::analysis_scope::Scope::Slice(pb::EntityRef {
                    kind: pb::EntityKind::CommandSlice as i32,
                    id: Some(id("compl", "pay-slice", 1)),
                })),
            }),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(
        resp.overall_score >= 0.0 && resp.overall_score <= 1.0,
        "overall_score must be in [0, 1]; got {}",
        resp.overall_score
    );
    assert_eq!(
        resp.provenance,
        pb::AnalysisProvenance::Deterministic as i32,
        "without an LLM analyzer the provenance must be Deterministic"
    );
}

// Item 3: check_information_completeness error case -- missing scope is
// InvalidArgument (the cheapest error case that exercises the classify path).
#[tokio::test(flavor = "multi_thread")]
async fn check_information_completeness_missing_scope_is_invalid_argument() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let err = c
        .check_information_completeness(pb::CheckInformationCompletenessRequest { scope: None })
        .await
        .unwrap_err();

    assert_eq!(err.code(), Code::InvalidArgument);
    let classified = classify_status(&err);
    assert_eq!(classified.code, ErrorCode::INVALID_PARAMS);
}

// Item 3: validate_project happy path -- empty-store domain scope returns no reports.
#[tokio::test(flavor = "multi_thread")]
async fn validate_project_domain_scope_returns_no_reports_on_empty_store() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let resp = c
        .validate_project(pb::ValidateProjectRequest {
            scope: Some(pb::validate_project_request::Scope::DomainId(id(
                "acme", "core", 1,
            ))),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resp.reports,
        [] as [trogon_atlas_proto::validate_project_response::ModelReport; 0]
    );
    assert_eq!(resp.total_errors, 0);
}

// Item 3: validate_project error case -- missing scope is InvalidArgument.
#[tokio::test(flavor = "multi_thread")]
async fn validate_project_missing_scope_is_invalid_argument() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let err = c
        .validate_project(pb::ValidateProjectRequest { scope: None })
        .await
        .unwrap_err();

    assert_eq!(err.code(), Code::InvalidArgument);
    let classified = classify_status(&err);
    assert_eq!(classified.code, ErrorCode::INVALID_PARAMS);
}

// Item 3: delete_by_query happy path -- creates an entity then bulk deletes it.
#[tokio::test(flavor = "multi_thread")]
async fn delete_by_query_deletes_matching_entities() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    c.put_entity(pb::PutEntityRequest {
        entity: Some(make_event("dbq", "order-placed", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    let resp = c
        .delete_by_query(pb::DeleteByQueryRequest {
            namespace: "dbq".into(),
            slug: String::new(),
            kind: 0,
            project: String::new(),
            max_deletes: 10,
            mode: pb::delete_entity_request::Mode::Force as i32,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.deleted_count, 1);
    assert!(!resp.dry_run);
}

// Item 3: list_entities_by_domain happy path -- returns empty when no entities exist.
#[tokio::test(flavor = "multi_thread")]
async fn list_entities_by_domain_returns_empty_on_empty_store() {
    let endpoint = start_server().await;
    let mut c = grpc_client(&endpoint).await;

    let resp = c
        .list_entities_by_domain(pb::ListEntitiesByDomainRequest {
            domain_id: Some(id("acme", "core", 1)),
            kinds: vec![],
            page_size: 0,
            page_token: String::new(),
            latest_versions_only: true,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.entities, [] as [trogon_atlas_proto::Entity; 0]);
}

// Item 3: stream_changes is intentionally not exposed as an MCP tool.
// The RPC returns a server-streaming response (`returns (stream ChangeEvent)`)
// which has no natural request/response mapping to the MCP tool protocol.
// list_changes (the paged polling sibling) covers the MCP surface instead.
// No MCP-level test is added for stream_changes.

// Item 15: missing required field in tool params produces invalid_params via
// the rmcp dispatch path. The `#[tool_router]` macro uses serde_json to
// deserialize the incoming params object; a missing required field
// (no default) causes a deserialization error that rmcp maps to
// invalid_params. We test this in-process by deserializing the params struct
// directly using the same serde_json path the router uses.
#[test]
fn get_entity_params_missing_kind_returns_deserialization_error() {
    use trogon_atlas_mcp::tools::EntitySpec;

    let json_missing_kind = serde_json::json!({
        "namespace": "shop",
        "slug": "order-placed",
        "version": 1
    });
    let result: Result<EntitySpec, _> = serde_json::from_value(json_missing_kind);
    assert!(
        result.is_err(),
        "deserializing EntitySpec without 'kind' must fail"
    );
}

// Item 4: stdio transport (JSON-RPC framing + tool_router dispatch) tested via
// an in-process tokio::io::duplex pair. The server side is a real McpServer
// connected to an in-process gRPC backend; the client side is rmcp::serve_client
// driving the initialize handshake and one call_tool round-trip.
//
// This exercises the rmcp JSON-RPC framing and the #[tool_router] dispatch path
// that the gRPC-only integration tests cannot reach.
//
// Both .serve() calls must start concurrently: the server waits for the client's
// initialize message, the client waits for the server's initialize response.
// Sequential setup deadlocks, so we use tokio::join! to start both at once.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_transport_initialize_and_list_namespaces() {
    let grpc_endpoint = start_server().await;

    // McpServer::connect requires an actual gRPC endpoint string.
    let mcp_server =
        trogon_atlas_mcp::server::McpServer::connect(&grpc_endpoint, None, None, 30, None)
            .await
            .unwrap();

    // Create an in-process bidirectional pipe. Both halves are split into
    // separate read/write ends so server and client can handshake concurrently.
    let (client_half, server_half) = tokio::io::duplex(64 * 1024);

    // The server's .serve() and the client's serve_client() both start the
    // MCP initialize handshake. They must run concurrently or both block.
    use rmcp::ServiceExt;
    let (mcp_service_result, client_result) = tokio::join!(
        mcp_server.serve(server_half),
        rmcp::serve_client((), client_half)
    );
    let mcp_service = mcp_service_result.unwrap();
    let client = client_result.unwrap();

    // list_namespaces takes no arguments and always succeeds even on an empty
    // store, so it is the cheapest tool call to verify end-to-end dispatch.
    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("list_namespaces"))
        .await
        .unwrap();

    assert!(
        !result.is_error.unwrap_or(false),
        "list_namespaces must succeed over the in-process MCP transport: {result:?}"
    );

    // Tear down cleanly.
    mcp_service.cancel().await.unwrap();
}

// Item 4: also verify that an invalid namespace arg surfaces as an MCP error
// (invalid_params) rather than a transport error.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_transport_invalid_namespace_returns_error() {
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
    let mcp_service = mcp_service_result.unwrap();
    let client = client_result.unwrap();

    // An invalid namespace (contains a space) must be caught by the local
    // validate_id_component guard before the RPC is issued. The handler
    // returns Err(rmcp::ErrorData) which rmcp converts to a JSON-RPC error
    // response; the client receives ServiceError::McpError. We match the
    // error to confirm it carries the right message.
    let mut args = serde_json::Map::new();
    args.insert("kind".into(), serde_json::json!("event"));
    args.insert("namespace".into(), serde_json::json!("bad namespace"));
    args.insert("slug".into(), serde_json::json!("order-placed"));
    let err = client
        .call_tool(rmcp::model::CallToolRequestParams::new("list_versions").with_arguments(args))
        .await
        .unwrap_err();

    match err {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(
                e.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "invalid namespace must produce INVALID_PARAMS: {e:?}"
            );
            assert!(
                e.message.contains("namespace"),
                "error message must mention 'namespace': {:?}",
                e.message
            );
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

// The apply_manifests / diff_manifests / export_namespace tools wrap
// trogon-atlas-client (shared with trogon-atlas) rather than a single RPC, so this
// exercises the create -> unchanged -> export round trip through the same
// in-process MCP transport as the tests above.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_transport_apply_manifests_round_trip() {
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
    let mcp_service = mcp_service_result.unwrap();
    let client = client_result.unwrap();

    let manifests_yaml = r"
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Event
metadata: { namespace: mcp-apply, name: thing.happened }
spec: { title: Thing happened }
";

    let mut args = serde_json::Map::new();
    args.insert("manifests_yaml".into(), serde_json::json!(manifests_yaml));
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("apply_manifests").with_arguments(args.clone()),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "apply_manifests must succeed: {result:?}"
    );
    let body: serde_json::Value =
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(
        body["lines"],
        serde_json::json!(["event:mcp-apply/thing.happened@1 created"])
    );

    // Re-applying the identical manifest is a no-op.
    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("apply_manifests").with_arguments(args))
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(
        body["lines"],
        serde_json::json!(["event:mcp-apply/thing.happened@1 unchanged"])
    );

    // export_namespace renders the applied entity back into manifest YAML.
    let mut export_args = serde_json::Map::new();
    export_args.insert("namespace".into(), serde_json::json!("mcp-apply"));
    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("export_namespace").with_arguments(export_args),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error.unwrap_or(false),
        "export_namespace must succeed: {result:?}"
    );
    let body: serde_json::Value =
        serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap();
    assert_eq!(body["skipped"], serde_json::json!([]));
    assert!(
        body["files"].as_object().is_some_and(|f| !f.is_empty()),
        "{body:?}"
    );

    mcp_service.cancel().await.unwrap();
}

// ===========================================================================
// Branch tools (Phase 1 Isolation + Phase 2 Review and merge). Same in-process
// stdio transport as the apply_manifests test above: a real McpServer wired
// to an in-process gRPC server backed by an ephemeral JetStream store, driven
// by rmcp::serve_client over a tokio::io::duplex pipe.
// ===========================================================================

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;
type McpServiceRunning =
    rmcp::service::RunningService<rmcp::RoleServer, trogon_atlas_mcp::server::McpServer>;

/// Start an MCP server + client pair over an in-process duplex pipe, against
/// a fresh in-process gRPC server. Same setup as every other
/// `mcp_stdio_transport_*` test in this file.
async fn start_mcp_client() -> (McpClient, McpServiceRunning, String) {
    let grpc_endpoint = start_server().await;
    let (client, service) = mcp_client_for(&grpc_endpoint, None).await;
    (client, service, grpc_endpoint)
}

/// Start a second MCP server + client pair against an *existing* gRPC
/// endpoint, optionally scoped to a branch. Branch scope for trogon-atlas-mcp
/// is bound once at `McpServer::connect` time (there is no per-call branch
/// header the way trogon-atlas's `--branch` flag works), so exercising a branch's
/// content through the MCP surface requires a second, branch-scoped server
/// instance talking to the same backend as the unscoped one that drives
/// diff_branch/merge_branch/update_branch/resolve_branch_entry (those RPCs
/// take the branch `name` as an explicit field, never via connect-time scope).
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

fn call_tool_result_text(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    serde_json::from_str(&result.content[0].as_text().unwrap().text).unwrap()
}

fn args_obj(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    m
}

// create_branch / list_branches / delete_branch: the three Phase 1 lifecycle
// tools. Their responses are Debug-formatted text (see server.rs), not JSON,
// so assertions match on substrings rather than parsing a JSON body.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_branch_lifecycle_create_list_delete() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let created = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(args_obj(&[
                ("name", serde_json::json!("mcp-lifecycle")),
                ("doc", serde_json::json!("mcp test branch")),
            ])),
        )
        .await
        .unwrap();
    assert!(!created.is_error.unwrap_or(false), "{created:?}");
    let created_text = created.content[0].as_text().unwrap().text.clone();
    assert!(created_text.contains("mcp-lifecycle"), "{created_text}");

    let listed = client
        .call_tool(rmcp::model::CallToolRequestParams::new("list_branches"))
        .await
        .unwrap();
    assert!(!listed.is_error.unwrap_or(false), "{listed:?}");
    let listed_text = listed.content[0].as_text().unwrap().text.clone();
    assert!(listed_text.contains("mcp-lifecycle"), "{listed_text}");

    let deleted = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_branch")
                .with_arguments(args_obj(&[("name", serde_json::json!("mcp-lifecycle"))])),
        )
        .await
        .unwrap();
    assert!(!deleted.is_error.unwrap_or(false), "{deleted:?}");
    let deleted_text = deleted.content[0].as_text().unwrap().text.clone();
    assert!(
        deleted_text.contains("mcp-lifecycle") && deleted_text.contains("deleted"),
        "{deleted_text}"
    );

    let listed_after = client
        .call_tool(rmcp::model::CallToolRequestParams::new("list_branches"))
        .await
        .unwrap();
    let listed_after_text = listed_after.content[0].as_text().unwrap().text.clone();
    assert!(
        !listed_after_text.contains("mcp-lifecycle"),
        "{listed_after_text}"
    );

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_create_branch_invalid_name_returns_error() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let err = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(args_obj(&[
                ("name", serde_json::json!("bad branch name")),
                ("doc", serde_json::json!("")),
            ])),
        )
        .await
        .unwrap_err();

    match err {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_delete_branch_missing_returns_error() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let err = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("delete_branch")
                .with_arguments(args_obj(&[("name", serde_json::json!("does-not-exist"))])),
        )
        .await
        .unwrap_err();

    match err {
        rmcp::ServiceError::McpError(e) => {
            assert_eq!(e.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{e:?}");
        }
        other => panic!("expected McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

// diff_branch / update_branch / merge_branch / resolve_branch_entry: the
// Phase 2 tools. These respond with `ok_response` JSON envelopes
// (`{"format": "binpb_base64"|"json", ...}`); requesting `json: true` lets
// the test assert on the proto3-JSON body directly instead of decoding
// base64-encoded protobuf bytes.
fn put_entity_json_args(entity_json: &str) -> serde_json::Map<String, serde_json::Value> {
    args_obj(&[("entity_json", serde_json::json!(entity_json))])
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

#[tokio::test(flavor = "multi_thread")]
async fn mcp_diff_update_merge_lifecycle_clean() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;

    client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(args_obj(&[
                ("name", serde_json::json!("mcp-p2-clean")),
                ("doc", serde_json::json!("")),
            ])),
        )
        .await
        .unwrap();

    // A second, branch-scoped MCP server/client pair against the same gRPC
    // backend authors content directly on the branch (see mcp_client_for's
    // doc comment for why two instances are needed).
    let (branch_client, branch_service) =
        mcp_client_for(&grpc_endpoint, Some("mcp-p2-clean")).await;

    let put_result = branch_client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(
                put_entity_json_args(&event_json("mcp-p2diff", "on-branch", 1, "On branch")),
            ),
        )
        .await
        .unwrap();
    assert!(!put_result.is_error.unwrap_or(false), "{put_result:?}");

    // diff_branch (via the unscoped client, by name) sees the new entity as
    // ADDED.
    let mut diff_args = args_obj(&[("name", serde_json::json!("mcp-p2-clean"))]);
    diff_args.insert("json".into(), serde_json::json!(true));
    let diff_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("diff_branch")
                .with_arguments(diff_args.clone()),
        )
        .await
        .unwrap();
    assert!(!diff_result.is_error.unwrap_or(false), "{diff_result:?}");
    let diff_body = call_tool_result_text(&diff_result);
    assert_eq!(diff_body["format"], serde_json::json!("json"));
    let diff_json: serde_json::Value =
        serde_json::from_str(diff_body["json"].as_str().unwrap()).unwrap();
    let entries = diff_json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{diff_json:?}");
    assert_eq!(entries[0]["status"], serde_json::json!("STATUS_ADDED"));

    // update_branch on a branch with no baseline movement rebases nothing and
    // surfaces no conflicts.
    let mut update_args = args_obj(&[("name", serde_json::json!("mcp-p2-clean"))]);
    update_args.insert("json".into(), serde_json::json!(true));
    let update_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("update_branch").with_arguments(update_args),
        )
        .await
        .unwrap();
    assert!(
        !update_result.is_error.unwrap_or(false),
        "{update_result:?}"
    );
    let update_body = call_tool_result_text(&update_result);
    let update_json: serde_json::Value =
        serde_json::from_str(update_body["json"].as_str().unwrap()).unwrap();
    // proto3 JSON omits zero/empty fields: absent means 0 / [].
    assert_eq!(update_json["rebasedCount"].as_u64().unwrap_or(0), 0);
    assert!(update_json["conflicts"]
        .as_array()
        .is_none_or(Vec::is_empty));

    // merge_branch lands the branch cleanly and deletes it (default
    // keep_branch=false).
    let mut merge_args = args_obj(&[("name", serde_json::json!("mcp-p2-clean"))]);
    merge_args.insert("json".into(), serde_json::json!(true));
    let merge_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("merge_branch").with_arguments(merge_args),
        )
        .await
        .unwrap();
    assert!(!merge_result.is_error.unwrap_or(false), "{merge_result:?}");
    let merge_body = call_tool_result_text(&merge_result);
    let merge_json: serde_json::Value =
        serde_json::from_str(merge_body["json"].as_str().unwrap()).unwrap();
    assert_eq!(merge_json["status"], serde_json::json!("STATUS_APPLIED"));
    assert_eq!(merge_json["appliedCount"], serde_json::json!(1));

    let listed = client
        .call_tool(rmcp::model::CallToolRequestParams::new("list_branches"))
        .await
        .unwrap();
    let listed_text = listed.content[0].as_text().unwrap().text.clone();
    assert!(
        !listed_text.contains("mcp-p2-clean"),
        "clean merge with keep_branch=false must delete the branch: {listed_text}"
    );

    branch_service.cancel().await.unwrap();
    mcp_service.cancel().await.unwrap();
}

// Conflicts deserve their own test: a branch edit and a baseline edit to the
// same entity collide, merge_branch must report CONFLICTS with non-empty
// conflictFieldPaths, and nothing may land.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_merge_branch_reports_conflict_field_paths() {
    let (client, mcp_service, grpc_endpoint) = start_mcp_client().await;
    let mut c = grpc_client(&grpc_endpoint).await;

    // Seed the baseline entity before branching.
    c.put_entity(pb::PutEntityRequest {
        entity: Some(make_event("mcp-p2cee", "a", 1)),
        create_only: true,
        if_match: String::new(),
        validate_only: false,
        force: false,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(args_obj(&[
                ("name", serde_json::json!("mcp-p2-conflict")),
                ("doc", serde_json::json!("")),
            ])),
        )
        .await
        .unwrap();

    let (branch_client, branch_service) =
        mcp_client_for(&grpc_endpoint, Some("mcp-p2-conflict")).await;

    // Branch edits "a".
    let branch_put = branch_client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("put_entity_json").with_arguments(
                put_entity_json_args(&event_json("mcp-p2cee", "a", 1, "Branch edit")),
            ),
        )
        .await
        .unwrap();
    assert!(!branch_put.is_error.unwrap_or(false), "{branch_put:?}");

    // Baseline also moves "a" (force=true to bypass no-op detection and etag
    // guard, matching the server-level conflict test's pattern).
    let mut force_put = make_event("mcp-p2cee", "a", 1);
    if let Some(pb::entity::Kind::Event(e)) = force_put.kind.as_mut() {
        e.title = "Baseline moved".to_string();
    }
    c.put_entity(pb::PutEntityRequest {
        entity: Some(force_put),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: true,
        operation_id: String::new(),
    })
    .await
    .unwrap();

    let mut merge_args = args_obj(&[
        ("name", serde_json::json!("mcp-p2-conflict")),
        ("keep_branch", serde_json::json!(true)),
    ]);
    merge_args.insert("json".into(), serde_json::json!(true));
    let merge_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("merge_branch").with_arguments(merge_args),
        )
        .await
        .unwrap();
    assert!(!merge_result.is_error.unwrap_or(false), "{merge_result:?}");
    let merge_body = call_tool_result_text(&merge_result);
    let merge_json: serde_json::Value =
        serde_json::from_str(merge_body["json"].as_str().unwrap()).unwrap();
    assert_eq!(
        merge_json["status"],
        serde_json::json!("STATUS_CONFLICTS"),
        "{merge_json:?}"
    );
    let conflicts = merge_json["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(
        conflicts[0]["status"],
        serde_json::json!("STATUS_CONFLICT_EDIT_EDIT")
    );
    let field_paths = conflicts[0]["conflictFieldPaths"].as_array().unwrap();
    assert!(!field_paths.is_empty(), "{conflicts:?}");
    assert!(
        field_paths
            .iter()
            .any(|p| p.as_str().unwrap().contains("title")),
        "{field_paths:?}"
    );

    // Nothing landed: baseline still shows the pre-merge value.
    let baseline = c
        .get_entity(pb::GetEntityRequest {
            kind: pb::EntityKind::Event as i32,
            id: Some(id("mcp-p2cee", "a", 1)),
        })
        .await
        .unwrap()
        .into_inner();
    match baseline.entity.unwrap().kind {
        Some(pb::entity::Kind::Event(e)) => assert_eq!(e.title, "Baseline moved"),
        other => panic!("unexpected: {other:?}"),
    }

    // A decision made from the conflict as merge reported it is refused once
    // baseline moves again, with structured data telling the agent to replan.
    let inspected_state = conflicts[0]["state"].clone();
    assert!(inspected_state.is_object(), "{conflicts:?}");
    let resolve_args = |state: serde_json::Value| {
        args_obj(&[
            ("name", serde_json::json!("mcp-p2-conflict")),
            ("kind", serde_json::json!("event")),
            ("namespace", serde_json::json!("mcp-p2cee")),
            ("slug", serde_json::json!("a")),
            ("version", serde_json::json!(1)),
            ("resolution", serde_json::json!("keep_ours")),
            ("expected_state", state),
        ])
    };
    let mut moved_again = make_event("mcp-p2cee", "a", 1);
    if let Some(pb::entity::Kind::Event(e)) = moved_again.kind.as_mut() {
        e.title = "Baseline moved".to_string();
        e.doc = "and again".to_string();
    }
    c.put_entity(pb::PutEntityRequest {
        entity: Some(moved_again),
        create_only: false,
        if_match: String::new(),
        validate_only: false,
        force: true,
        operation_id: String::new(),
    })
    .await
    .unwrap();
    let stale = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("resolve_branch_entry")
                .with_arguments(resolve_args(inspected_state.clone())),
        )
        .await
        .unwrap_err();
    let rmcp::ServiceError::McpError(stale) = stale else {
        panic!("expected an MCP error, got {stale:?}");
    };
    assert_eq!(stale.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    let data = stale
        .data
        .expect("a stale resolution must carry structured data");
    assert_eq!(data["category"], "stale_state", "{data}");
    assert_eq!(data["next_action"], "replan", "{data}");
    assert_eq!(
        data["next_action_detail"]["branch"], "mcp-p2-conflict",
        "{data}"
    );
    assert_ne!(
        data["next_action_detail"]["expected_state"], data["next_action_detail"]["actual_state"],
        "{data}"
    );

    let mut diff_args = args_obj(&[("name", serde_json::json!("mcp-p2-conflict"))]);
    diff_args.insert("json".into(), serde_json::json!(true));
    let diff_result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("diff_branch").with_arguments(diff_args))
        .await
        .unwrap();
    let diff_body = call_tool_result_text(&diff_result);
    let diff_json: serde_json::Value =
        serde_json::from_str(diff_body["json"].as_str().unwrap()).unwrap();
    let fresh_state = diff_json["entries"][0]["state"].clone();

    // resolve_branch_entry(keep_ours) from the fresh state clears the
    // conflict; a follow-up merge then lands cleanly, exercising
    // resolve_branch_entry's JSON args and success response shape end to end.
    let resolve_result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("resolve_branch_entry")
                .with_arguments(resolve_args(fresh_state)),
        )
        .await
        .unwrap();
    assert!(
        !resolve_result.is_error.unwrap_or(false),
        "{resolve_result:?}"
    );
    assert_eq!(
        resolve_result.content[0].as_text().unwrap().text,
        "resolved"
    );

    let mut merge_args_2 = args_obj(&[("name", serde_json::json!("mcp-p2-conflict"))]);
    merge_args_2.insert("json".into(), serde_json::json!(true));
    let merge_result_2 = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("merge_branch").with_arguments(merge_args_2),
        )
        .await
        .unwrap();
    let merge_body_2 = call_tool_result_text(&merge_result_2);
    let merge_json_2: serde_json::Value =
        serde_json::from_str(merge_body_2["json"].as_str().unwrap()).unwrap();
    assert_eq!(
        merge_json_2["status"],
        serde_json::json!("STATUS_APPLIED"),
        "after keep_ours resolution the merge must land cleanly: {merge_json_2:?}"
    );

    branch_service.cancel().await.unwrap();
    mcp_service.cancel().await.unwrap();
}

// merge_branch / diff_branch / update_branch / resolve_branch_entry against a
// branch name that was never created: the store returns NotFound, which
// classify_status maps to INVALID_PARAMS (see the classify_status unit tests
// above), never an internal error.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_branch_tools_reject_unknown_branch_name() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let cases: &[(&str, serde_json::Map<String, serde_json::Value>)] = &[
        (
            "diff_branch",
            args_obj(&[("name", serde_json::json!("ghost-branch"))]),
        ),
        (
            "merge_branch",
            args_obj(&[("name", serde_json::json!("ghost-branch"))]),
        ),
        (
            "update_branch",
            args_obj(&[("name", serde_json::json!("ghost-branch"))]),
        ),
        (
            "resolve_branch_entry",
            args_obj(&[
                ("name", serde_json::json!("ghost-branch")),
                ("kind", serde_json::json!("event")),
                ("namespace", serde_json::json!("ns")),
                ("slug", serde_json::json!("s")),
                ("version", serde_json::json!(1)),
                ("resolution", serde_json::json!("keep_ours")),
                ("expected_state", serde_json::json!({})),
            ]),
        ),
        (
            "delete_branch",
            args_obj(&[("name", serde_json::json!("ghost-branch"))]),
        ),
    ];

    for (tool_name, args) in cases {
        let err = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new(*tool_name).with_arguments(args.clone()),
            )
            .await
            .unwrap_err();
        match err {
            rmcp::ServiceError::McpError(e) => {
                assert_eq!(
                    e.code,
                    rmcp::model::ErrorCode::INVALID_PARAMS,
                    "{tool_name}: {e:?}"
                );
            }
            other => panic!("{tool_name}: expected McpError, got {other:?}"),
        }
    }

    mcp_service.cancel().await.unwrap();
}

// validate_project description: "set exactly one" of project_* OR domain_*.
// Passing both currently succeeds and silently scopes to the project.
#[tokio::test(flavor = "multi_thread")]
async fn mcp_validate_project_rejects_ambiguous_dual_scope() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let result = client
        .call_tool(
            rmcp::model::CallToolRequestParams::new("validate_project").with_arguments(args_obj(
                &[
                    ("project_namespace", serde_json::json!("acme")),
                    ("project_slug", serde_json::json!("core")),
                    ("project_version", serde_json::json!(1)),
                    ("domain_namespace", serde_json::json!("acme")),
                    ("domain_slug", serde_json::json!("commerce")),
                    ("domain_version", serde_json::json!(1)),
                ],
            )),
        )
        .await;

    match result {
        Ok(ok) => {
            assert!(
                ok.is_error.unwrap_or(false),
                "dual project+domain scope must be rejected; tool succeeded: {ok:?}"
            );
        }
        Err(rmcp::ServiceError::McpError(e)) => {
            assert_eq!(e.code, ErrorCode::INVALID_PARAMS, "{e:?}");
        }
        Err(other) => panic!("expected invalid_params McpError, got {other:?}"),
    }

    mcp_service.cancel().await.unwrap();
}

fn orders_library_json(ns: &str) -> serde_json::Value {
    serde_json::json!({
        "typeLibrary": {
            "id": {"namespace": ns, "slug": "shop.orders.v1", "version": 1},
            "title": "Order types",
            "files": [{
                "path": "shop/orders/v1/orders.proto",
                "content": "syntax = \"proto3\";\npackage shop.orders.v1;\nmessage OrderPlaced { string order_id = 1; int64 total_cents = 2; }\n"
            }]
        }
    })
}

fn order_placed_event_json(ns: &str) -> serde_json::Value {
    serde_json::json!({
        "event": {
            "id": {"namespace": ns, "slug": "order-placed", "version": 1},
            "title": "Order placed",
            "schema": {
                "@type": "type.googleapis.com/shop.orders.v1.OrderPlaced",
                "orderId": "o-1",
                "totalCents": "1250"
            }
        }
    })
}

async fn call_ok(
    client: &McpClient,
    tool: &'static str,
    args: serde_json::Map<String, serde_json::Value>,
) -> rmcp::model::CallToolResult {
    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e:?}"));
    assert!(!result.is_error.unwrap_or(false), "{tool}: {result:?}");
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn json_tools_spell_schemas_with_tenant_types() {
    let (client, mcp_service, _grpc_endpoint) = start_mcp_client().await;

    let ns = "mcp-tenant-put";
    call_ok(
        &client,
        "put_entity_json",
        put_entity_json_args(&orders_library_json(ns).to_string()),
    )
    .await;
    call_ok(
        &client,
        "put_entity_json",
        put_entity_json_args(&order_placed_event_json(ns).to_string()),
    )
    .await;
    let got = call_ok(
        &client,
        "get_entity_json",
        args_obj(&[
            ("kind", serde_json::json!("event")),
            ("namespace", serde_json::json!(ns)),
            ("slug", serde_json::json!("order-placed")),
            ("version", serde_json::json!(1)),
        ]),
    )
    .await;
    let schema = &call_tool_result_text(&got)["json"]["event"]["schema"];
    assert_eq!(
        schema["@type"],
        serde_json::json!("type.googleapis.com/shop.orders.v1.OrderPlaced"),
        "{schema}"
    );
    assert_eq!(schema["orderId"], serde_json::json!("o-1"), "{schema}");

    let ns = "mcp-tenant-batch";
    let request = serde_json::json!({
        "ops": [
            {"put": {"entity": order_placed_event_json(ns)}},
            {"put": {"entity": orders_library_json(ns)}},
        ]
    });
    call_ok(
        &client,
        "batch_mutate_json",
        args_obj(&[("request_json", serde_json::json!(request.to_string()))]),
    )
    .await;

    mcp_service.cancel().await.unwrap();
}
